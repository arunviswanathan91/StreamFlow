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
 * Phase 3 — Visualization. Renders a density scatter + histogram for a sample and
 * checks the PNG and coordinate metadata (the same render the gating canvas reuses).
 */
class VisualizationCommandTest {

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
        assumeTrue(caps.path("flowCore").asBoolean(false), "flowCore not installed");
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
    void rendersDensityScatterWithCoordMetadata() throws Exception {
        ObjectNode args = JSON.createObjectNode();
        args.put("x", "FSC-A").put("y", "SSC-A").put("type", "scatter");
        JsonNode r = bridge.submit("render_plot", args, null).future().get(120, TimeUnit.SECONDS);

        Path png = Paths.get(r.path("png").asText());
        assertTrue(Files.exists(png) && Files.size(png) > 0, "PNG should be written: " + png);
        assertEquals(2, r.path("xrange").size(), "xrange should be [min,max]");
        assertEquals(2, r.path("yrange").size(), "yrange should be [min,max]");
        assertEquals(4, r.path("plotbox_px").size(), "plotbox should be [l,t,r,b]");
        Files.deleteIfExists(png);
    }

    @Test
    void rendersAllPlotTypes() throws Exception {
        for (String type : new String[]{"pseudocolor", "dot", "contour", "density", "zebra"}) {
            ObjectNode args = JSON.createObjectNode();
            args.put("x", "FSC-A").put("y", "SSC-A").put("type", type);
            JsonNode r = bridge.submit("render_plot", args, null).future().get(120, TimeUnit.SECONDS);
            Path png = Paths.get(r.path("png").asText());
            assertTrue(Files.exists(png) && Files.size(png) > 0, type + " PNG should be written");
            Files.deleteIfExists(png);
        }
    }

    @Test
    void rendersHistogram() throws Exception {
        ObjectNode args = JSON.createObjectNode();
        args.put("x", "PE-A").put("type", "histogram");
        JsonNode r = bridge.submit("render_plot", args, null).future().get(120, TimeUnit.SECONDS);
        Path png = Paths.get(r.path("png").asText());
        assertTrue(Files.exists(png) && Files.size(png) > 0, "histogram PNG should be written");
        Files.deleteIfExists(png);
    }
}
