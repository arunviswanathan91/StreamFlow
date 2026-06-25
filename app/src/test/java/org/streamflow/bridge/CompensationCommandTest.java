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
import java.util.List;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Phase 2 — Compensation (Workflow 1 / 3). Loads the single-colour controls +
 * unstained from the "multi stain" folder, computes a spillover matrix with
 * flowCore, and applies it. Skips if the R stack or control files are absent.
 */
class CompensationCommandTest {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static Path repoRoot;
    private static Path controlsDir;

    private RBridge bridge;

    private static Path multiStainSample;

    @BeforeAll
    static void locate() {
        Path cwd = Paths.get("").toAbsolutePath();
        repoRoot = Files.exists(cwd.resolve("engine")) ? cwd : cwd.getParent();
        controlsDir = repoRoot.resolve("test-assests").resolve("fcs").resolve("multi stain");
        multiStainSample = controlsDir.resolve("multistain-bv421-pe-fitc-apc-bv711-AF700.fcs");
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
        assumeTrue(Files.isDirectory(controlsDir), "controls folder missing: " + controlsDir);
        bridge = RBridge.start();
        JsonNode caps = bridge.submit("capabilities", null, null).future().get(20, TimeUnit.SECONDS)
                .path("packages");
        assumeTrue(caps.path("flowCore").asBoolean(false), "flowCore not installed");
    }

    @AfterEach
    void tearDown() {
        if (bridge != null) bridge.close();
    }

    @Test
    void extractsAndAppliesEmbeddedSpillover() throws Exception {
        // Primary, deterministic path: the cytometer wrote a $SPILL matrix into the
        // FCS. Extract it and apply it (matches WF2 "apply the matrix").
        assumeTrue(Files.exists(multiStainSample), "multistain sample missing");
        ObjectNode load = JSON.createObjectNode();
        ArrayNode files = JSON.createArrayNode();
        files.add(multiStainSample.toString().replace('\\', '/'));
        load.set("files", files);
        bridge.submit("load_fcs", load, null).future().get(120, TimeUnit.SECONDS);

        JsonNode spill = bridge.submit("extract_spillover", null, null).future().get(60, TimeUnit.SECONDS);
        int nch = spill.path("channels").size();
        assertTrue(nch > 0, "embedded spillover should have channels");
        assertEquals(nch, spill.path("matrix").size(), "spillover matrix should be square");

        JsonNode applied = bridge.submit("apply_compensation", null, null).future().get(60, TimeUnit.SECONDS);
        assertTrue(applied.path("applied").asBoolean(), "compensation should apply");
    }

    @Test
    void computesSpilloverFromControls() throws Exception {
        // WF1: single-colour controls + unstained (exclude the biological multistain
        // sample). Deterministic peak-ratio method auto-detects each control's channel.
        ArrayNode files = JSON.createArrayNode();
        try (Stream<Path> s = Files.list(controlsDir)) {
            List<Path> controls = s.filter(p -> p.toString().toLowerCase().endsWith(".fcs"))
                    .filter(p -> {
                        String n = p.getFileName().toString().toLowerCase();
                        return n.contains("alone") || n.contains("unstain");
                    }).toList();
            assumeTrue(controls.size() >= 2, "need unstained + single-colour controls");
            controls.forEach(p -> files.add(p.toString().replace('\\', '/')));
        }
        ObjectNode load = JSON.createObjectNode();
        load.set("files", files);
        bridge.submit("load_fcs", load, null).future().get(120, TimeUnit.SECONDS);

        JsonNode spill = bridge.submit("compute_spillover", null, null).future().get(120, TimeUnit.SECONDS);
        int nch = spill.path("channels").size();
        assertTrue(nch > 0, "expected fluorescence channels");
        assertEquals(nch, spill.path("matrix").size(), "spillover matrix should be square");
        // every diagonal entry must be exactly 1 (self-channel normalised)
        for (int i = 0; i < nch; i++) {
            assertEquals(1.0, spill.path("matrix").get(i).get(i).asDouble(), 1e-9,
                    "diagonal should be 1 at index " + i);
        }
        JsonNode applied = bridge.submit("apply_compensation", null, null).future().get(60, TimeUnit.SECONDS);
        assertTrue(applied.path("applied").asBoolean(), "compensation should apply");
    }
}
