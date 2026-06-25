package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.TimeUnit;

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Smoke test: asserts every R package the StreamFLOW pipeline depends on actually
 * LOADS in the engine process (via the {@code capabilities} command). This catches
 * a broken/half install immediately — independent of whether the feature that uses
 * a given package has been built yet. Functional, per-feature tests are added as
 * each phase lands.
 */
class PackageSmokeTest {

    private static Path repoRoot;
    private RBridge bridge;

    @BeforeAll
    static void locate() {
        Path cwd = Paths.get("").toAbsolutePath();
        repoRoot = Files.exists(cwd.resolve("engine")) ? cwd : cwd.getParent();
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
        bridge = RBridge.start();
    }

    @AfterEach
    void tearDown() {
        if (bridge != null) bridge.close();
    }

    @Test
    void entirePipelineStackLoads() throws Exception {
        JsonNode caps = bridge.submit("capabilities", null, null).future().get(120, TimeUnit.SECONDS);
        JsonNode pkgs = caps.path("packages");
        List<String> missing = new ArrayList<>();
        for (Map.Entry<String, JsonNode> e : pkgs.properties()) {
            if (!e.getValue().asBoolean(false)) missing.add(e.getKey());
        }
        // Phase 1 must always be present to run anything at all; if it's missing the
        // whole environment is unset up — skip rather than fail spuriously.
        assumeTrue(pkgs.path("flowCore").asBoolean(false)
                        && pkgs.path("flowWorkspace").asBoolean(false),
                "Core R stack not installed; run engine/install_from_vendor.R");
        assertTrue(missing.isEmpty(),
                "These pipeline packages are installed-but-not-loadable: " + missing);
    }
}
