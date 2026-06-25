package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
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
 * Phase 1 integration test — drives real FCS files (test-assests/fcs) through the
 * engine's setup + workspace commands. Requires flowCore + flowWorkspace in the
 * R library; skips gracefully (via the engine 'capabilities' probe) otherwise.
 */
class SetupCommandTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static Path repoRoot;
    private static Path fcsDir;

    private RBridge bridge;

    private static Path multiStainDir;

    @BeforeAll
    static void locate() {
        Path cwd = Paths.get("").toAbsolutePath();           // app/
        repoRoot = Files.exists(cwd.resolve("engine")) ? cwd : cwd.getParent();
        fcsDir = repoRoot.resolve("test-assests").resolve("fcs");
        // The "multi stain" folder is a single panel: the multistain sample + its
        // single-colour compensation controls + unstained (all same channels).
        multiStainDir = fcsDir.resolve("multi stain");
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
        assumeTrue(rAvailable(), "Rscript not available; set -Dstreamflow.rscript=");
        assumeTrue(Files.isDirectory(fcsDir), "test FCS folder missing: " + fcsDir);
        bridge = RBridge.start();
        JsonNode caps = bridge.submit("capabilities", null, null).future().get(20, TimeUnit.SECONDS)
                .path("packages");
        assumeTrue(caps.path("flowCore").asBoolean(false) && caps.path("flowWorkspace").asBoolean(false),
                "flowCore/flowWorkspace not installed in R library; skipping Phase 1 integration");
    }

    @AfterEach
    void tearDown() {
        if (bridge != null) bridge.close();
    }

    @Test
    void loadsCompatibleFcsAndSummarizes() throws Exception {
        JsonNode r = loadFcs(); // the "multi stain" panel (sample + controls, one layout)
        assertTrue(r.path("n_samples").asInt() >= 2, "expected multiple panel-compatible samples");
        assertTrue(r.path("channels").size() > 0, "expected channels");
        assertTrue(r.path("samples").get(0).path("events").asInt() > 0, "expected event counts");
    }

    @Test
    void mixedPanelsGiveClearError() throws Exception {
        // Recursively loading ALL of test-assests/fcs mixes the single/dual/multi
        // panels (different channel layouts) — engine should reject clearly.
        ObjectNode args = JSON.createObjectNode();
        args.put("folder", fcsDir.toString().replace('\\', '/'));
        args.put("recursive", true);
        var ex = org.junit.jupiter.api.Assertions.assertThrows(
                java.util.concurrent.ExecutionException.class,
                () -> bridge.submit("load_fcs", args, null).future().get(120, TimeUnit.SECONDS));
        assertTrue(ex.getCause() instanceof RJobException, "expected RJobException");
        assertTrue(ex.getCause().getMessage().toLowerCase().contains("channel layout"),
                "error should explain the panel mismatch: " + ex.getCause().getMessage());
    }

    @Test
    void savesAndReloadsWorkspace() throws Exception {
        int n = loadFcs().path("n_samples").asInt();
        Path sfw = Files.createTempFile("sf_test_", ".sfw");
        Files.deleteIfExists(sfw); // engine writes it fresh

        ObjectNode saveArgs = JSON.createObjectNode();
        saveArgs.put("file", sfw.toString().replace('\\', '/'));
        JsonNode saved = bridge.submit("save_workspace", saveArgs, null).future().get(60, TimeUnit.SECONDS);
        assertTrue(saved.path("saved").asBoolean(), "workspace should report saved");
        assertTrue(Files.exists(sfw) && Files.size(sfw) > 0, "sfw file should exist and be non-empty");

        bridge.submit("reset", null, null).future().get(15, TimeUnit.SECONDS);

        ObjectNode loadArgs = JSON.createObjectNode();
        loadArgs.put("file", sfw.toString().replace('\\', '/'));
        JsonNode reloaded = bridge.submit("load_workspace", loadArgs, null).future().get(60, TimeUnit.SECONDS);
        assertEquals(n, reloaded.path("n_samples").asInt(), "reloaded workspace sample count should match");

        Files.deleteIfExists(sfw);
    }

    private JsonNode loadFcs() throws Exception {
        // Load the "multi stain" folder — one consistent panel (sample + controls).
        ObjectNode args = JSON.createObjectNode();
        args.put("folder", multiStainDir.toString().replace('\\', '/'));
        args.put("recursive", false);
        return bridge.submit("load_fcs", args, null).future().get(120, TimeUnit.SECONDS);
    }
}
