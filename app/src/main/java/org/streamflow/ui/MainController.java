package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.application.Platform;
import javafx.collections.FXCollections;
import javafx.concurrent.Task;
import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.scene.Node;
import javafx.scene.control.Button;
import javafx.scene.control.CheckMenuItem;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.ProgressBar;
import javafx.scene.layout.StackPane;
import javafx.stage.FileChooser;

import org.streamflow.bridge.BridgeService;
import org.streamflow.bridge.RJobException;

import java.io.File;
import java.io.IOException;
import java.io.UncheckedIOException;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Application shell: left-nav module switching, floating toast/job-chip notifications, the File
 * menu (workspace save/open), and the {@link JobRunner} that all modules use to
 * run engine commands with shared progress + cancel. Module views are loaded
 * from FXML and handed an {@link AppContext} once the engine is ready.
 */
public class MainController implements JobRunner {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private ListView<String> navList;
    @FXML private StackPane contentPane;
    @FXML private StackPane moduleHost;
    @FXML private ProgressBar globalProgressBar;
    @FXML private CheckMenuItem autoSaveMenuItem;

    // ---- collapsible sidebar (global — same behaviour from every tab, since this whole region
    // lives outside contentPane) ----
    @FXML private javafx.scene.layout.VBox sidebarBox;
    @FXML private javafx.scene.layout.VBox sidebarContent;
    @FXML private javafx.scene.layout.VBox sidebarRibbon;
    @FXML private Button sidebarToggleButton;
    @FXML private Button ribbonExpandButton;
    private boolean sidebarCollapsed = false;
    // Collapsed strip is wide enough that the expand arrow sits fully inside it with margin on both
    // sides, so it reads as a slim clickable edge tab rather than a button clipped by too-narrow a strip.
    private static final double SIDEBAR_EXPANDED_W = 220, SIDEBAR_COLLAPSED_W = 32;

    // Floating overlay UI (replaces the old persistent bottom status bar): toasts for one-off
    // messages, and a job chip (spin icon + cancel) that only exists while a task is running.
    // Overall progress shows on the thin globalProgressBar strip instead of inside the chip.
    private javafx.scene.layout.HBox jobChip;
    private Button chipCancelButton;
    private javafx.animation.RotateTransition jobChipSpin;

    // module name -> view node and controller
    private final Map<String, Node> views = new LinkedHashMap<>();
    private final Map<String, Object> controllers = new LinkedHashMap<>();
    private Node placeholder;

    private BridgeService bridge;
    private SetupController setupController;
    private AppSettings settings;
    private WorkspaceModel workspace;
    private AppContext appCtx;
    private Task<?> currentTask;

    // ---- autosave / unsaved-changes state -----------------------------------
    private File lastWorkspaceFile;            // where autosave writes (null until first save/open)
    private boolean autoSaveEnabled = false;   // starts OFF; auto-enables after the first manual save
    private long sessionStartMs;               // for the 10/30-minute nudges
    private long lastAutoSaveMs;               // throttle silent saves to ~every 7 min
    private boolean asked10, asked30;          // each nudge fires at most once per experiment

    @FXML
    public void initialize() {
        placeholder = buildPlaceholder("Select a module. Real modules arrive per phase.");
        loadModule("Setup", "/org/streamflow/ui/setup.fxml");
        loadModule("Workstation", "/org/streamflow/ui/workstation.fxml");
        loadModule("Compensation", "/org/streamflow/ui/compensation.fxml");
        loadModule("Transformation", "/org/streamflow/ui/transformation.fxml");
        loadModule("Visualization", "/org/streamflow/ui/visualization.fxml");
        loadModule("Dim. Reduction", "/org/streamflow/ui/dimreduction.fxml");
        loadModule("Clustering", "/org/streamflow/ui/clustering.fxml");
        loadModule("Statistics", "/org/streamflow/ui/statistics.fxml");
        loadModule("Cell Cycle", "/org/streamflow/ui/cell-cycle.fxml");
        loadModule("Proliferation", "/org/streamflow/ui/proliferation.fxml");
        loadModule("Apoptosis", "/org/streamflow/ui/apoptosis.fxml");
        loadModule("Stats Comparison", "/org/streamflow/ui/stat-comparison.fxml");
        loadModule("Kinetic", "/org/streamflow/ui/kinetic.fxml");
        loadModule("Classifier", "/org/streamflow/ui/classifier.fxml");
        loadModule("Cross-Sample", "/org/streamflow/ui/cross-sample.fxml");
        loadModule("Longitudinal", "/org/streamflow/ui/longitudinal.fxml");
        loadModule("3D Scatter", "/org/streamflow/ui/scatter3d.fxml");
        loadModule("Export", "/org/streamflow/ui/export.fxml");
        loadModule("Analysis Log", "/org/streamflow/ui/analysis-log.fxml");
        loadModule("Developer / Engine", "/org/streamflow/ui/devconsole.fxml");
        setupController = (SetupController) controllers.get("Setup");

        // Setup is merged into Workstation (the home); FCS import lives in File ▸ Load FCS….
        // Visualization (server-side matplotlib) is redundant with the native Graph Window.
        var names = FXCollections.observableArrayList(
                "Workstation", "Compensation", "Transformation",
                "Dim. Reduction", "Clustering", "Statistics",
                "Cell Cycle", "Proliferation", "Apoptosis", "Stats Comparison",
                "Kinetic", "Classifier", "Cross-Sample", "Longitudinal", "3D Scatter",
                "Export", "Analysis Log", "Developer / Engine");
        navList.setItems(names);
        navList.getSelectionModel().selectedItemProperty().addListener((o, prev, sel) -> showModule(sel));
        navList.getSelectionModel().select("Workstation");

        // AutoSave starts ON by default (per user preference) — the menu item's FXML selected="true"
        // matches this so no spurious onAutoSaveChanged fires here (no value change occurs).
        autoSaveMenuItem.setSelected(true);
        autoSaveEnabled = true;
        autoSaveMenuItem.selectedProperty().addListener((o, was, on) -> onAutoSaveChanged(on));

        setupSidebarToggle();
    }

    /** Global collapsible-sidebar toggle (works identically from every tab, since {@code sidebarBox}
     *  lives outside {@code contentPane}). Collapsing shrinks the sidebar to a thin edge ribbon so the
     *  active tab can use the full window width; the ribbon re-expands it. */
    private void setupSidebarToggle() {
        iconOnlyButton(sidebarToggleButton, "fas-angle-left");
        iconOnlyButton(ribbonExpandButton, "fas-angle-right");
        UiFx.hoverPulse(sidebarToggleButton);
        UiFx.hoverPulse(ribbonExpandButton);
        sidebarToggleButton.setOnAction(e -> collapseSidebar());
        ribbonExpandButton.setOnAction(e -> expandSidebar());
        // The whole collapsed strip is clickable (it lights up on hover), not just the arrow glyph —
        // a bigger, more forgiving target that matches the "click the edge tab to expand" affordance.
        sidebarRibbon.setOnMouseClicked(e -> expandSidebar());
    }

    private static void iconOnlyButton(Button b, String iconLiteral) {
        org.kordamp.ikonli.javafx.FontIcon fi = new org.kordamp.ikonli.javafx.FontIcon(iconLiteral);
        fi.setIconSize(14);
        fi.setIconColor(javafx.scene.paint.Color.web("#CFE3F2"));
        b.setGraphic(fi);
    }

    /** Animate the sidebar down to a thin ribbon: fade the nav content out, shrink the width, then
     *  swap in the ribbon tab (faded in). */
    private void collapseSidebar() {
        if (sidebarCollapsed) return;
        sidebarCollapsed = true;
        javafx.animation.FadeTransition fadeOutContent =
                new javafx.animation.FadeTransition(javafx.util.Duration.millis(120), sidebarContent);
        fadeOutContent.setFromValue(1); fadeOutContent.setToValue(0);
        fadeOutContent.setOnFinished(e1 -> {
            sidebarContent.setVisible(false); sidebarContent.setManaged(false);
            javafx.animation.Timeline shrink = new javafx.animation.Timeline(
                    new javafx.animation.KeyFrame(javafx.util.Duration.millis(200),
                            new javafx.animation.KeyValue(sidebarBox.prefWidthProperty(), SIDEBAR_COLLAPSED_W, javafx.animation.Interpolator.EASE_BOTH),
                            new javafx.animation.KeyValue(sidebarBox.minWidthProperty(), SIDEBAR_COLLAPSED_W, javafx.animation.Interpolator.EASE_BOTH),
                            new javafx.animation.KeyValue(sidebarBox.maxWidthProperty(), SIDEBAR_COLLAPSED_W, javafx.animation.Interpolator.EASE_BOTH)));
            shrink.setOnFinished(e2 -> {
                sidebarRibbon.setVisible(true); sidebarRibbon.setManaged(true);
                sidebarRibbon.setOpacity(0);
                javafx.animation.FadeTransition fadeInRibbon =
                        new javafx.animation.FadeTransition(javafx.util.Duration.millis(120), sidebarRibbon);
                fadeInRibbon.setFromValue(0); fadeInRibbon.setToValue(1);
                fadeInRibbon.play();
            });
            shrink.play();
        });
        fadeOutContent.play();
    }

    /** Reverse of {@link #collapseSidebar()}: fade the ribbon out, grow the width back, then fade the
     *  nav content back in. */
    private void expandSidebar() {
        if (!sidebarCollapsed) return;
        sidebarCollapsed = false;
        javafx.animation.FadeTransition fadeOutRibbon =
                new javafx.animation.FadeTransition(javafx.util.Duration.millis(100), sidebarRibbon);
        fadeOutRibbon.setFromValue(1); fadeOutRibbon.setToValue(0);
        fadeOutRibbon.setOnFinished(e1 -> {
            sidebarRibbon.setVisible(false); sidebarRibbon.setManaged(false);
            javafx.animation.Timeline grow = new javafx.animation.Timeline(
                    new javafx.animation.KeyFrame(javafx.util.Duration.millis(200),
                            new javafx.animation.KeyValue(sidebarBox.prefWidthProperty(), SIDEBAR_EXPANDED_W, javafx.animation.Interpolator.EASE_BOTH),
                            new javafx.animation.KeyValue(sidebarBox.minWidthProperty(), SIDEBAR_EXPANDED_W, javafx.animation.Interpolator.EASE_BOTH),
                            new javafx.animation.KeyValue(sidebarBox.maxWidthProperty(), SIDEBAR_EXPANDED_W, javafx.animation.Interpolator.EASE_BOTH)));
            grow.setOnFinished(e2 -> {
                sidebarContent.setVisible(true); sidebarContent.setManaged(true);
                sidebarContent.setOpacity(0);
                javafx.animation.FadeTransition fadeInContent =
                        new javafx.animation.FadeTransition(javafx.util.Duration.millis(140), sidebarContent);
                fadeInContent.setFromValue(0); fadeInContent.setToValue(1);
                fadeInContent.play();
            });
            grow.play();
        });
        fadeOutRibbon.play();
    }

    /** Lazily build the floating job-cancel chip (shown only while a task is running). Compact —
     *  no internal ProgressBar; overall progress now shows as the extremely thin {@code
     *  globalProgressBar} strip at the very bottom of the window instead. */
    private void ensureJobChip() {
        if (jobChip != null) return;
        chipCancelButton = new Button("Cancel");
        chipCancelButton.setStyle("-fx-font-size: 11px; -fx-padding: 2 8 2 8;");
        chipCancelButton.setOnAction(e -> { if (currentTask != null && currentTask.isRunning()) currentTask.cancel(); });
        org.kordamp.ikonli.javafx.FontIcon spinnerIcon = new org.kordamp.ikonli.javafx.FontIcon("fas-sync-alt");
        spinnerIcon.setIconSize(12);
        spinnerIcon.setIconColor(javafx.scene.paint.Color.web("#8FD3FF"));
        jobChipSpin = UiFx.spin(spinnerIcon);
        Label working = new Label("Working…");
        working.setStyle("-fx-text-fill: white; -fx-font-size: 11px;");
        jobChip = new javafx.scene.layout.HBox(6, spinnerIcon, working, chipCancelButton);
        jobChip.setStyle("-fx-background-color: rgba(20,30,45,0.92); -fx-padding: 4 10 4 10; -fx-background-radius: 5;");
        jobChip.setAlignment(javafx.geometry.Pos.CENTER_LEFT);
        StackPane.setAlignment(jobChip, javafx.geometry.Pos.BOTTOM_LEFT);
        StackPane.setMargin(jobChip, new javafx.geometry.Insets(0, 0, 16, 16));
    }

    /** Show/hide the floating job chip AND the thin global progress strip — the only trace of
     *  "something is running" now that the persistent status bar is gone. Bound to the engine-wide
     *  busy signal, not any single task. */
    private void setJobChipVisible(boolean visible) {
        ensureJobChip();
        if (visible) {
            if (!contentPane.getChildren().contains(jobChip)) contentPane.getChildren().add(jobChip);
            jobChipSpin.play();
        } else {
            contentPane.getChildren().remove(jobChip);
            jobChipSpin.pause();
        }
        globalProgressBar.setVisible(visible);
        globalProgressBar.setManaged(visible);
    }

    private void loadModule(String name, String fxml) {
        try {
            FXMLLoader loader = new FXMLLoader(getClass().getResource(fxml));
            Node node = loader.load();
            views.put(name, node);
            controllers.put(name, loader.getController());
        } catch (IOException e) {
            throw new UncheckedIOException("Failed to load module FXML: " + fxml, e);
        }
    }

    private void showModule(String name) {
        Node node = (name == null) ? null : views.get(name);
        // Swap ONLY the module host's child — contentPane's other children (toasts, job chip) persist.
        moduleHost.getChildren().setAll(node != null ? node : placeholder);
    }

    /** Called once the engine is ready; distributes the context to all modules. */
    public void bindBridge(BridgeService bridge) {
        this.bridge = bridge;
        this.settings = new AppSettings();
        this.workspace = new WorkspaceModel();
        AppContext ctx = new AppContext(bridge, this, new ChannelAliases(), workspace, settings, new AuditLog(), new FmoStore(),
                name -> Platform.runLater(() -> navList.getSelectionModel().select(name)));
        this.appCtx = ctx;
        // Compensation applied is now a one-time toast instead of a persistent badge.
        workspace.compApplied().addListener((o, was, applied) -> { if (applied) toast("Compensation applied.", "#2E7D32"); });
        for (Object c : controllers.values()) {
            if (c instanceof ContextAware ca) ca.init(ctx);
        }
        bridge.busyProperty().addListener((o, was, busy) -> setJobChipVisible(busy));
        startAutoSave();
    }

    // ---- autosave -----------------------------------------------------------

    /** Tick every minute: silent-save to a known file (~every 7 min), else nudge to save (10/30 min). */
    private void startAutoSave() {
        sessionStartMs = System.currentTimeMillis();
        javafx.animation.Timeline t = new javafx.animation.Timeline(
                new javafx.animation.KeyFrame(javafx.util.Duration.minutes(1), e -> autoSaveTick()));
        t.setCycleCount(javafx.animation.Animation.INDEFINITE);
        t.play();
    }

    private void autoSaveTick() {
        if (!autoSaveEnabled || workspace == null || !workspace.isDirty() || bridge == null) return;
        long now = System.currentTimeMillis();
        if (lastWorkspaceFile != null) {                 // we know where to write → save silently
            if (now - lastAutoSaveMs >= 7 * 60_000L) saveWorkspaceTo(lastWorkspaceFile, true, null);
            return;
        }
        // never saved this experiment — gently nudge the user to choose a file
        long elapsedMin = (now - sessionStartMs) / 60_000L;
        if (!asked10 && elapsedMin >= 10) {
            asked10 = true;
            nudgeSave("Quick one 👀", "It's been 10 minutes and you've got unsaved gates.\nWant to save this workspace?");
        } else if (!asked30 && elapsedMin >= 30) {
            asked30 = true;
            nudgeSave("Still there? 🙂", "30 minutes in and nothing's been saved yet —\ndid you forget to save your work?");
        }
    }

    private void nudgeSave(String header, String msg) {
        javafx.scene.control.Alert a = new javafx.scene.control.Alert(
                javafx.scene.control.Alert.AlertType.CONFIRMATION, msg,
                javafx.scene.control.ButtonType.YES, javafx.scene.control.ButtonType.NO);
        a.setTitle("Save workspace?"); a.setHeaderText(header);
        AppIcons.theme(a, window());
        if (a.showAndWait().orElse(javafx.scene.control.ButtonType.NO) == javafx.scene.control.ButtonType.YES) {
            onSaveWorkspace();
        }
    }

    /** Auto-save switch flipped. ON: if the workspace was never saved, ask for a file now (so silent
     *  autosave has a target); if it already has a file, the timer saves it at intervals. OFF: stop. */
    private void onAutoSaveChanged(boolean on) {
        autoSaveEnabled = on;
        if (!on) { status("Auto-save off."); return; }
        boolean hasData = workspace != null && !workspace.sampleNames().isEmpty();
        if (lastWorkspaceFile == null && hasData) {
            status("Auto-save on — choose where to save this workspace.");
            javafx.scene.control.Alert a = new javafx.scene.control.Alert(
                    javafx.scene.control.Alert.AlertType.CONFIRMATION,
                    "Auto-save needs a file to write to. Save this workspace now?",
                    javafx.scene.control.ButtonType.YES, javafx.scene.control.ButtonType.NO);
            a.setTitle("Auto-save"); a.setHeaderText("Save the workspace?");
            AppIcons.theme(a, window());
            if (a.showAndWait().orElse(javafx.scene.control.ButtonType.NO) == javafx.scene.control.ButtonType.YES) {
                onSaveWorkspace();
            }
        } else {
            status(lastWorkspaceFile != null
                    ? "Auto-save on — saving to " + lastWorkspaceFile.getName() + " every few minutes."
                    : "Auto-save on.");
        }
    }

    /** Window close / File ▸ Exit: clean → quit silently; unsaved → offer Save / Don't save / Cancel. */
    public void confirmCloseAndExit(javafx.stage.WindowEvent ev) {
        if (workspace == null || !workspace.isDirty()) { Platform.exit(); return; }   // nothing unsaved
        javafx.scene.control.ButtonType save = new javafx.scene.control.ButtonType("Save");
        javafx.scene.control.ButtonType discard = new javafx.scene.control.ButtonType("Don't save");
        javafx.scene.control.Alert a = new javafx.scene.control.Alert(
                javafx.scene.control.Alert.AlertType.CONFIRMATION,
                "You have unsaved gating changes.",
                save, discard, javafx.scene.control.ButtonType.CANCEL);
        a.setTitle("Close StreamFLOW"); a.setHeaderText("Save before closing?");
        AppIcons.theme(a, window());
        javafx.scene.control.ButtonType r = a.showAndWait().orElse(javafx.scene.control.ButtonType.CANCEL);
        if (r == javafx.scene.control.ButtonType.CANCEL) { if (ev != null) ev.consume(); return; }
        if (r == discard) { Platform.exit(); return; }
        // Save: write then exit. Known file → silent save then quit; otherwise let the user pick a path.
        if (ev != null) ev.consume();   // don't close yet; exit only after the save completes
        if (lastWorkspaceFile != null) saveWorkspaceTo(lastWorkspaceFile, true, Platform::exit);
        else onSaveWorkspace();         // user picks a file; they can close again afterwards
    }

    public void setEngineStatus(String text) {
        Platform.runLater(() -> toast(text, "#243447"));
    }

    // ---- JobRunner ----------------------------------------------------------

    @Override
    public <T> void run(Task<T> task, Consumer<T> onSuccess) {
        currentTask = task;
        ensureJobChip();
        // Never let the bar go indeterminate (-1): the animated indeterminate
        // ProgressBar forces continuous scene repaints (the whole-window "shimmer").
        // Show determinate progress when a task reports it, else a static 0.
        globalProgressBar.progressProperty().unbind();
        globalProgressBar.setProgress(0);
        task.progressProperty().addListener((o, ov, nv) -> {
            double p = nv.doubleValue();
            globalProgressBar.setProgress(p < 0 ? 0 : p);
        });

        task.setOnSucceeded(e -> {
            resetProgress();
            if (onSuccess != null) onSuccess.accept(task.getValue());
        });
        task.setOnFailed(e -> {
            resetProgress();
            Throwable ex = task.getException();
            if (ex instanceof RJobException rje) {
                status("ERROR: " + rje.getMessage());
            } else {
                status("FAILED: " + (ex == null ? "unknown error" : ex.toString()));
            }
        });
        task.setOnCancelled(e -> { resetProgress(); status("Cancelled."); });

        Thread t = new Thread(task, "job");
        t.setDaemon(true);
        t.start();
    }

    @Override
    public void status(String message) {
        if (message == null || message.isBlank()) return;
        boolean error = message.startsWith("ERROR") || message.startsWith("FAILED");
        if (Platform.isFxApplicationThread()) toast(message, error ? "#B00020" : "#243447");
        else Platform.runLater(() -> toast(message, error ? "#B00020" : "#243447"));
    }

    private void resetProgress() {
        if (globalProgressBar != null) globalProgressBar.setProgress(0);
    }

    // Bottom-right stack of active toasts, so simultaneous notifications (e.g. a save confirmation
    // and an error) appear one above the other instead of exactly overlapping. Lazily created.
    private javafx.scene.layout.VBox toastStack;

    private void ensureToastStack() {
        if (toastStack != null) return;
        toastStack = new javafx.scene.layout.VBox(6);
        toastStack.setAlignment(javafx.geometry.Pos.BOTTOM_RIGHT);
        toastStack.setPickOnBounds(false);   // empty gaps between toasts never intercept clicks
        StackPane.setAlignment(toastStack, javafx.geometry.Pos.BOTTOM_RIGHT);
        StackPane.setMargin(toastStack, new javafx.geometry.Insets(0, 20, 20, 0));
        contentPane.getChildren().add(toastStack);
    }

    /** Floating, auto-dismissing notification — replaces the removed persistent status bar for
     *  one-off messages (save/load confirmations, errors, engine status). Bottom-right, ~3.5s total.
     *  Stacks with any other currently-visible toasts instead of overlapping them. */
    private void toast(String message, String colorHex) {
        ensureToastStack();
        Label t = new Label(message);
        t.setStyle(
                "-fx-background-color: " + colorHex + ";" +
                "-fx-text-fill: white;" +
                "-fx-padding: 6 14 6 14;" +
                "-fx-background-radius: 6;" +
                "-fx-font-size: 12px;");
        toastStack.getChildren().add(t);
        javafx.animation.FadeTransition fade = new javafx.animation.FadeTransition(
                javafx.util.Duration.seconds(2.0), t);
        fade.setFromValue(1.0);
        fade.setToValue(0.0);
        fade.setDelay(javafx.util.Duration.seconds(1.5));
        fade.setOnFinished(e -> toastStack.getChildren().remove(t));
        fade.play();
    }

    // ---- File menu: workspace ----------------------------------------------

    @FXML
    private void onOpenWorkspace() {
        if (bridge == null) { status("Engine not ready."); return; }
        // Guard: if the current experiment has unsaved changes, offer to save it first.
        if (workspace != null && workspace.isDirty()) {
            javafx.scene.control.ButtonType save  = new javafx.scene.control.ButtonType("Save");
            javafx.scene.control.ButtonType discard = new javafx.scene.control.ButtonType("Don't save");
            javafx.scene.control.Alert ask = new javafx.scene.control.Alert(
                    javafx.scene.control.Alert.AlertType.CONFIRMATION,
                    "You have unsaved gating changes. Save before opening another workspace?",
                    save, discard, javafx.scene.control.ButtonType.CANCEL);
            ask.setTitle("Unsaved changes"); ask.setHeaderText("Save current workspace?");
            AppIcons.theme(ask, window());
            javafx.scene.control.ButtonType r = ask.showAndWait().orElse(javafx.scene.control.ButtonType.CANCEL);
            if (r == javafx.scene.control.ButtonType.CANCEL) return;
            if (r == save) {
                smartSave();   // overwrite existing file or show Save As for untitled
                return;        // user re-tries Open Workspace after save completes
            }
        }
        doOpenWorkspace();
    }

    private void doOpenWorkspace() {
        FileChooser fc = workspaceChooser("Open workspace");
        File f = fc.showOpenDialog(window());
        if (f == null) return;
        purgeWorkspaceState();
        ObjectNode args = JSON.createObjectNode();
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        status("Opening " + f.getName() + "…");
        run(bridge.command("load_workspace", args), summary -> {
            if (setupController != null) setupController.populate(summary);
            restoreGates(summary.path("gates"));
            if (workspace != null) {
                workspace.seedChannelScalesFromTrees();       // BUG-11: restore per-marker scales
                workspace.seedPopLabelOffsetsFromTrees();     // BUG-14: restore per-population label positions
            }
            if (appCtx != null) appCtx.auditLog().restore(summary.path("audit_log"));
            refreshModules();
            lastWorkspaceFile = f;
            if (workspace != null) workspace.markClean();
            if (!autoSaveEnabled) { autoSaveEnabled = true; autoSaveMenuItem.setSelected(true); }
            asked10 = asked30 = false; sessionStartMs = System.currentTimeMillis();
            navList.getSelectionModel().select("Workstation");
            status("Workspace loaded: " + f.getName());
        });
    }

    /** File ▸ New Workspace — saves current if dirty, then purges all state for a clean slate. */
    @FXML
    private void onNewWorkspace() {
        if (workspace != null && workspace.isDirty()) {
            javafx.scene.control.ButtonType save    = new javafx.scene.control.ButtonType("Save");
            javafx.scene.control.ButtonType discard = new javafx.scene.control.ButtonType("Don't save");
            javafx.scene.control.Alert ask = new javafx.scene.control.Alert(
                    javafx.scene.control.Alert.AlertType.CONFIRMATION,
                    "You have unsaved gating changes.",
                    save, discard, javafx.scene.control.ButtonType.CANCEL);
            ask.setTitle("New Workspace"); ask.setHeaderText("Save before starting fresh?");
            AppIcons.theme(ask, window());
            javafx.scene.control.ButtonType r = ask.showAndWait().orElse(javafx.scene.control.ButtonType.CANCEL);
            if (r == javafx.scene.control.ButtonType.CANCEL) return;
            if (r == save) { smartSave(); return; }   // user starts New Workspace again after save
        }
        purgeWorkspaceState();
        status("New workspace — load FCS files to begin.");
        navList.getSelectionModel().select("Workstation");
    }

    private File lastFcsDir;   // #20 remember the last folder across loads

    /** File ▸ Load FCS — appends data to the current workspace; never prompts to save or clears state. */
    @FXML
    private void onLoadFcs() {
        if (bridge == null) { status("Engine not ready."); return; }
        chooseAndLoadFcs();
    }

    private void chooseAndLoadFcs() {
        javafx.stage.DirectoryChooser dc = new javafx.stage.DirectoryChooser();
        dc.setTitle("Select FCS folder");
        if (lastFcsDir != null && lastFcsDir.isDirectory()) dc.setInitialDirectory(lastFcsDir);
        File dir = dc.showDialog(window());
        if (dir == null) return;
        lastFcsDir = dir;
        javafx.scene.control.Alert ask = new javafx.scene.control.Alert(
                javafx.scene.control.Alert.AlertType.CONFIRMATION,
                "Search sub-folders for .fcs files too?",
                javafx.scene.control.ButtonType.YES, javafx.scene.control.ButtonType.NO);
        ask.setHeaderText("Include sub-folders?");
        AppIcons.theme(ask, window());
        boolean recursive = ask.showAndWait().orElse(javafx.scene.control.ButtonType.NO)
                == javafx.scene.control.ButtonType.YES;
        ObjectNode args = JSON.createObjectNode();
        args.put("folder", dir.getAbsolutePath().replace('\\', '/'));
        args.put("recursive", recursive);
        status("Loading FCS from " + dir.getName() + "…");
        run(bridge.command("load_fcs", args), summary -> {
            if (setupController != null) setupController.populate(summary);   // appends new samples to workspace
            refreshModules();
            if (workspace != null) workspace.markDirty();   // new data added → unsaved changes
            asked10 = asked30 = false; sessionStartMs = System.currentTimeMillis();
            navList.getSelectionModel().select("Workstation");
            status("Loaded FCS from " + dir.getName() + ".");
        });
    }

    /** Wipe all experiment data: trees, events, gates, FMO, autosave target. */
    private void purgeWorkspaceState() {
        if (workspace != null) workspace.clearAll();
        if (appCtx != null) appCtx.fmo().clearAll();
        lastWorkspaceFile = null;
        // Auto-save follows the Settings ▸ General default (ON unless the user changed it this session).
        autoSaveEnabled = settings == null || settings.defaultAutoSave();
        autoSaveMenuItem.setSelected(autoSaveEnabled);
        asked10 = asked30 = false;
        sessionStartMs = System.currentTimeMillis();
        refreshModules();   // update all UI panels to reflect the now-empty workspace
    }

    /** #31 — re-run each module's workspace-driven refresh after data loads (drops manual Refresh). */
    private void refreshModules() {
        for (Object c : controllers.values()) {
            if (c instanceof Refreshable r) {
                try { r.refreshFromWorkspace(); } catch (Exception ignored) {}
            }
        }
    }

    /** File ▸ Save Workspace — overwrites the existing file if known; otherwise shows Save As dialog. */
    @FXML
    private void onSaveWorkspace() {
        if (bridge == null) { status("Engine not ready."); return; }
        smartSave();
    }

    /** File ▸ Save Workspace As — always shows the Save dialog. */
    @FXML
    private void onSaveWorkspaceAs() {
        if (bridge == null) { status("Engine not ready."); return; }
        FileChooser fc = workspaceChooser("Save workspace as");
        fc.setInitialFileName(lastWorkspaceFile != null ? lastWorkspaceFile.getName() : "experiment.sfw");
        File f = fc.showSaveDialog(window());
        if (f == null) return;
        saveWorkspaceTo(f, false, null);
    }

    /** If we already know where to save, overwrite silently; otherwise show the Save As dialog. */
    private void smartSave() {
        if (lastWorkspaceFile != null) {
            saveWorkspaceTo(lastWorkspaceFile, false, null);
        } else {
            onSaveWorkspaceAs();
        }
    }

    /** Write the workspace to {@code f}. {@code silent} suppresses the verbose status (used by autosave).
     *  Runs {@code onDone} on the FX thread after a successful save. */
    private void saveWorkspaceTo(File f, boolean silent, Runnable onDone) {
        if (bridge == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        args.set("gates", serializeGates());   // gates live on the Java side; ship them to the .sfw
        if (appCtx != null) args.set("audit_log", appCtx.auditLog().toJson(JSON));   // #32b persist the log
        if (!silent) status("Saving " + f.getName() + "…");
        run(bridge.command("save_workspace", args), r -> {
            lastWorkspaceFile = f;
            lastAutoSaveMs = System.currentTimeMillis();
            if (workspace != null) workspace.markClean();
            if (silent) {
                showAutoSaveToast(f.getName());
            } else {
                status("Workspace saved: " + f.getName()
                        + " (" + r.path("n_gates").asInt() + " gate(s)).");
                // First manual save → turn autosave on automatically
                if (!autoSaveEnabled) {
                    autoSaveEnabled = true;
                    autoSaveMenuItem.setSelected(true);
                }
            }
            if (onDone != null) onDone.run();
        });
    }

    /** Floating toast shown after a silent auto-save. */
    private void showAutoSaveToast(String filename) {
        toast("✓  Auto-saved · " + filename, "#2E7D32");
    }

    // ---- workspace gate (de)serialization -----------------------------------

    /** Serialise every sample's gating tree to {sample: [ {id,parent_id,name,type,channels,xs,ys,angle} ]}. */
    private ObjectNode serializeGates() {
        ObjectNode out = JSON.createObjectNode();
        if (workspace == null) return out;
        for (String sample : workspace.samples()) {
            PopNode root = workspace.treeFor(sample);
            ArrayNode arr = JSON.createArrayNode();
            Map<PopNode, String> ids = new LinkedHashMap<>();
            int seq = 0;
            for (PopNode n : root.selfAndDescendants()) if (!n.isRoot()) ids.put(n, "G" + (++seq));
            for (PopNode n : root.selfAndDescendants()) {
                if (n.isRoot()) continue;
                CytoPlot.Gate g = n.gate;
                ObjectNode gn = JSON.createObjectNode();
                gn.put("id", ids.get(n));
                if (n.parent != null && !n.parent.isRoot()) gn.put("parent_id", ids.get(n.parent));
                gn.put("name", g.name);
                gn.put("type", g.type);
                if ("subsample".equals(g.type)) { gn.put("sub_n", g.subN); gn.put("sub_seed", g.subSeed); }
                gn.put("x_channel", g.xChan);
                if (g.yChan != null && !g.yChan.isBlank()) gn.put("y_channel", g.yChan);
                gn.put("angle", g.angle);
                ArrayNode xs = gn.putArray("xs");
                if (g.xs != null) for (double x : g.xs) xs.add(x);
                ArrayNode ys = gn.putArray("ys");
                if (g.ys != null) for (double y : g.ys) ys.add(y);
                // Persist the per-node view (axes + scales) so a saved workspace reopens on the same
                // axes/scale instead of reverting to the FSC/SSC Linear default. See ui-bug-log BUG-09.
                if (n.viewX != null)      gn.put("view_x", n.viewX);
                if (n.viewY != null)      gn.put("view_y", n.viewY);
                if (n.viewXScale != null) gn.put("view_x_scale", n.viewXScale);
                if (n.viewYScale != null) gn.put("view_y_scale", n.viewYScale);
                if (g.lblDx != 0 || g.lblDy != -4) { gn.put("lbl_dx", g.lblDx); gn.put("lbl_dy", g.lblDy); }
                // Persist computed counts so the Workstation tree shows %/counts for EVERY sample on
                // reopen without needing to open each one (BUG-16).
                if (n.count >= 0) gn.put("count", n.count);
                if (!Double.isNaN(n.parentPct)) gn.put("parent_pct", n.parentPct);
                arr.add(gn);
            }
            if (!arr.isEmpty()) out.set(sample, arr);
        }
        return out;
    }

    /** Rebuild the gating trees from a loaded .sfw "gates" node into the workspace. */
    private void restoreGates(JsonNode gates) {
        if (workspace == null || gates == null || gates.isMissingNode() || !gates.isObject()) return;
        gates.fieldNames().forEachRemaining(sample -> {
            JsonNode arr = gates.get(sample);
            PopNode root = new PopNode(null, null);
            Map<String, PopNode> byId = new HashMap<>();
            for (JsonNode gn : arr) {                       // pass 1: create nodes
                double[] xs = toArray(gn.path("xs"));
                JsonNode ysN = gn.path("ys");
                double[] ys = (ysN.isArray() && ysN.size() > 0) ? toArray(ysN) : null;
                CytoPlot.Gate g = new CytoPlot.Gate(
                        gn.path("name").asText(), gn.path("type").asText(),
                        gn.path("x_channel").asText(null),
                        gn.path("y_channel").asText(null), xs, ys);
                g.angle = gn.path("angle").asDouble(0);
                if ("subsample".equals(g.type)) { g.subN = gn.path("sub_n").asInt(-1); g.subSeed = gn.path("sub_seed").asLong(0); }
                PopNode pn = new PopNode(g, null);
                // Restore the per-node view (axes + scales) saved by serializeGates. See ui-bug-log BUG-09.
                pn.viewX      = gn.path("view_x").asText(null);
                pn.viewY      = gn.path("view_y").asText(null);
                pn.viewXScale = gn.path("view_x_scale").asText(null);
                pn.viewYScale = gn.path("view_y_scale").asText(null);
                if (gn.has("lbl_dx")) g.lblDx = gn.path("lbl_dx").asDouble(0);
                if (gn.has("lbl_dy")) g.lblDy = gn.path("lbl_dy").asDouble(-4);
                pn.count = gn.path("count").asInt(-1);                      // BUG-16: restore saved counts
                if (gn.has("parent_pct")) pn.parentPct = gn.path("parent_pct").asDouble();
                byId.put(gn.path("id").asText(), pn);
            }
            for (JsonNode gn : arr) {                       // pass 2: link parents
                PopNode node = byId.get(gn.path("id").asText());
                String pid = gn.path("parent_id").asText(null);
                PopNode parent = (pid != null) ? byId.getOrDefault(pid, root) : root;
                node.parent = parent;
                parent.children.add(node);
            }
            dedupSiblingsByName(root);   // BUG-15 auto-heal: drop duplicate same-named gates from old saves
            workspace.replaceTree(sample, root);
        });
        workspace.notifyTreeChanged();
    }

    /** Remove duplicate same-named sibling gates (keeps the first). Gates are auto-named uniquely, so
     *  same-named siblings only arise from the old apply-to-all duplication bug (BUG-15). */
    private static void dedupSiblingsByName(PopNode parent) {
        java.util.Set<String> seen = new java.util.HashSet<>();
        parent.children.removeIf(c -> c.name() != null && !seen.add(c.name()));
        for (PopNode c : parent.children) dedupSiblingsByName(c);
    }

    private static double[] toArray(JsonNode arr) {
        if (arr == null || !arr.isArray()) return new double[0];
        double[] a = new double[arr.size()];
        for (int i = 0; i < arr.size(); i++) a[i] = arr.get(i).asDouble();
        return a;
    }

    @FXML
    private void onExit() {
        confirmCloseAndExit(null);
    }

    @FXML
    private void onSettings() {
        if (settings == null) { status("Engine not ready."); return; }
        SettingsController.open(settings);
    }

    // ---- help + about -------------------------------------------------------

    @FXML private void onUserGuide()        { showHelpWindow("StreamFLOW — User Guide", USER_GUIDE); }
    @FXML private void onCompensationHelp() { showHelpWindow("StreamFLOW — Compensation Help", COMPENSATION_HELP); }

    @FXML
    private void onAbout() {
        javafx.scene.layout.VBox box = new javafx.scene.layout.VBox(12);
        box.setAlignment(javafx.geometry.Pos.CENTER);
        box.setStyle("-fx-padding:28; -fx-background-color:#0D1B2A;");
        javafx.scene.image.Image logo = AppIcons.logo();
        if (logo != null) {
            javafx.scene.image.ImageView iv = new javafx.scene.image.ImageView(logo);
            iv.setFitWidth(120); iv.setFitHeight(120); iv.setPreserveRatio(true);
            box.getChildren().add(iv);
        }
        Label name = new Label("StreamFLOW 2.0"); name.getStyleClass().add("title");
        Label desc = new Label("Native JavaFX flow-cytometry analysis with a Python / FlowKit engine.\n"
                + "FlowJo-class gating, compensation, and analysis — open and reproducible.");
        desc.getStyleClass().add("subtitle"); desc.setWrapText(true);
        desc.setStyle("-fx-text-alignment:center;"); desc.setMaxWidth(420);
        box.getChildren().addAll(name, desc);

        javafx.scene.control.Dialog<javafx.scene.control.ButtonType> dlg = new javafx.scene.control.Dialog<>();
        dlg.setTitle("About StreamFLOW"); dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(box);
        dlg.getDialogPane().getButtonTypes().add(javafx.scene.control.ButtonType.CLOSE);
        dlg.getDialogPane().getStylesheets().add(
                getClass().getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
        dlg.getDialogPane().getStyleClass().add("app-root");
        if (window() != null) dlg.initOwner(window());
        dlg.showAndWait();
    }

    /** A scrollable, dark-themed help window: logo header + a list of {heading, body} sections. */
    private void showHelpWindow(String title, String[][] sections) {
        javafx.scene.layout.VBox box = new javafx.scene.layout.VBox(12);
        box.getStyleClass().add("content");
        box.setStyle("-fx-padding:24;");

        javafx.scene.layout.HBox header = new javafx.scene.layout.HBox(14);
        header.setAlignment(javafx.geometry.Pos.CENTER_LEFT);
        javafx.scene.image.Image logo = AppIcons.logo();
        if (logo != null) {
            javafx.scene.image.ImageView iv = new javafx.scene.image.ImageView(logo);
            iv.setFitWidth(56); iv.setFitHeight(56); iv.setPreserveRatio(true);
            header.getChildren().add(iv);
        }
        Label t = new Label(title); t.getStyleClass().add("title");
        header.getChildren().add(t);
        box.getChildren().add(header);

        for (String[] sec : sections) {
            Label h = new Label(sec[0]);
            h.getStyleClass().add("title"); h.setStyle("-fx-font-size:15;");
            Label b = new Label(sec[1]);
            b.getStyleClass().add("subtitle"); b.setWrapText(true); b.setMaxWidth(700);
            box.getChildren().addAll(h, b);
        }

        javafx.scene.control.ScrollPane sp = new javafx.scene.control.ScrollPane(box);
        sp.setFitToWidth(true);
        javafx.scene.Scene scene = new javafx.scene.Scene(sp, 780, 660);
        scene.getStylesheets().add(
                getClass().getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
        javafx.stage.Stage stage = new javafx.stage.Stage();
        stage.setTitle(title);
        stage.setScene(scene);
        AppIcons.apply(stage);
        stage.show();
    }

    private static final String[][] USER_GUIDE = {
        {"1 · Load your data",
         "File → Load FCS… to import one or many .fcs files (they must share a panel). The sidebar "
         + "lists every analysis module; the Workstation gives a FlowJo-style multi-sample overview."},
        {"2 · Gate in a Graph Window",
         "Double-click a sample (Workstation) to open a Graph Window. Pick X/Y channels and a scale "
         + "(Linear / Log / Logicle / ArcSinh — fluorescence defaults to Logicle). Draw gates with the "
         + "tool buttons (P polygon, R rectangle, E ellipse, I interval, Q quadrant). Each gate becomes "
         + "a population in the gating tree; double-click a population to drill into it. Gates sync "
         + "across every window and are saved with the workspace."},
        {"3 · Compensation",
         "The Compensation module extracts an embedded spillover matrix, or computes one from single-"
         + "stain controls (Compute from Controls → wizard). Edit coefficients in the heatmap or table, "
         + "Preview the before/after on any channel pair, then Apply. See Help → Compensation Help."},
        {"4 · Analysis modules",
         "Cell Cycle (Watson / Dean-Jett-Fox), Proliferation (dye-dilution generations), Apoptosis "
         + "(Annexin/PI quadrants) — all interactive with adjustable fits and publication export. "
         + "Statistics, Cross-Sample, Classifier (PCA/RF), Kinetic, Longitudinal, Dim. Reduction and "
         + "Clustering read the shared workspace. Classifier and Cross-Sample are selection-based: tick "
         + "the samples to include and their events load on demand."},
        {"5 · Export",
         "Any plot's Copy button puts a high-DPI image on the clipboard (set the DPI under File → "
         + "Settings, or in the Graph Window's export-options dialog); Save SVG writes a vector file. "
         + "Analysis Log → Methods generates a journal-ready methods paragraph from your gating tree."},
        {"6 · Workspace",
         "File → Save Workspace… writes a .sfw file with every sample's gating tree, compensation and "
         + "settings; Open Workspace… restores them. GatingML export is available for interchange."},
    };

    private static final String[][] COMPENSATION_HELP = {
        {"What compensation does",
         "Fluorophores emit across multiple detectors (spillover). Compensation subtracts that spillover "
         + "using an N×N spillover matrix so each channel reflects only its own fluorophore."},
        {"Option A — Extract from FCS",
         "If the cytometer wrote a $SPILL/$SPILLOVER matrix into the file, Extract from FCS loads it "
         + "directly. Many instruments embed an identity matrix (no spillover) — if the heatmap is all "
         + "zeros off-diagonal, compute one from controls instead."},
        {"Option B — Compute from Controls (wizard)",
         "Load your single-stain controls + an unstained (universal negative). In the wizard, set each "
         + "file's Role (Unstained / Single stain / Ignore) and Detector (auto-matched by brightest "
         + "signal — override if wrong). StreamFLOW applies a size-cleanup gate (FSC/SSC), splits each "
         + "control's primary detector into negative/positive (Otsu), and computes spillover as the "
         + "median positive signal minus unstained autofluorescence, normalised so the diagonal = 1.0 "
         + "(Bagwell-Adams). Drag the dashed line on any separation histogram to adjust its split."},
        {"Do I need the size gate?",
         "Yes — a scatter (FSC/SSC) cleanup gate removes debris and dead cells whose odd autofluorescence "
         + "biases the medians, so it makes the matrix more accurate. The positive/negative split on the "
         + "arcsinh histogram is the core of the calculation; the size gate just cleans the input. "
         + "StreamFLOW does the cleanup automatically (central-percentile box) before the split."},
        {"Edit, preview, apply",
         "Edit any coefficient in the heatmap (click a cell) or the Matrix table — the two stay in sync. "
         + "Use Before/After preview on a spillover pair to see the effect of the current matrix (a tight "
         + "diagonal in the After plot means good compensation). Click Apply to compensate all samples."},
        {"Residual diagnostic",
         "After applying, run the Residual diagnostic: it correlates the compensated channels and flags "
         + "any pair with |r| > 0.2 (over- or under-compensation), outlining the offending coefficients "
         + "right on the heatmap so you know which to nudge."},
    };

    // ---- helpers ------------------------------------------------------------

    private FileChooser workspaceChooser(String title) {
        FileChooser fc = new FileChooser();
        fc.setTitle(title);
        fc.getExtensionFilters().add(
                new FileChooser.ExtensionFilter("StreamFLOW workspace (*.sfw)", "*.sfw"));
        if (lastWorkspaceFile != null && lastWorkspaceFile.getParentFile() != null
                && lastWorkspaceFile.getParentFile().isDirectory())
            fc.setInitialDirectory(lastWorkspaceFile.getParentFile());
        return fc;
    }

    private javafx.stage.Window window() {
        return contentPane.getScene() == null ? null : contentPane.getScene().getWindow();
    }

    private Node buildPlaceholder(String text) {
        Label l = new Label(text);
        l.getStyleClass().add("subtitle");
        StackPane p = new StackPane(l);
        p.getStyleClass().add("content");
        return p;
    }
}
