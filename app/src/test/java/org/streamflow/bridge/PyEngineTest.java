package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.junit.jupiter.api.AfterAll;
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
 * M0 verification: the Java bridge spawns the Python (FlowKit) engine and the
 * data path works end-to-end — capabilities, FCS load, and the binary event blob
 * for native rendering.
 */
class PyEngineTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static Path repoRoot, pyExe, multiStain;
    private RBridge bridge;

    @BeforeAll
    static void locate() {
        Path cwd = Paths.get("").toAbsolutePath();
        repoRoot = Files.exists(cwd.resolve("engine")) ? cwd : cwd.getParent();
        pyExe = repoRoot.resolve("engine").resolve("py-env").resolve("Scripts").resolve("python.exe");
        multiStain = repoRoot.resolve("test-assests").resolve("fcs").resolve("multi stain");
        System.setProperty("streamflow.python", pyExe.toString());
        System.setProperty("streamflow.engine.py",
                repoRoot.resolve("engine").resolve("streamflow_engine.py").toString());
    }

    @BeforeEach
    void setUp() throws Exception {
        assumeTrue(Files.exists(pyExe), "Python venv not found; run engine setup");
        bridge = RBridge.start();
    }

    @AfterEach
    void tearDown() {
        if (bridge != null) bridge.close();
    }

    @AfterAll
    static void clearEngineSelection() {
        // Don't let the Python selection leak into R-engine tests in a full run.
        System.clearProperty("streamflow.python");
        System.clearProperty("streamflow.engine.py");
    }

    @Test
    void pythonEngineReportsFlowkit() throws Exception {
        JsonNode caps = bridge.submit("capabilities", null, null).future().get(20, TimeUnit.SECONDS)
                .path("packages");
        assertTrue(caps.path("flowkit").asBoolean(false), "FlowKit should be available");
    }

    @Test
    void loadsFcsViaFlowKit() throws Exception {
        assumeTrue(Files.isDirectory(multiStain), "multi stain folder missing");
        ObjectNode args = JSON.createObjectNode();
        args.put("folder", multiStain.toString().replace('\\', '/'));
        args.put("recursive", false);
        JsonNode r = bridge.submit("load_fcs", args, null).future().get(120, TimeUnit.SECONDS);
        assertEquals(8, r.path("n_samples").asInt(), "expected 8 samples in multi stain");
        assertTrue(r.path("channels").size() > 0, "expected channels");
    }

    @Test
    void streamsEventBlobForNativeRendering() throws Exception {
        assumeTrue(Files.isDirectory(multiStain), "multi stain folder missing");
        ObjectNode load = JSON.createObjectNode();
        ArrayNode files = JSON.createArrayNode();
        files.add(multiStain.resolve("multistain-bv421-pe-fitc-apc-bv711-AF700.fcs")
                .toString().replace('\\', '/'));
        load.set("files", files);
        bridge.submit("load_fcs", load, null).future().get(120, TimeUnit.SECONDS);

        ObjectNode args = JSON.createObjectNode();
        args.put("n", 50000);
        JsonNode r = bridge.submit("get_events", args, null).future().get(60, TimeUnit.SECONDS);
        int rows = r.path("rows").asInt(), cols = r.path("cols").asInt();
        assertTrue(rows > 0 && cols > 0, "expected an event matrix");
        Path bin = Paths.get(r.path("file").asText());
        assertTrue(Files.exists(bin), "event blob should be written: " + bin);
        assertEquals((long) rows * cols * 4, Files.size(bin), "blob should be rows*cols float32");
        Files.deleteIfExists(bin);
    }
}
