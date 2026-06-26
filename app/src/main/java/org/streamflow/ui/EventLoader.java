package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.concurrent.Task;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/**
 * Loads sample events from the engine on demand and caches them in the {@link WorkspaceModel},
 * so analysis modules (Classifier, Cross-Sample, …) can work from a user <em>selection</em>
 * instead of requiring each sample to be opened in a graph window first. Samples already cached
 * are skipped; the rest are fetched sequentially (one engine round-trip each) and the supplied
 * callback runs on the FX thread once every requested sample is available.
 */
final class EventLoader {

    private static final ObjectMapper JSON = new ObjectMapper();

    private EventLoader() {}

    /**
     * Ensure every name in {@code samples} has cached {@link EventData}, loading the missing ones
     * from the engine. {@code onProgress} is called with a status string as each loads (may be null);
     * {@code onDone} runs once all are cached.
     */
    static void ensureLoaded(AppContext ctx, List<String> samples,
                             Consumer<String> onProgress, Runnable onDone) {
        List<String> missing = new ArrayList<>();
        for (String s : samples) if (ctx.workspace().data(s) == null) missing.add(s);
        if (missing.isEmpty()) { onDone.run(); return; }
        loadNext(ctx, missing, 0, onProgress, onDone);
    }

    private static void loadNext(AppContext ctx, List<String> missing, int idx,
                                 Consumer<String> onProgress, Runnable onDone) {
        if (idx >= missing.size()) { onDone.run(); return; }
        String s = missing.get(idx);
        if (onProgress != null) onProgress.accept("Loading events: " + s
                + " (" + (idx + 1) + "/" + missing.size() + ")…");
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", s);
        Task<JsonNode> task = ctx.bridge().command("get_events", args);
        ctx.jobs().run(task, r -> {
            try {
                List<String> chans = new ArrayList<>();
                r.path("channels").forEach(n -> chans.add(n.asText()));
                Path bin = Paths.get(r.path("file").asText());
                EventData d = EventData.read(bin, chans, r.path("rows").asInt(), r.path("cols").asInt());
                try { Files.deleteIfExists(bin); } catch (Exception ignored) {}
                ctx.workspace().putData(s, d);
            } catch (Exception ignored) {
                // a sample that fails to load is simply skipped; the caller reports coverage
            }
            loadNext(ctx, missing, idx + 1, onProgress, onDone);
        });
    }
}
