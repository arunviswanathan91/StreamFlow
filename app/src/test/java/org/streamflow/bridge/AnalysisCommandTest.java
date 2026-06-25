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
 * Phase 4 — Dimensionality reduction + clustering (the formerly UI-freezing jobs).
 * Runs UMAP and FlowSOM on small event counts and checks the rendered output.
 */
class AnalysisCommandTest {

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
    void runsUmap() throws Exception {
        ObjectNode a = JSON.createObjectNode();
        a.put("method", "umap").put("n_events", 600);
        JsonNode r = bridge.submit("run_dimredux", a, null).future().get(180, TimeUnit.SECONDS);
        Path png = Paths.get(r.path("png").asText());
        assertTrue(Files.exists(png) && Files.size(png) > 0, "UMAP embedding PNG should exist");
        assertTrue(r.path("n").asInt() <= 600, "should subsample to n_events");
        Files.deleteIfExists(png);
    }

    @Test
    void runsFlowSomClustering() throws Exception {
        JsonNode caps = bridge.submit("capabilities", null, null).future().get(20, TimeUnit.SECONDS).path("packages");
        assumeTrue(caps.path("FlowSOM").asBoolean(false), "FlowSOM not installed");
        ObjectNode a = JSON.createObjectNode();
        a.put("method", "flowsom").put("k", 6).put("n_events", 2000);
        JsonNode r = bridge.submit("run_clustering", a, null).future().get(180, TimeUnit.SECONDS);
        Path png = Paths.get(r.path("png").asText());
        assertTrue(Files.exists(png) && Files.size(png) > 0, "cluster heatmap PNG should exist");
        assertTrue(r.path("clusters").size() > 0, "should return clusters");
        int totalClustered = 0;
        for (JsonNode c : r.path("clusters")) totalClustered += c.path("count").asInt();
        assertTrue(totalClustered > 0, "clusters should contain events");
        Files.deleteIfExists(png);
    }
}
