package org.streamflow.plugins;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.concurrent.Task;
import org.streamflow.bridge.RPaths;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Map;

/**
 * Executes an R plugin through {@code engine/run_plugin.R}.
 *
 * The shim always writes a JSON response — including {@code {"ok": false, "error": ...}} — so this
 * never has to parse an R stack trace. Anything R prints to stdout/stderr is captured as the plugin
 * log and surfaced to the user when a run fails.
 */
public final class PluginRunner {

    private static final ObjectMapper JSON = new ObjectMapper();

    private PluginRunner() {}

    /** Thrown when the plugin ran but reported {@code ok:false}, or the runner itself failed. */
    public static class PluginException extends RuntimeException {
        public PluginException(String message) { super(message); }
    }

    /** Locate {@code engine/run_plugin.R} next to the packaged app or in the dev checkout. */
    static Path runnerScript() {
        for (Path base : List.of(RPaths.appDir(), RPaths.repoRoot())) {
            if (base == null) continue;
            Path p = base.resolve("engine").resolve("run_plugin.R");
            if (Files.isRegularFile(p)) return p;
        }
        throw new PluginException("engine/run_plugin.R not found (is the R runway installed?)");
    }

    /**
     * Background task that runs {@code plugin} with {@code request} and yields the parsed response.
     * Never runs on the FX thread — plugins can take minutes.
     */
    public static Task<JsonNode> run(PluginManifest plugin, Map<String, Object> request) {
        return new Task<>() {
            @Override protected JsonNode call() throws Exception {
                Path rscript = RPaths.rscript();
                if (rscript == null) throw new PluginException("No R installation found.");

                Path work = Files.createTempDirectory("sf-plugin-");
                Path reqFile = work.resolve("request.json");
                Path respFile = work.resolve("response.json");
                try {
                    ObjectNode req = JSON.createObjectNode();
                    request.forEach((k, v) -> req.set(k, JSON.valueToTree(v)));
                    Files.writeString(reqFile, JSON.writeValueAsString(req));

                    ProcessBuilder pb = new ProcessBuilder(
                            rscript.toString(), runnerScript().toString(),
                            plugin.dir().toString(), reqFile.toString(), respFile.toString());
                    pb.redirectErrorStream(true);
                    Process proc = pb.start();

                    StringBuilder log = new StringBuilder();
                    try (var r = proc.inputReader()) {
                        String line;
                        while ((line = r.readLine()) != null) {
                            log.append(line).append('\n');
                            updateMessage(line);            // surface progress in the UI
                            if (isCancelled()) { proc.destroy(); throw new InterruptedException("cancelled"); }
                        }
                    }
                    int exit = proc.waitFor();

                    if (!Files.isRegularFile(respFile)) {
                        throw new PluginException("Plugin produced no response (exit " + exit + ").\n"
                                + tail(log.toString()));
                    }
                    JsonNode resp = JSON.readTree(Files.readString(respFile));
                    if (!resp.path("ok").asBoolean(false)) {
                        throw new PluginException(resp.path("error").asText("Plugin failed.")
                                + "\n" + tail(log.toString()));
                    }
                    return resp;
                } finally {
                    deleteQuietly(respFile);
                    deleteQuietly(reqFile);
                    deleteQuietly(work);
                }
            }
        };
    }

    /** 0-based row indices from a plugin response field (e.g. {@code kept_indices}). */
    public static int[] indices(JsonNode resp, String field) {
        JsonNode arr = resp.path(field);
        if (!arr.isArray()) return new int[0];
        int[] out = new int[arr.size()];
        for (int i = 0; i < out.length; i++) out[i] = arr.get(i).asInt();
        return out;
    }

    private static String tail(String s) {
        String[] lines = s.strip().split("\n");
        int from = Math.max(0, lines.length - 12);
        return String.join("\n", java.util.Arrays.copyOfRange(lines, from, lines.length));
    }

    private static void deleteQuietly(Path p) {
        try { Files.deleteIfExists(p); } catch (IOException ignored) { }
    }
}
