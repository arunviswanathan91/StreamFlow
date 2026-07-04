package org.streamflow.ui;

/**
 * Implemented by module controllers that should re-read the {@link WorkspaceModel} (sample list,
 * channels, gating trees) when an experiment is loaded — so the user no longer has to press a manual
 * "Refresh" after File ▸ Load FCS… or Open Workspace (#31). {@link MainController} calls
 * {@link #refreshFromWorkspace()} on every {@code Refreshable} module after a load completes.
 *
 * <p>Implementations should be cheap and side-effect-free beyond updating their own view (e.g. repopulate
 * a sample combo); they must not auto-run heavy analyses.
 */
public interface Refreshable {
    void refreshFromWorkspace();
}
