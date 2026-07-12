package org.streamflow.plugins;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import org.streamflow.bridge.RPaths;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.stream.Stream;

/**
 * Discovers R plugins by scanning for {@code <root>/engine/plugins/<id>/plugin.json}.
 *
 * Plugins live in this repo and are bundled only into an R-enabled build (see release.yml's
 * {@code bundle_r}); the default installer is Python-only, so {@link #rAvailable()} is what the UI
 * uses to decide whether to offer them at all. Discovery itself never needs R, which keeps it
 * unit-testable on a machine without an R install.
 */
public final class PluginRegistry {

    private static final ObjectMapper JSON = new ObjectMapper();

    private PluginRegistry() {}

    /** Candidate roots: the packaged app dir first, then the dev repo checkout. */
    static List<Path> pluginRoots() {
        List<Path> roots = new ArrayList<>();
        try { roots.add(RPaths.appDir().resolve("engine").resolve("plugins")); } catch (Exception ignored) {}
        try { roots.add(RPaths.repoRoot().resolve("engine").resolve("plugins")); } catch (Exception ignored) {}
        return roots;
    }

    /** True when an Rscript executable can be resolved (bundled R-Portable, R_HOME, a system R, or PATH). */
    public static boolean rAvailable() {
        try {
            Path rs = RPaths.rscript();
            // A bare "Rscript"/"Rscript.exe" means "hope it's on PATH" — only trust an absolute hit.
            return rs != null && rs.isAbsolute() && Files.exists(rs);
        } catch (Exception e) {
            return false;
        }
    }

    /** All plugins found under the first existing root. Never throws; a bad manifest is skipped. */
    public static List<PluginManifest> discover() {
        for (Path root : pluginRoots()) {
            if (root != null && Files.isDirectory(root)) return discoverIn(root);
        }
        return List.of();
    }

    /** Parse the {@code plugin.json} of every sub-directory. Malformed manifests are skipped, not fatal. */
    public static List<PluginManifest> discoverIn(Path root) {
        List<PluginManifest> out = new ArrayList<>();
        try (Stream<Path> dirs = Files.list(root)) {
            for (Path dir : dirs.filter(Files::isDirectory).toList()) {
                Path manifest = dir.resolve("plugin.json");
                if (!Files.isRegularFile(manifest)) continue;
                try {
                    out.add(parse(dir, manifest));
                } catch (Exception ignored) {
                    // A single broken plugin must not hide the healthy ones.
                }
            }
        } catch (IOException ignored) { }
        out.sort(java.util.Comparator.comparing(PluginManifest::name, String.CASE_INSENSITIVE_ORDER));
        return out;
    }

    static PluginManifest parse(Path dir, Path manifestFile) throws IOException {
        JsonNode n = JSON.readTree(Files.readString(manifestFile));
        JsonNode entry = n.path("entry");
        String script = entry.path("script").asText(null);
        String function = entry.path("function").asText(null);
        if (script == null || function == null) {
            throw new IOException("plugin.json must declare entry.script and entry.function");
        }
        String id = n.path("id").asText(dir.getFileName().toString());

        List<String> pkgs = new ArrayList<>();
        n.path("requires").path("packages").forEach(p -> pkgs.add(p.asText()));

        List<PluginManifest.Param> params = new ArrayList<>();
        for (JsonNode p : n.path("params")) {
            params.add(new PluginManifest.Param(
                    p.path("id").asText(),
                    p.path("label").asText(p.path("id").asText()),
                    p.path("type").asText("string"),
                    p.path("required").asBoolean(false),
                    p.path("help").asText(null),
                    p.has("min") ? p.path("min").asDouble() : null,
                    p.has("max") ? p.path("max").asDouble() : null,
                    p.has("default") ? toPlain(p.path("default")) : null));
        }

        List<String> outputs = new ArrayList<>();
        n.path("outputs").forEach(o -> outputs.add(o.asText()));

        return new PluginManifest(dir, id,
                n.path("name").asText(id),
                n.path("version").asText("0.0.0"),
                n.path("description").asText(""),
                n.path("status").asText("experimental"),
                script, function, pkgs, params, outputs);
    }

    private static Object toPlain(JsonNode v) {
        if (v.isBoolean()) return v.asBoolean();
        if (v.isInt() || v.isLong()) return v.asLong();
        if (v.isNumber()) return v.asDouble();
        return v.asText();
    }
}
