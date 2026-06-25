package org.streamflow.ui;

import javafx.concurrent.Task;

import java.util.function.Consumer;

/**
 * Runs a bridge {@link Task} on a background thread while driving the shared
 * status bar (progress + cancel) on the FX thread. Implemented by the shell
 * ({@link MainController}) and handed to module controllers via {@link AppContext}
 * so every module reports progress and cancellation through one consistent UI.
 */
public interface JobRunner {

    /**
     * @param task      a not-yet-started bridge task (from {@code BridgeService.command})
     * @param onSuccess called on the FX thread with the task result on success
     */
    <T> void run(Task<T> task, Consumer<T> onSuccess);

    /** Append a line to the shared activity log / status area. */
    void status(String message);
}
