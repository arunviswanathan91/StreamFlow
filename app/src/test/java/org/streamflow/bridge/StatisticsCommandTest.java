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

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Phase 3c — Statistics. Computes per-population count / %total / MFI on a gated
 * sample and checks root + gate rows carry counts and per-channel MFI.
 */
class StatisticsCommandTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static Path sample;
    private RBridge bridge;

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
    }

    @AfterEach
    void tearDown() {
        if (bridge != null) bridge.close();
    }

    @Test
    void computesPerPopulationStats() throws Exception {
        // Establish data extent, then add a central rectangle gate.
        ObjectNode r = JSON.createObjectNode();
        r.put("x", "FSC-A").put("y", "SSC-A");
        JsonNode plot = bridge.submit("render_plot", r, null).future().get(120, TimeUnit.SECONDS);
        double xmin = plot.path("xrange").get(0).asDouble(), xmax = plot.path("xrange").get(1).asDouble();
        double ymin = plot.path("yrange").get(0).asDouble(), ymax = plot.path("yrange").get(1).asDouble();
        try { Files.deleteIfExists(Paths.get(plot.path("png").asText())); } catch (Exception ignored) {}

        ObjectNode g = JSON.createObjectNode();
        g.put("name", "Cells").put("parent", "root").put("type", "rectangle")
                .put("x", "FSC-A").put("y", "SSC-A");
        ObjectNode coords = g.putObject("coords");
        coords.putArray("x").add(xmin + (xmax - xmin) * 0.2).add(xmin + (xmax - xmin) * 0.8);
        coords.putArray("y").add(ymin + (ymax - ymin) * 0.2).add(ymin + (ymax - ymin) * 0.8);
        bridge.submit("add_gate", g, null).future().get(120, TimeUnit.SECONDS);

        JsonNode stats = bridge.submit("compute_stats", null, null).future().get(120, TimeUnit.SECONDS);
        assertTrue(stats.path("channels").size() > 0, "expected fluorescence channels");
        JsonNode rows = stats.path("rows");
        assertTrue(rows.size() >= 2, "expected root + gate rows");

        boolean haveRoot = false, haveGate = false;
        int rootCount = 0, gateCount = 0;
        for (JsonNode row : rows) {
            String pop = row.path("population").asText();
            if (pop.equals("root")) { haveRoot = true; rootCount = row.path("count").asInt(); }
            if (pop.contains("Cells")) {
                haveGate = true; gateCount = row.path("count").asInt();
                assertTrue(row.path("mfi").size() > 0, "gate row should carry per-channel MFI");
            }
        }
        assertTrue(haveRoot && haveGate, "stats should include root and the gate population");
        assertTrue(gateCount > 0 && gateCount < rootCount, "gate should be a subset of root");
    }
}
