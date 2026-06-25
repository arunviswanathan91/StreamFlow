package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.BufferedReader;
import java.io.BufferedWriter;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStreamWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.Callable;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.LinkedBlockingQueue;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Consumer;

/**
 * Owns the persistent R engine process and the stdio JSON protocol.
 *
 * <h2>Threading model</h2>
 * R is single-threaded and holds one mutable GatingSet, so every command is
 * funnelled through a <b>single-thread executor</b> ({@link #commandExec}) that
 * is the sole writer of R's stdin. A dedicated <b>reader thread</b> consumes
 * R's stdout, parses each JSON line, and routes it (by request id) to a
 * per-request {@link BlockingQueue}. The executor task that issued the request
 * blocks on that queue until a terminal {@code result}/{@code error}/
 * {@code cancelled} frame arrives, forwarding {@code progress} frames to the
 * caller's consumer along the way.
 *
 * <h2>Lifecycle &amp; orphan safety</h2>
 * On shutdown the R process tree is killed (Windows {@code taskkill /T /F}; R
 * spawns native children). As a second guard against a hard JVM crash that
 * skips shutdown hooks, the engine self-terminates when its stdin reaches EOF.
 */
public final class RBridge implements AutoCloseable {

    private static final Logger log = LoggerFactory.getLogger(RBridge.class);
    private static final ObjectMapper JSON = new ObjectMapper();
    private static final long READY_TIMEOUT_MS = 60_000;
    private static final boolean IS_WINDOWS =
            System.getProperty("os.name", "").toLowerCase().contains("win");

    private final Process process;
    private final BufferedWriter toR;
    private final Path controlDir;

    private final ExecutorService commandExec =
            Executors.newSingleThreadExecutor(r -> namedDaemon(r, "R-dispatch"));
    private final Thread readerThread;
    private final ScheduledExecutorService heartbeat =
            Executors.newSingleThreadScheduledExecutor(r -> namedDaemon(r, "R-heartbeat"));

    private final AtomicLong idSeq = new AtomicLong(1);
    private final ConcurrentHashMap<Long, BlockingQueue<JsonNode>> pending = new ConcurrentHashMap<>();
    private final BlockingQueue<JsonNode> readyQueue = new LinkedBlockingQueue<>();
    private volatile boolean closing = false;

    private RBridge(Process process, Path controlDir) {
        this.process = process;
        this.controlDir = controlDir;
        this.toR = new BufferedWriter(new OutputStreamWriter(process.getOutputStream(), StandardCharsets.UTF_8));

        // stderr -> log (pure log sink, never the protocol)
        Thread errPump = namedDaemon(() -> pump(process.getErrorStream()), "R-stderr");
        errPump.start();

        this.readerThread = namedDaemon(this::readLoop, "R-stdout-reader");
        this.readerThread.start();
    }

    /** Spawn the engine and block until it announces {@code ready}. */
    public static RBridge start() throws IOException {
        Path controlDir = Files.createTempDirectory("streamflow_ctl_");
        java.util.List<String> cmd = EnginePaths.command(controlDir);

        ProcessBuilder pb = new ProcessBuilder(cmd);
        pb.redirectErrorStream(false);

        if (!EnginePaths.usePython()) {
            Path rhome = RPaths.bundledRHome();
            if (rhome != null) {
                pb.environment().put("R_HOME", rhome.toString());
                Path lib = RPaths.bundledRLibrary();
                if (lib != null) pb.environment().put("R_LIBS_USER", lib.toString());
            }
        }

        log.info("Starting engine: {}", String.join(" ", cmd));
        Process proc = pb.start();
        RBridge bridge = new RBridge(proc, controlDir);
        bridge.awaitReady();
        bridge.heartbeat.scheduleAtFixedRate(bridge::pingQuietly, 5, 5, TimeUnit.SECONDS);
        Runtime.getRuntime().addShutdownHook(new Thread(bridge::killProcessTree, "R-shutdown"));
        return bridge;
    }

    private void awaitReady() throws IOException {
        try {
            JsonNode ready = readyQueue.poll(READY_TIMEOUT_MS, TimeUnit.MILLISECONDS);
            if (ready == null) {
                killProcessTree();
                throw new IOException("R engine did not become ready within "
                        + READY_TIMEOUT_MS + "ms");
            }
            log.info("R engine ready: {}", ready);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new IOException("Interrupted waiting for R engine", e);
        }
    }

    // ---- public API ---------------------------------------------------------

    /**
     * Submit a command to the R engine. Runs on the single command thread, so
     * calls are serialized in submission order. The returned {@link Future}
     * completes with R's {@code result} payload, or fails with
     * {@link RJobException} / {@link RJobCancelledException}.
     *
     * @param onProgress invoked (on the dispatch thread) for each progress frame; may be null
     */
    public RJob submit(String cmd, JsonNode args, Consumer<RProgress> onProgress) {
        long id = idSeq.getAndIncrement();
        Callable<JsonNode> task = () -> runCommand(id, cmd, args, onProgress);
        Future<JsonNode> future = commandExec.submit(task);
        return new RJob(id, future);
    }

    /** Request cooperative cancellation of an in-flight job by dropping its flag file. */
    public void cancel(long id) {
        try {
            Files.createFile(controlDir.resolve("cancel_" + id + ".flag"));
        } catch (IOException e) {
            log.warn("Could not write cancel flag for job {}: {}", id, e.toString());
        }
    }

    public boolean isAlive() {
        return process.isAlive();
    }

    // ---- command execution (on the single command thread) -------------------

    private JsonNode runCommand(long id, String cmd, JsonNode args, Consumer<RProgress> onProgress)
            throws IOException, InterruptedException {
        BlockingQueue<JsonNode> q = new LinkedBlockingQueue<>();
        pending.put(id, q);
        try {
            ObjectNode req = JSON.createObjectNode();
            req.put("id", id);
            req.put("cmd", cmd);
            if (args != null) req.set("args", args);
            writeLine(JSON.writeValueAsString(req));

            while (true) {
                JsonNode msg = q.take();
                String type = msg.path("type").asText();
                switch (type) {
                    case "progress" -> {
                        if (onProgress != null) {
                            onProgress.accept(new RProgress(
                                    id,
                                    msg.path("frac").asDouble(Double.NaN),
                                    msg.path("msg").isNull() ? null : msg.path("msg").asText(null)));
                        }
                    }
                    case "result" -> {
                        return msg;
                    }
                    case "pong" -> {
                        return msg;   // ping probe completes (id-correlated pong)
                    }
                    case "cancelled" -> throw new RJobCancelledException(id);
                    case "error" -> throw new RJobException(
                            msg.path("message").asText("Unknown R error"),
                            jsonToStringList(msg.path("trace")));
                    default -> log.warn("Unexpected reply type '{}' for job {}", type, id);
                }
            }
        } finally {
            pending.remove(id);
        }
    }

    // ---- reader thread ------------------------------------------------------

    private void readLoop() {
        try (BufferedReader in = new BufferedReader(
                new InputStreamReader(process.getInputStream(), StandardCharsets.UTF_8))) {
            String line;
            while ((line = in.readLine()) != null) {
                if (line.isBlank()) continue;
                JsonNode msg;
                try {
                    msg = JSON.readTree(line);
                } catch (Exception parse) {
                    log.error("Un-parseable line from R (protocol violation): {}", line);
                    continue;
                }
                route(msg);
            }
        } catch (IOException e) {
            if (!closing) log.warn("R stdout reader ended: {}", e.toString());
        }
        log.info("R stdout closed; engine has exited.");
    }

    private void route(JsonNode msg) {
        String type = msg.path("type").asText("");
        if ("ready".equals(type)) {
            readyQueue.offer(msg);
            return;
        }
        // Route id-correlated replies (incl. an id-tagged pong) to their job FIRST,
        // so a dev-console ping completes; a heartbeat pong has no pending job and
        // falls through to be swallowed below.
        if (msg.has("id") && !msg.path("id").isNull()) {
            long id = msg.path("id").asLong();
            BlockingQueue<JsonNode> q = pending.get(id);
            if (q != null) {
                q.offer(msg);
                return;
            }
        }
        if ("pong".equals(type)) {
            return; // heartbeat ack (no pending job)
        }
        log.debug("Unrouted R message: {}", msg);
    }

    // ---- low-level IO -------------------------------------------------------

    private synchronized void writeLine(String line) throws IOException {
        toR.write(line);
        toR.write('\n');
        toR.flush();
    }

    private void pingQuietly() {
        try {
            ObjectNode ping = JSON.createObjectNode();
            ping.put("id", idSeq.getAndIncrement());
            ping.put("cmd", "ping");
            writeLine(JSON.writeValueAsString(ping));
        } catch (IOException e) {
            log.debug("Heartbeat write failed (engine may be exiting): {}", e.toString());
        }
    }

    private void pump(java.io.InputStream stream) {
        try (BufferedReader r = new BufferedReader(new InputStreamReader(stream, StandardCharsets.UTF_8))) {
            String line;
            while ((line = r.readLine()) != null) {
                log.info("[R] {}", line);
            }
        } catch (IOException ignored) {
            // stream closed on shutdown
        }
    }

    // ---- shutdown -----------------------------------------------------------

    @Override
    public void close() {
        closing = true;
        heartbeat.shutdownNow();
        commandExec.shutdownNow();
        // Polite shutdown: close stdin so the engine sees EOF and quits.
        try {
            toR.write("__shutdown__\n");
            toR.flush();
            toR.close();
        } catch (IOException ignored) {
        }
        try {
            if (!process.waitFor(3, TimeUnit.SECONDS)) {
                killProcessTree();
            }
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            killProcessTree();
        }
        try {
            deleteRecursive(controlDir);
        } catch (IOException ignored) {
        }
    }

    /** Hard kill of the R process and its descendants (R spawns BLAS/native children). */
    private void killProcessTree() {
        if (!process.isAlive()) return;
        long pid = process.pid();
        if (IS_WINDOWS) {
            try {
                new ProcessBuilder("taskkill", "/PID", Long.toString(pid), "/T", "/F")
                        .inheritIO().start().waitFor(5, TimeUnit.SECONDS);
            } catch (Exception e) {
                log.warn("taskkill failed for PID {}: {}", pid, e.toString());
            }
        }
        process.descendants().forEach(ProcessHandle::destroyForcibly);
        process.destroyForcibly();
    }

    // ---- helpers ------------------------------------------------------------

    private static java.util.List<String> jsonToStringList(JsonNode node) {
        java.util.List<String> out = new java.util.ArrayList<>();
        if (node != null && node.isArray()) {
            node.forEach(n -> out.add(n.asText()));
        }
        return out;
    }

    private static Thread namedDaemon(Runnable r, String name) {
        Thread t = new Thread(r, name);
        t.setDaemon(true);
        return t;
    }

    private static void deleteRecursive(Path dir) throws IOException {
        if (!Files.exists(dir)) return;
        try (var walk = Files.walk(dir)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> {
                try { Files.deleteIfExists(p); } catch (IOException ignored) {}
            });
        }
    }
}
