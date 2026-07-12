package org.streamflow.plugins;

import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Optional;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Discovery + manifest parsing must work with NO R installed, so these run everywhere.
 * They read the real plugins shipped in engine/plugins/, which also guards the manifests
 * themselves against typos (a broken plugin.json fails the build).
 */
class PluginRegistryTest {

    /** engine/plugins/ relative to the repo root, located from the test's working dir (app/). */
    private static Path pluginsRoot() {
        for (Path p : List.of(Path.of("..", "engine", "plugins"), Path.of("engine", "plugins"))) {
            if (Files.isDirectory(p)) return p;
        }
        return null;
    }

    @Test
    void discoversTheShippedPluginsAndParsesTheirManifests() {
        Path root = pluginsRoot();
        assertNotNull(root, "engine/plugins not found from the test working directory");

        List<PluginManifest> plugins = PluginRegistry.discoverIn(root);
        assertFalse(plugins.isEmpty(), "expected at least one plugin under engine/plugins");

        // Every manifest must declare a runnable entry point that actually exists on disk.
        for (PluginManifest m : plugins) {
            assertNotNull(m.id(), "plugin id");
            assertNotNull(m.entryScript(), "entry.script for " + m.id());
            assertNotNull(m.entryFunction(), "entry.function for " + m.id());
            assertTrue(Files.isRegularFile(m.entryScriptPath()),
                    "entry script missing for " + m.id() + ": " + m.entryScriptPath());
        }
    }

    @Test
    void peacoqcManifestDeclaresItsContract() {
        Path root = pluginsRoot();
        assertNotNull(root);
        Optional<PluginManifest> peaco = PluginRegistry.discoverIn(root).stream()
                .filter(p -> p.id().equals("peacoqc")).findFirst();
        assertTrue(peaco.isPresent(), "peacoqc plugin should be discovered");

        PluginManifest m = peaco.get();
        assertEquals("streamflow_run", m.entryFunction());
        assertEquals("peacoqc.R", m.entryScript());
        // It cannot run without these R packages; the UI surfaces them when R is missing one.
        assertTrue(m.rPackages().containsAll(List.of("flowCore", "PeacoQC")), m.rPackages().toString());
        // The app consumes kept_indices to build the cleaned sub-population.
        assertTrue(m.outputs().contains("kept_indices"), m.outputs().toString());
        assertTrue(m.params().stream().anyMatch(p -> p.id().equals("mad")), "mad param");
    }

    @Test
    void aDirectoryWithoutAManifestIsIgnoredAndDoesNotThrow(@org.junit.jupiter.api.io.TempDir Path tmp)
            throws Exception {
        Files.createDirectory(tmp.resolve("not-a-plugin"));   // no plugin.json inside
        Files.writeString(tmp.resolve("stray.txt"), "x");      // a loose file, not a directory
        assertTrue(PluginRegistry.discoverIn(tmp).isEmpty());
    }

    @Test
    void aMalformedManifestIsSkippedRatherThanFailingDiscovery(@org.junit.jupiter.api.io.TempDir Path tmp)
            throws Exception {
        Path bad = Files.createDirectory(tmp.resolve("bad"));
        Files.writeString(bad.resolve("plugin.json"), "{ \"id\": \"bad\" }");   // no entry{} block
        assertTrue(PluginRegistry.discoverIn(tmp).isEmpty(), "manifest without entry must be skipped");
    }
}
