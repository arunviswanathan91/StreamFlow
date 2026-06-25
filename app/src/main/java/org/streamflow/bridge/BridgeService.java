package org.streamflow.bridge;

import com.fasterxml.jackson.databind.JsonNode;
import javafx.application.Platform;
import javafx.beans.property.ReadOnlyBooleanProperty;
import javafx.beans.property.ReadOnlyBooleanWrapper;
import javafx.concurrent.Task;

/**
 * JavaFX-facing facade over {@link RBridge}. Wraps each engine command in a
 * {@link Task} so the UI binds to {@code progressProperty()} /
 * {@code messageProperty()} and stays responsive while R computes. A
 * {@code busy} property reflects whether a job is currently running so the UI
 * can grey out R-dependent controls (the single-worker UX rule).
 */
public final class BridgeService implements AutoCloseable {

    private final RBridge bridge;
    private final ReadOnlyBooleanWrapper busy = new ReadOnlyBooleanWrapper(false);

    private BridgeService(RBridge bridge) {
        this.bridge = bridge;
    }

    /** Start the engine (blocks until ready) and wrap it in a service. */
    public static BridgeService start() throws Exception {
        return new BridgeService(RBridge.start());
    }

    public ReadOnlyBooleanProperty busyProperty() {
        return busy.getReadOnlyProperty();
    }

    public boolean isAlive() {
        return bridge.isAlive();
    }

    /**
     * Build (but do not start) a Task that runs an engine command. The caller
     * starts it on a background thread; the task forwards R progress to its own
     * progress/message properties and supports cancellation.
     *
     * <p>Cancellation is cooperative: {@link Task#cancel()} drops the engine's
     * cancel flag and lets the in-flight R loop stop at its next checkpoint.
     */
    public Task<JsonNode> command(String cmd, JsonNode args) {
        return new RCommandTask(cmd, args);
    }

    @Override
    public void close() {
        bridge.close();
    }

    private final class RCommandTask extends Task<JsonNode> {
        private final String cmd;
        private final JsonNode args;
        private volatile long jobId = -1;

        RCommandTask(String cmd, JsonNode args) {
            this.cmd = cmd;
            this.args = args;
        }

        @Override
        protected JsonNode call() throws Exception {
            setBusy(true);
            try {
                // submit() returns immediately with the job id; we attach a
                // progress consumer and block on the future from this background
                // task thread, keeping the FX thread free.
                RJob job = bridge.submit(cmd, args, p -> {
                    if (!Double.isNaN(p.frac())) updateProgress(p.frac(), 1.0);
                    if (p.message() != null) updateMessage(p.message());
                });
                jobId = job.id();
                if (isCancelled()) bridge.cancel(jobId); // cancelled between submit and now
                return job.future().get();
            } finally {
                setBusy(false);
            }
        }

        @Override
        protected void cancelled() {
            if (jobId >= 0) bridge.cancel(jobId);
            super.cancelled();
        }
    }

    private void setBusy(boolean value) {
        if (Platform.isFxApplicationThread()) {
            busy.set(value);
        } else {
            Platform.runLater(() -> busy.set(value));
        }
    }
}
