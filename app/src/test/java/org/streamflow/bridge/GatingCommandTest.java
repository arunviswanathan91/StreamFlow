package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.concurrent.TimeUnit;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Phase 4 — Interactive gating engine. Verifies flowCore gates round-trip from
 * drawn data coordinates onto a GatingSet, including a calibration check that a
 * full-range rectangle captures every event (coordinate fidelity).
 */
class GatingCommandTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static Path sample;
    private RBridge bridge;
    private double xmin, xmax, ymin, ymax;
    private int total;

    @BeforeAll
    static void locate() {
        Path cwd = Paths.get("").toAbsolutePath();
        Path repoRoot = Files.exists(cwd.resolve("engine")) ? cwd : cwd.getParent();
        sample = repoRoot.resolve("test-assests").resolve("fcs").resolve("multi stain")
                .resolve("multistain-bv421-pe-fitc-apc-bv711-AF700.fcs");
        System.setProperty("streamflow.engine",
                repoRoot.resolve("engine").resolve("streamflow_engine.R").toString());
    }

    private static boolean rAvailable() {
        try {
            Process p = new ProcessBuilder(RPaths.rscript().toString(), "--version")
                    .redirectErrorStream(true).start();
            return p.waitFor(15, TimeUnit.SECONDS) && p.exitValue() == 0;
        } catch (Exception e) {
            return false;
        }
    }

    @BeforeEach
    void setUp() throws Exception {
        assumeTrue(rAvailable(), "Rscript not available");
        assumeTrue(Files.exists(sample), "multistain sample missing");
        bridge = RBridge.start();
        JsonNode caps = bridge.submit("capabilities", null, null).future().get(20, TimeUnit.SECONDS)
                .path("packages");
        assumeTrue(caps.path("flowCore").asBoolean(false)
                && caps.path("flowWorkspace").asBoolean(false), "R stack not installed");

        ObjectNode load = JSON.createObjectNode();
        ArrayNode files = JSON.createArrayNode();
        files.add(sample.toString().replace('\\', '/'));
        load.set("files", files);
        bridge.submit("load_fcs", load, null).future().get(120, TimeUnit.SECONDS);

        // Get the FSC-A/SSC-A data extent from a render.
        ObjectNode r = JSON.createObjectNode();
        r.put("x", "FSC-A").put("y", "SSC-A").put("type", "pseudocolor");
        JsonNode plot = bridge.submit("render_plot", r, null).future().get(120, TimeUnit.SECONDS);
        xmin = plot.path("xrange").get(0).asDouble(); xmax = plot.path("xrange").get(1).asDouble();
        ymin = plot.path("yrange").get(0).asDouble(); ymax = plot.path("yrange").get(1).asDouble();
        try { Files.deleteIfExists(Paths.get(plot.path("png").asText())); } catch (Exception ignored) {}
    }

    @AfterEach
    void tearDown() {
        if (bridge != null) bridge.close();
    }

    private JsonNode addRect(String name, double x1, double x2, double y1, double y2) throws Exception {
        ObjectNode a = JSON.createObjectNode();
        a.put("name", name).put("parent", "root").put("type", "rectangle")
                .put("x", "FSC-A").put("y", "SSC-A").put("sample", sample.getFileName().toString());
        ObjectNode coords = a.putObject("coords");
        coords.putArray("x").add(x1).add(x2);
        coords.putArray("y").add(y1).add(y2);
        return bridge.submit("add_gate", a, null).future().get(120, TimeUnit.SECONDS);
    }

    @Test
    void fullRangeRectangleCapturesAllEvents() throws Exception {
        // Extend a hair beyond the data extent so all events are strictly inside.
        double dx = (xmax - xmin) * 0.01, dy = (ymax - ymin) * 0.01;
        JsonNode r = addRect("All", xmin - dx, xmax + dx, ymin - dy, ymax + dy);
        total = r.path("parent_count").asInt();
        assertTrue(total > 0, "sample should have events");
        assertEquals(total, r.path("count").asInt(),
                "a full-range gate must capture every event (coordinate fidelity)");
    }

    @Test
    void centralRectangleIsAStrictSubset() throws Exception {
        JsonNode r = addRect("Center",
                xmin + (xmax - xmin) * 0.25, xmin + (xmax - xmin) * 0.75,
                ymin + (ymax - ymin) * 0.25, ymin + (ymax - ymin) * 0.75);
        int count = r.path("count").asInt();
        int parent = r.path("parent_count").asInt();
        assertTrue(count > 0 && count < parent,
                "central gate should be a strict subset: " + count + " of " + parent);
        assertTrue(r.path("percent").asDouble() > 0 && r.path("percent").asDouble() < 100);
    }

    @Test
    void polygonGateAndListAndRemove() throws Exception {
        ObjectNode a = JSON.createObjectNode();
        a.put("name", "Poly").put("parent", "root").put("type", "polygon")
                .put("x", "FSC-A").put("y", "SSC-A").put("sample", sample.getFileName().toString());
        ObjectNode coords = a.putObject("coords");
        double mx = (xmin + xmax) / 2, my = (ymin + ymax) / 2;
        coords.putArray("x").add(xmin).add(mx).add(xmax).add(mx);
        coords.putArray("y").add(my).add(ymin).add(my).add(ymax);
        JsonNode added = bridge.submit("add_gate", a, null).future().get(120, TimeUnit.SECONDS);
        assertTrue(added.path("count").asInt() > 0, "polygon should capture events");

        JsonNode list = bridge.submit("list_gates", null, null).future().get(60, TimeUnit.SECONDS);
        assertTrue(list.path("gates").size() >= 1, "list should include the polygon");

        ObjectNode rm = JSON.createObjectNode();
        rm.put("node", "/Poly");
        JsonNode removed = bridge.submit("remove_gate", rm, null).future().get(60, TimeUnit.SECONDS);
        assertEquals("/Poly", removed.path("removed").asText());
    }
}
