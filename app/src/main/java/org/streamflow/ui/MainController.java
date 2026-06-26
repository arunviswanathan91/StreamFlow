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
import java.time.LocalTime;
import java.time.format.DateTimeFormatter;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * Application shell: left-nav module switching, the shared status bar, the File
 * menu (workspace save/open), and the {@link JobRunner} that all modules use to
 * run engine commands with shared progress + cancel. Module views are loaded
 * from FXML and handed an {@link AppContext} once the engine is ready.
 */
public class MainController implements JobRunner {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final DateTimeFormatter TS = DateTimeFormatter.ofPattern("HH:mm:ss");

    @FXML private ListView<String> navList;
    @FXML private StackPane contentPane;
    @FXML private Label statusLabel;
    @FXML private Label engineLabel;
    @FXML private ProgressBar progressBar;
    @FXML private Button cancelButton;

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
                "Analysis Log", "Developer / Engine");
        navList.setItems(names);
        navList.getSelectionModel().selectedItemProperty().addListener((o, prev, sel) -> showModule(sel));
        navList.getSelectionModel().select("Workstation");

        progressBar.setProgress(0);
        cancelButton.setDisable(true);
        cancelButton.setOnAction(e -> { if (currentTask != null && currentTask.isRunning()) currentTask.cancel(); });
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
        contentPane.getChildren().setAll(node != null ? node : placeholder);
    }

    /** Called once the engine is ready; distributes the context to all modules. */
    public void bindBridge(BridgeService bridge) {
        this.bridge = bridge;
        this.settings = new AppSettings();
        this.workspace = new WorkspaceModel();
        AppContext ctx = new AppContext(bridge, this, new ChannelAliases(), workspace, settings, new AuditLog(), new FmoStore());
        this.appCtx = ctx;
        for (Object c : controllers.values()) {
            if (c instanceof ContextAware ca) ca.init(ctx);
        }
        bridge.busyProperty().addListener((o, was, busy) -> {
            cancelButton.setDisable(!busy);
            statusLabel.setText(busy ? "Working…" : "Idle");
        });
    }

    public void setEngineStatus(String text) {
        Platform.runLater(() -> engineLabel.setText(text));
    }

    // ---- JobRunner ----------------------------------------------------------

    @Override
    public <T> void run(Task<T> task, Consumer<T> onSuccess) {
        currentTask = task;
        // Never let the bar go indeterminate (-1): the animated indeterminate
        // ProgressBar forces continuous scene repaints (the whole-window "shimmer").
        // Show determinate progress when a task reports it, else a static 0.
        progressBar.progressProperty().unbind();
        progressBar.setProgress(0);
        task.progressProperty().addListener((o, ov, nv) -> {
            double p = nv.doubleValue();
            progressBar.setProgress(p < 0 ? 0 : p);
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
        String line = "[" + LocalTime.now().format(TS) + "] " + message;
        if (Platform.isFxApplicationThread()) statusLabel.setText(line);
        else Platform.runLater(() -> statusLabel.setText(line));
    }

    private void resetProgress() {
        progressBar.setProgress(0);
    }

    // ---- File menu: workspace ----------------------------------------------

    @FXML
    private void onOpenWorkspace() {
        if (bridge == null) { status("Engine not ready."); return; }
        FileChooser fc = workspaceChooser("Open workspace");
        File f = fc.showOpenDialog(window());
        if (f == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        status("Opening " + f.getName() + "…");
        run(bridge.command("load_workspace", args), summary -> {
            if (setupController != null) setupController.populate(summary);
            restoreGates(summary.path("gates"));
            navList.getSelectionModel().select("Workstation");
            status("Workspace loaded: " + f.getName());
        });
    }

    @FXML
    private void onLoadFcs() {
        if (bridge == null) { status("Engine not ready."); return; }
        javafx.stage.DirectoryChooser dc = new javafx.stage.DirectoryChooser();
        dc.setTitle("Select FCS folder");
        File dir = dc.showDialog(window());
        if (dir == null) return;
        javafx.scene.control.Alert ask = new javafx.scene.control.Alert(
                javafx.scene.control.Alert.AlertType.CONFIRMATION,
                "Search sub-folders for .fcs files too?",
                javafx.scene.control.ButtonType.YES, javafx.scene.control.ButtonType.NO);
        ask.setHeaderText("Include sub-folders?");
        boolean recursive = ask.showAndWait().orElse(javafx.scene.control.ButtonType.NO)
                == javafx.scene.control.ButtonType.YES;
        ObjectNode args = JSON.createObjectNode();
        args.put("folder", dir.getAbsolutePath().replace('\\', '/'));
        args.put("recursive", recursive);
        status("Loading FCS from " + dir.getName() + "…");
        run(bridge.command("load_fcs", args), summary -> {
            if (appCtx != null) appCtx.fmo().clearAll();   // FMO references don't carry across experiments (#18)
            if (setupController != null) setupController.populate(summary);   // publishes to the workspace
            navList.getSelectionModel().select("Workstation");
            status("Loaded FCS from " + dir.getName() + ".");
        });
    }

    @FXML
    private void onSaveWorkspace() {
        if (bridge == null) { status("Engine not ready."); return; }
        FileChooser fc = workspaceChooser("Save workspace");
        fc.setInitialFileName("experiment.sfw");
        File f = fc.showSaveDialog(window());
        if (f == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        args.set("gates", serializeGates());   // gates live on the Java side; ship them to the .sfw
        status("Saving " + f.getName() + "…");
        run(bridge.command("save_workspace", args),
                r -> status("Workspace saved: " + f.getName() + " ("
                        + r.path("n_gates").asInt() + " gate(s))."));
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
                gn.put("x_channel", g.xChan);
                if (g.yChan != null && !g.yChan.isBlank()) gn.put("y_channel", g.yChan);
                gn.put("angle", g.angle);
                ArrayNode xs = gn.putArray("xs");
                if (g.xs != null) for (double x : g.xs) xs.add(x);
                ArrayNode ys = gn.putArray("ys");
                if (g.ys != null) for (double y : g.ys) ys.add(y);
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
                byId.put(gn.path("id").asText(), new PopNode(g, null));
            }
            for (JsonNode gn : arr) {                       // pass 2: link parents
                PopNode node = byId.get(gn.path("id").asText());
                String pid = gn.path("parent_id").asText(null);
                PopNode parent = (pid != null) ? byId.getOrDefault(pid, root) : root;
                node.parent = parent;
                parent.children.add(node);
            }
            workspace.replaceTree(sample, root);
        });
        workspace.notifyTreeChanged();
    }

    private static double[] toArray(JsonNode arr) {
        if (arr == null || !arr.isArray()) return new double[0];
        double[] a = new double[arr.size()];
        for (int i = 0; i < arr.size(); i++) a[i] = arr.get(i).asDouble();
        return a;
    }

    @FXML
    private void onExit() {
        Platform.exit();
    }

    @FXML
    private void onSettings() {
        if (settings == null) { status("Engine not ready."); return; }
        javafx.scene.control.Spinner<Integer> dpi = new javafx.scene.control.Spinner<>(72, 1200, settings.exportDpi(), 50);
        dpi.setEditable(true);
        javafx.scene.layout.GridPane export = new javafx.scene.layout.GridPane();
        export.setHgap(10); export.setVgap(8); export.setStyle("-fx-padding:14;");
        export.addRow(0, new Label("Export DPI:"), dpi);
        export.add(new Label("Used by Copy and Save-as-SVG (300 = publication)."), 0, 1, 2, 1);

        Label about = new Label("StreamFLOW 2.0\nNative JavaFX flow-cytometry analysis.\nPython/FlowKit engine.");
        about.setStyle("-fx-padding:14;");

        javafx.scene.control.TabPane tabs = new javafx.scene.control.TabPane(
                new javafx.scene.control.Tab("Export", export),
                new javafx.scene.control.Tab("About", about));
        tabs.setTabClosingPolicy(javafx.scene.control.TabPane.TabClosingPolicy.UNAVAILABLE);

        javafx.scene.control.Dialog<javafx.scene.control.ButtonType> dlg = new javafx.scene.control.Dialog<>();
        dlg.setTitle("Settings"); dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(tabs);
        dlg.getDialogPane().getButtonTypes().addAll(javafx.scene.control.ButtonType.OK, javafx.scene.control.ButtonType.CANCEL);
        if (dlg.showAndWait().orElse(javafx.scene.control.ButtonType.CANCEL) == javafx.scene.control.ButtonType.OK) {
            settings.setExportDpi(dpi.getValue());
            status("Export DPI set to " + settings.exportDpi() + ".");
        }
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
