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
 * Phase 2 — Transformation. Loads the multistain sample, applies logicle and
 * arcsinh transforms to its fluorescence channels.
 */
class TransformationCommandTest {

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
    void appliesLogicleTransform() throws Exception {
        ObjectNode args = JSON.createObjectNode();
        args.put("method", "logicle");
        JsonNode r = bridge.submit("apply_transformation", args, null).future().get(120, TimeUnit.SECONDS);
        assertTrue(r.path("applied").asBoolean(), "logicle should apply");
        assertTrue(r.path("channels").size() > 0, "expected transformed fluor channels");
    }

    @Test
    void appliesArcsinhTransform() throws Exception {
        ObjectNode args = JSON.createObjectNode();
        args.put("method", "arcsinh");
        args.put("cofactor", 150);
        JsonNode r = bridge.submit("apply_transformation", args, null).future().get(120, TimeUnit.SECONDS);
        assertTrue(r.path("applied").asBoolean(), "arcsinh should apply");
    }
}
