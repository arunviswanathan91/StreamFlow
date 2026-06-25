package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.animation.Interpolator;
import javafx.animation.KeyFrame;
import javafx.animation.KeyValue;
import javafx.animation.Timeline;
import javafx.beans.property.DoubleProperty;
import javafx.beans.property.SimpleDoubleProperty;
import javafx.collections.FXCollections;
import javafx.concurrent.Task;
import javafx.util.Duration;
import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.geometry.Pos;
import javafx.scene.Node;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonType;
import javafx.scene.control.ColorPicker;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Dialog;
import javafx.scene.control.Hyperlink;
import javafx.scene.control.Label;
import javafx.scene.control.Slider;
import javafx.scene.control.TextInputDialog;
import javafx.scene.control.TreeCell;
import javafx.scene.control.TreeItem;
import javafx.scene.control.TreeView;
import javafx.scene.layout.BorderPane;
import javafx.scene.layout.GridPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.StackPane;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.stage.Stage;
import org.controlsfx.control.PopOver;
import org.streamflow.bridge.RJobException;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;

/**
 * FlowJo-style Graph Window backed by native {@link CytoPlot} rendering. Events
 * are fetched from the engine once (or supplied directly for a drill-down); all
 * axis/plot/gate interaction is local and instant. Double-clicking a population
 * opens it in a new window scoped to that gate's events (FlowJo drill-down).
 */
public class GraphWindowController {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final String HIST = "(Histogram)";

    @FXML private ComboBox<String> plotTypeCombo, xAxisCombo, yAxisCombo, toolCombo;
    @FXML private ComboBox<String> xScaleCombo, yScaleCombo;
    @FXML private Button xAxisOptsButton, yAxisOptsButton;
    @FXML private Button prevSampleButton, nextSampleButton, copyButton, channelsButton;
    @FXML private Button fmoButton;
    @FXML private Button compareButton;
    @FXML private ComboBox<String> sampleJumpCombo;
    @FXML private javafx.scene.control.CheckBox smoothCheck;
    @FXML private javafx.scene.control.CheckBox snapCheck;
    @FXML private javafx.scene.control.CheckBox confidenceCheck;
    @FXML private javafx.scene.control.CheckBox backgateCheck;
    @FXML private ComboBox<String> histModeCombo;
    @FXML private Slider bandwidthSlider;
    @FXML private StackPane plotHost;
    @FXML private TreeView<PopNode> gateTree;
    @FXML private HBox breadcrumbBar;
    @FXML private HBox downsampleBanner;
    @FXML private Label downsampleLabel;
    @FXML private Label statusLabel;

    private final CytoPlot plot = new CytoPlot();
    private AppContext ctx;
    private String sample;

    private EventData rootData;          // whole sample (the root population)
    private EventData currentData;       // events currently shown in the plot (= dataForNode(currentNode))
    private PopNode rootNode;           // "All Events" (or the population a child window is rooted at)
    private PopNode currentNode;        // population currently shown in the plot
    private String rootName = "All Events";
    private String sampleFile = "";                        // FCS file = workspace key + engine sample arg
    private PopNode initialFocus;                          // node to focus when a child view opens
    private boolean suppressAxisEvents;                    // guard while restoring a population's view

    private List<String> sampleNames = new ArrayList<>(); // for prev/next nav
    private int sampleIndex = -1;

    private CytoPlot.Gate clipboardGate;   // copied gate template (Ctrl+C)

    // ---- undo/redo (§13, command pattern, 50 deep) --------------------------
    private record UndoEntry(Runnable undo, Runnable redo) {}
    private final java.util.ArrayDeque<UndoEntry> undoStack = new java.util.ArrayDeque<>();
    private final java.util.ArrayDeque<UndoEntry> redoStack = new java.util.ArrayDeque<>();
    private static final int UNDO_LIMIT = 50;
    private boolean suppressUndo = false;

    // gate edit pre-state (captured by setOnGateEditStart before any drag begins)
    private CytoPlot.Gate pendingEditGate;
    private double[] pendingEditXs, pendingEditYs;
    private double pendingEditAngle;

    // §11 Options panel fields
    @FXML private javafx.scene.control.CheckBox lightModeCheck;
    @FXML private javafx.scene.control.CheckBox labelsVisibleCheck;
    @FXML private javafx.scene.control.CheckBox fmoVisibleCheck;

    /** Top-level window for one sample, navigable across the whole sample list. */
    public static void open(AppContext ctx, List<String> sampleNames, int index, List<String> channels) {
        String name = sampleNames.get(index);
        Win w = build(ctx, name);
        w.controller().sampleNames = new ArrayList<>(sampleNames);
        w.controller().sampleIndex = index;
        w.controller().sampleFile = name;
        w.controller().loadFromEngine(true);
        w.stage().show();
    }

    /** Drill-down window: another view of the SAME sample/tree, initially focused on {@code focus}. */
    public static void openChild(AppContext ctx, String sampleFile, PopNode focus) {
        Win w = build(ctx, sampleFile);
        GraphWindowController c = w.controller();
        c.sampleFile = sampleFile;
        // Enable ◀/▶ navigation so you can scroll the SAME population across all samples.
        c.sampleNames = new ArrayList<>(ctx.workspace().sampleNames());
        c.sampleIndex = c.sampleNames.indexOf(sampleFile);
        c.initialFocus = focus;
        c.loadFromEngine(true);   // hits the workspace cache → shares tree + events, then focuses 'focus'
        w.stage().show();
    }

    private record Win(Stage stage, GraphWindowController controller) {}

    private static Win build(AppContext ctx, String title) {
        try {
            FXMLLoader loader = new FXMLLoader(
                    GraphWindowController.class.getResource("/org/streamflow/ui/graph-window.fxml"));
            BorderPane root = loader.load();
            GraphWindowController c = loader.getController();
            c.ctx = ctx;
            ctx.workspace().addTreeChangeListener(c::onExternalTreeChange);   // live sync across windows
            c.sample = title;
            c.plot.setChannelLabeler(ch -> ctx.aliases().label(ch));
            Scene scene = new Scene(root, 1040, 720);
            scene.getStylesheets().add(GraphWindowController.class
                    .getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
            scene.setOnKeyPressed(ev -> {
                if (ev.isControlDown()) {
                    switch (ev.getCode()) {
                        case C -> { c.copyGate(); return; }
                        case V -> { c.pasteGateElastic(); return; }
                        case Z -> { if (ev.isShiftDown()) c.doRedo(); else c.doUndo(); return; }
                        case Y -> { c.doRedo(); return; }
                        default -> { }
                    }
                    return;
                }
                switch (ev.getCode()) {
                    case ESCAPE -> { c.plot.cancelDrawing(); c.toolCombo.getSelectionModel().select("None"); }
                    case DELETE, BACK_SPACE -> c.deleteSelectedPopulation();
                    case LEFT  -> c.gotoSample(c.sampleIndex - 1);
                    case RIGHT -> c.gotoSample(c.sampleIndex + 1);
                    case P -> c.toolCombo.getSelectionModel().select("Polygon");
                    case R -> c.toolCombo.getSelectionModel().select("Rectangle");
                    case E -> c.toolCombo.getSelectionModel().select("Ellipse");
                    case I -> c.toolCombo.getSelectionModel().select("Interval");
                    case Q -> c.toolCombo.getSelectionModel().select("Quadrant");
                    case N -> c.toolCombo.getSelectionModel().select("None");
                    default -> { }
                }
            });
            Stage stage = new Stage();
            stage.setTitle("StreamFLOW — " + title);
            stage.setScene(scene);
            return new Win(stage, c);
        } catch (Exception e) {
            throw new RuntimeException("Could not open graph window: " + e.getMessage(), e);
        }
    }

    @FXML
    public void initialize() {
        plotTypeCombo.setItems(FXCollections.observableArrayList(
                "pseudocolor", "dot", "contour", "density", "zebra", "histogram"));
        plotTypeCombo.getSelectionModel().select("pseudocolor");
        toolCombo.setItems(FXCollections.observableArrayList(
                "None", "Polygon", "Rectangle", "Ellipse", "Interval", "Quadrant"));
        toolCombo.getSelectionModel().select("None");
        xScaleCombo.setItems(FXCollections.observableArrayList("Linear", "Log", "Logicle", "ArcSinh"));
        yScaleCombo.setItems(FXCollections.observableArrayList("Linear", "Log", "Logicle", "ArcSinh"));
        xScaleCombo.getSelectionModel().select("Linear");
        yScaleCombo.getSelectionModel().select("Linear");
        xAxisOptsButton.setOnAction(e -> showAxisOptions(true, xAxisOptsButton));
        yAxisOptsButton.setOnAction(e -> showAxisOptions(false, yAxisOptsButton));

        plotHost.getChildren().add(plot);
        plot.prefWidthProperty().bind(plotHost.widthProperty());
        plot.prefHeightProperty().bind(plotHost.heightProperty());
        plot.setOnGateDrawn(this::onGateDrawn);
        plot.setOnGateEditStart(g -> {
            pendingEditGate = g;
            pendingEditXs = g.xs.clone();
            pendingEditYs = g.ys == null ? null : g.ys.clone();
            pendingEditAngle = g.angle;
        });
        plot.setOnGateChanged(g -> {
            recomputeCounts(); refreshTreeLabels(); snapshotGate(g);
            if (pendingEditGate == g && pendingEditXs != null) {
                final double[] oldXs = pendingEditXs.clone();
                final double[] oldYs = pendingEditYs == null ? null : pendingEditYs.clone();
                final double oldAngle = pendingEditAngle;
                final double[] newXs = g.xs.clone();
                final double[] newYs = g.ys == null ? null : g.ys.clone();
                final double newAngle = g.angle;
                pushUndo(
                    () -> { g.xs = oldXs.clone(); if (g.ys != null && oldYs != null) { g.ys = oldYs.clone(); } g.angle = oldAngle;
                            recomputeCounts(); refreshTreeLabels(); plot.refresh(); notifyTree(); },
                    () -> { g.xs = newXs.clone(); if (g.ys != null && newYs != null) { g.ys = newYs.clone(); } g.angle = newAngle;
                            recomputeCounts(); refreshTreeLabels(); plot.refresh(); notifyTree(); }
                );
            }
            pendingEditGate = null; pendingEditXs = null; pendingEditYs = null;
        });
        plot.setOnHistoryRequest(this::openHistoryDialog);
        plot.setOnGateDeleted(this::onGateDeletedFromPlot);
        plot.setOnRenameRequest(this::renameGate);
        plot.setOnColorRequest(this::pickGateColor);
        plot.setOnOpenChild(this::openChildForGate);   // double-click gate body / "Open in new window"
        plot.setOnStatsConfig(this::configureStats);
        plot.setOnBusy(busy -> {                 // disable axis controls while a render runs
            xAxisCombo.setDisable(busy); yAxisCombo.setDisable(busy);
            xScaleCombo.setDisable(busy); yScaleCombo.setDisable(busy);
            plotTypeCombo.setDisable(busy);
        });

        plotTypeCombo.setOnAction(e -> { if (!suppressAxisEvents) applyAxes(); });
        xAxisCombo.setOnAction(e -> { if (suppressAxisEvents) return; xScaleCombo.getSelectionModel().select(defaultScale(xAxisCombo.getValue())); applyAxes(); });
        yAxisCombo.setOnAction(e -> { if (suppressAxisEvents) return; yScaleCombo.getSelectionModel().select(defaultScale(yAxisCombo.getValue())); applyAxes(); });
        xScaleCombo.setOnAction(e -> { if (!suppressAxisEvents) applyAxes(); });
        yScaleCombo.setOnAction(e -> { if (!suppressAxisEvents) applyAxes(); });
        toolCombo.setOnAction(e -> plot.setTool(toolCombo.getValue()));

        // real Ikonli icons (MD2 cog for axis settings, FontAwesome5 for the rest)
        iconOnly(prevSampleButton, "fas-chevron-left", "◀");
        iconOnly(nextSampleButton, "fas-chevron-right", "▶");
        iconOnly(xAxisOptsButton, "mdi2c-cog", "⚙");
        iconOnly(yAxisOptsButton, "mdi2c-cog", "⚙");
        withIcon(copyButton, "fas-copy");
        withIcon(channelsButton, "fas-tags");

        histModeCombo.setItems(FXCollections.observableArrayList("Filled Smooth", "Raw Bars", "Line Only"));
        histModeCombo.getSelectionModel().select("Filled Smooth");
        smoothCheck.setOnAction(e -> plot.setSmooth(smoothCheck.isSelected()));
        snapCheck.setOnAction(e -> plot.setSnapEnabled(snapCheck.isSelected()));
        confidenceCheck.setOnAction(e -> plot.setShowConfidence(confidenceCheck.isSelected()));
        backgateCheck.setOnAction(e -> {
            if (!backgateCheck.isSelected()) { plot.setHighlight(null); statusLabel.setText("Backgating off."); }
            else statusLabel.setText("Backgating on — select a descendant population in the tree to overlay it.");
        });
        fmoButton.setOnMouseClicked(e -> {
            if (ctx == null) return;
            String xch = xAxisCombo.getValue();
            boolean alreadySet = xch != null && ctx.fmo().has(xch);
            // Shift-click always clears; a plain click TOGGLES (set if absent, clear if present).
            if (e.isShiftDown() || alreadySet) {
                if (xch != null) ctx.fmo().clear(xch);
                if (yAxisCombo.getValue() != null) ctx.fmo().clear(yAxisCombo.getValue());
                applyFmoLines(HIST.equals(yAxisCombo.getValue()));
                statusLabel.setText("FMO reference cleared for current channel(s).");
            } else {
                setFmoFromCurrent();
            }
        });
        histModeCombo.setOnAction(e -> plot.setHistMode(histModeCombo.getValue()));
        bandwidthSlider.valueProperty().addListener((o, a, b) -> plot.setHistBandwidth(b.doubleValue()));

        prevSampleButton.setOnAction(e -> gotoSample(sampleIndex - 1));
        nextSampleButton.setOnAction(e -> gotoSample(sampleIndex + 1));
        sampleJumpCombo.setOnAction(e -> {
            int i = sampleJumpCombo.getSelectionModel().getSelectedIndex();
            if (i >= 0 && i != sampleIndex) gotoSample(i);
        });

        gateTree.setShowRoot(true);
        gateTree.setCellFactory(tv -> new TreeCell<>() {
            @Override protected void updateItem(PopNode n, boolean empty) {
                super.updateItem(n, empty);
                if (empty || n == null) { setText(null); setTooltip(null); return; }
                String nm = n.isRoot() ? rootName : n.name();
                setText(n.isRoot()
                        ? String.format("%s — %d", nm, Math.max(n.count, 0))
                        : String.format("%s — %d (%.1f%%)", nm, Math.max(n.count, 0),
                                        Double.isNaN(n.parentPct) ? 0 : n.parentPct));
                // #20 cell-type ontology hint
                CellOntology.Entry e = n.isRoot() ? null : CellOntology.match(n.name());
                if (e != null) {
                    setTooltip(new javafx.scene.control.Tooltip(
                            e.canonical() + "\nMarkers: " + e.markers() + "\nTypical: " + e.typical()));
                } else {
                    setTooltip(null);
                }
            }
        });
        gateTree.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> {
            if (b == null || b.getValue() == currentNode) return;
            PopNode sel = b.getValue();
            if (backgateCheck.isSelected() && currentNode != null && isStrictDescendant(currentNode, sel)) {
                overlayBackgate(sel);    // keep the current plot, overlay sel's events
            } else {
                selectNode(sel);
            }
        });
        gateTree.setOnMouseClicked(e -> {            // double-click a node -> open it in a new window
            if (e.getClickCount() == 2) {
                TreeItem<PopNode> it = gateTree.getSelectionModel().getSelectedItem();
                if (it != null && it.getValue() != null && !it.getValue().isRoot()) openChildForNode(it.getValue());
            }
        });
        refreshNav();
        if (lightModeCheck != null) lightModeCheck.setOnAction(e -> plot.setLightMode(lightModeCheck.isSelected()));
        if (labelsVisibleCheck != null) labelsVisibleCheck.setOnAction(e -> plot.setLabelsVisible(labelsVisibleCheck.isSelected()));
        if (fmoVisibleCheck != null) fmoVisibleCheck.setOnAction(e -> plot.setFmoVisible(fmoVisibleCheck.isSelected()));
        // Dragging an FMO line repositions it and persists the new level for the shown channel(s).
        plot.setOnFmoChanged((x, y) -> {
            if (ctx == null) return;
            String xch = xAxisCombo.getValue(), ych = yAxisCombo.getValue();
            if (xch != null && !Double.isNaN(x)) ctx.fmo().set(xch, x, sampleFile);
            if (!HIST.equals(ych) && ych != null && !Double.isNaN(y)) ctx.fmo().set(ych, y, sampleFile);
            statusLabel.setText("FMO line moved.");
        });
    }

    // ---- sample navigation (§3) ---------------------------------------------

    /** Switch to another sample in the list, keeping axes, scales, and gates. */
    private void gotoSample(int index) {
        if (sampleNames.isEmpty() || index < 0 || index >= sampleNames.size() || index == sampleIndex) return;
        sampleIndex = index;
        sample = sampleNames.get(index);
        sampleFile = sample;
        loadFromEngine(false);
    }

    /** Enable/populate the nav strip; hidden for drill-down windows. */
    private void refreshNav() {
        boolean navigable = sampleNames.size() > 1;
        prevSampleButton.setDisable(!navigable || sampleIndex <= 0);
        nextSampleButton.setDisable(!navigable || sampleIndex >= sampleNames.size() - 1);
        sampleJumpCombo.setDisable(sampleNames.isEmpty());
        if (!sampleNames.equals(sampleJumpCombo.getItems())) {
            sampleJumpCombo.setItems(FXCollections.observableArrayList(sampleNames));
        }
        if (sampleIndex >= 0) sampleJumpCombo.getSelectionModel().select(sampleIndex);
    }

    // ---- data loading -------------------------------------------------------

    private void loadFromEngine(boolean initial) {
        refreshNav();
        // Workspace cache hit → restore instantly (shares the gating tree, so gates persist).
        EventData cached = ctx.workspace().data(sampleFile);
        if (cached != null) {
            statusLabel.setText(sampleFile + " — " + cached.rows() + " events (restored)");
            if (initial) useData(cached); else reapplyData(cached);
            return;
        }
        if (sampleIndexTitle() != null) currentStage().setTitle("StreamFLOW — " + sampleIndexTitle());
        statusLabel.setText("Loading " + sampleFile + "…");
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sampleFile);
        Task<JsonNode> task = ctx.bridge().command("get_events", args);
        task.setOnSucceeded(e -> {
            try {
                JsonNode r = task.getValue();
                List<String> chans = new ArrayList<>();
                r.path("channels").forEach(n -> chans.add(n.asText()));
                Path bin = Paths.get(r.path("file").asText());
                EventData d = EventData.read(bin, chans, r.path("rows").asInt(), r.path("cols").asInt());
                try { Files.deleteIfExists(bin); } catch (Exception ignored) {}
                ctx.workspace().putData(sampleFile, d);   // cache for restore + child windows
                if (initial) useData(d); else reapplyData(d);
                int rows = r.path("rows").asInt(), total = r.path("total").asInt();
                boolean subsampled = total > rows;
                statusLabel.setText(sampleFile + " — " + rows + " events"
                        + (subsampled ? " (subsampled from " + total + ")" : ""));
                downsampleBanner.setVisible(subsampled);
                downsampleBanner.setManaged(subsampled);
                if (subsampled) downsampleLabel.setText(
                        "Displaying " + rows + " of " + total + " events — adjust limit in Settings.");
            } catch (Exception ex) {
                statusLabel.setText("Could not load events: " + ex.getMessage());
            }
        });
        task.setOnFailed(e -> {
            Throwable ex = task.getException();
            statusLabel.setText(ex instanceof RJobException ? ex.getMessage()
                    : "Load failed: " + (ex == null ? "?" : ex.getMessage()));
        });
        Thread t = new Thread(task, "get-events");
        t.setDaemon(true);
        t.start();
    }

    /** Load a sample: set up axes, then bind to the sample's SHARED workspace tree (so gating
     *  persists across windows) and focus the requested node (or its root). */
    private void useData(EventData d) {
        rootData = d;
        List<String> chans = d.channels();
        xAxisCombo.setItems(FXCollections.observableArrayList(chans));
        List<String> yOpts = new ArrayList<>(); yOpts.add(HIST); yOpts.addAll(chans);
        yAxisCombo.setItems(FXCollections.observableArrayList(yOpts));
        aliasCombo(xAxisCombo); aliasCombo(yAxisCombo);
        selectPreferred(xAxisCombo, chans, "FSC-A");
        selectPreferred(yAxisCombo, chans, "SSC-A");
        xScaleCombo.getSelectionModel().select(defaultScale(xAxisCombo.getValue()));
        yScaleCombo.getSelectionModel().select(defaultScale(yAxisCombo.getValue()));
        rootNode = ctx.workspace().treeFor(sampleFile);   // shared, persistent gating tree
        rebuildTree();
        recomputeCounts();
        PopNode focus = (initialFocus != null && isInTree(initialFocus)) ? initialFocus : rootNode;
        selectNode(focus);
    }

    private boolean isInTree(PopNode n) {
        return rootNode != null && rootNode.selfAndDescendants().contains(n);
    }

    /** Navigating to another sample: switch to THAT sample's own gating tree, keep axes/scales,
     *  and stay on the same population (matched by name path) so you can scroll P1 across samples. */
    private void reapplyData(EventData d) {
        rootData = d;
        List<String> keepPath = currentNode != null ? pathNames(currentNode) : new ArrayList<>();
        rootNode = ctx.workspace().treeFor(sampleFile);   // this sample's own persistent tree
        rebuildTree();
        recomputeCounts();
        PopNode target = resolveByNames(rootNode, keepPath);
        selectNode(target != null ? target : rootNode);
        refreshTreeLabels();
    }

    private boolean selfNotifying = false;

    /** Notify the workspace of a tree change made BY THIS window (so the Workstation and any other
     *  open graph windows refresh), without this window re-syncing to its own change. */
    private void notifyTree() {
        selfNotifying = true;
        try { ctx.workspace().notifyTreeChanged(); } finally { selfNotifying = false; }
    }

    /** Another window (or the Workstation) changed this sample's gating tree — re-sync this view,
     *  keeping the same population selected by name (so a deleted population drops out everywhere). */
    private void onExternalTreeChange() {
        if (selfNotifying || rootNode == null || ctx == null) return;
        List<String> keepPath = currentNode != null ? pathNames(currentNode) : new ArrayList<>();
        rootNode = ctx.workspace().treeFor(sampleFile);
        rebuildTree();
        recomputeCounts();
        PopNode target = resolveByNames(rootNode, keepPath);
        selectNode(target != null ? target : rootNode);
        refreshTreeLabels();
    }

    /** Find a population by its name-path (from just below root) within {@code root}, or null. */
    private PopNode resolveByNames(PopNode root, List<String> names) {
        PopNode cur = root;
        for (String nm : names) {
            PopNode next = null;
            for (PopNode ch : cur.children) if (ch.name().equals(nm)) { next = ch; break; }
            if (next == null) return null;
            cur = next;
        }
        return cur;
    }

    // ---- gating hierarchy (§8) ----------------------------------------------

    /** Events for a population: root events filtered by the AND of its ancestor gates. */
    private EventData dataForNode(PopNode node) {
        if (node == null || node.isRoot()) return rootData;
        boolean[] keep = new boolean[rootData.rows()];
        java.util.Arrays.fill(keep, true);
        for (CytoPlot.Gate g : node.chain()) {
            boolean[] m = CytoPlot.mask(rootData, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        return rootData.subset(keep);
    }

    /** True if {@code node} is a proper descendant of {@code ancestor}. */
    private static boolean isStrictDescendant(PopNode ancestor, PopNode node) {
        if (ancestor == null || node == null || ancestor == node) return false;
        for (PopNode n = node.parent; n != null; n = n.parent) if (n == ancestor) return true;
        return false;
    }

    /** Backgating: overlay {@code desc}'s events (orange) on the current plot without drilling in.
     *  The highlight mask is over {@link #currentData} — apply only the gates BELOW currentNode. */
    private void overlayBackgate(PopNode desc) {
        if (currentData == null) { return; }
        boolean[] keep = new boolean[currentData.rows()];
        java.util.Arrays.fill(keep, true);
        // gates from just below currentNode down to desc (the "extra" chain)
        List<CytoPlot.Gate> full = desc.chain();
        int skip = currentNode == null ? 0 : currentNode.chain().size();
        for (int i = skip; i < full.size(); i++) {
            boolean[] m = CytoPlot.mask(currentData, full.get(i));
            for (int r = 0; r < keep.length; r++) keep[r] = keep[r] && m[r];
        }
        int n = 0; for (boolean b : keep) if (b) n++;
        plot.setHighlight(keep);
        statusLabel.setText(String.format("Backgating '%s' onto '%s' — %,d event(s) highlighted",
                desc.name(), currentNode == null ? rootName : currentNode.name(), n));
    }

    /** Show a population in the plot: its events + its direct children as gates. */
    private void selectNode(PopNode node) {
        currentNode = node;
        // Restore the axes this population was gated on so its child gates are shown + editable.
        // If it has no children yet (viewX unset), fall back to the axes its OWN gate was drawn on
        // (and the parent's scales) instead of dropping to the default FSC/SSC view.
        String ax = node.viewX, ay = node.viewY, axs = node.viewXScale, ays = node.viewYScale;
        if (ax == null && !node.isRoot() && node.gate != null) {
            ax = node.gate.xChan;
            ay = node.gate.yChan;
            if (node.parent != null) { axs = node.parent.viewXScale; ays = node.parent.viewYScale; }
        }
        if (ax != null) {
            suppressAxisEvents = true;
            if (xAxisCombo.getItems().contains(ax)) xAxisCombo.getSelectionModel().select(ax);
            if (ay != null && yAxisCombo.getItems().contains(ay)) yAxisCombo.getSelectionModel().select(ay);
            if (axs != null) xScaleCombo.getSelectionModel().select(axs);
            if (ays != null) yScaleCombo.getSelectionModel().select(ays);
            suppressAxisEvents = false;
        }
        plot.clearGates();
        currentData = dataForNode(node);
        plot.setData(currentData);                 // also clears any backgate highlight
        for (PopNode ch : node.children) plot.addGate(ch.gate);
        applyAxes();
        // keep the tree selection in sync without re-entering the listener loop
        TreeItem<PopNode> item = findItem(gateTree.getRoot(), node);
        if (item != null) gateTree.getSelectionModel().select(item);
        refreshBreadcrumb();
        statusLabel.setText(node.name() + " — " + Math.max(node.count, 0) + " events");
        // window title reflects the focused population so each view shows its mother path
        String t = node.isRoot()
                ? (sampleIndexTitle() != null ? sampleIndexTitle() : sampleFile)
                : sampleFile + " · " + String.join(" / ", pathNames(node));
        currentStage().setTitle("StreamFLOW — " + t);
    }

    /** Recompute every node's event count + percent-of-parent for the current sample. */
    private void recomputeCounts() {
        if (rootNode == null || rootData == null) return;
        rootNode.count = rootData.rows();
        for (PopNode n : rootNode.selfAndDescendants()) {
            if (n.isRoot()) continue;
            EventData pd = dataForNode(n.parent);
            int c = 0; for (boolean b : CytoPlot.mask(pd, n.gate)) if (b) c++;
            n.count = c;
            n.parentPct = pd.rows() == 0 ? 0 : 100.0 * c / pd.rows();
            n.gate.statLine = statLineFor(n);
        }
    }

    // ---- gate statistics (Feature 1) ----------------------------------------

    private String statLineFor(PopNode node) {
        CytoPlot.Gate g = node.gate;
        if (g == null) return "";
        EventData nd = null;     // node events, built lazily for MFI/geomean/CV
        List<String> parts = new ArrayList<>();
        for (String key : g.statKeys) {
            if (key.equals("parent")) {
                parts.add(String.format("%.1f%% of parent", Double.isNaN(node.parentPct) ? 0 : node.parentPct));
            } else if (key.equals("total")) {
                double pct = rootData.rows() == 0 ? 0 : 100.0 * node.count / rootData.rows();
                parts.add(String.format("%.1f%% of total", pct));
            } else if (key.equals("count")) {
                parts.add(String.format("%,d events", node.count));
            } else if (key.startsWith("mfi:") || key.startsWith("geomean:") || key.startsWith("cv:")) {
                String chan = key.substring(key.indexOf(':') + 1);
                if (nd == null) nd = dataForNode(node);
                double[] v = channelValues(nd, chan);
                String cl = ctx.aliases().label(chan);
                if (key.startsWith("mfi:")) parts.add(cl + " MFI " + fmtStat(median(v)));
                else if (key.startsWith("geomean:")) parts.add(cl + " gMean " + fmtStat(geomean(v)));
                else parts.add(cl + " CV " + String.format("%.1f%%", cv(v)));
            }
        }
        return String.join("  |  ", parts);
    }

    private static double[] channelValues(EventData d, String chan) {
        int c = d.indexOf(chan);
        if (c < 0) return new double[0];
        double[] v = new double[d.rows()];
        for (int r = 0; r < v.length; r++) v[r] = d.get(r, c);
        return v;
    }
    private static double median(double[] v) {
        if (v.length == 0) return 0;
        double[] s = v.clone(); java.util.Arrays.sort(s); int n = s.length;
        return n % 2 == 1 ? s[n / 2] : (s[n / 2 - 1] + s[n / 2]) / 2;
    }
    private static double geomean(double[] v) {
        double sum = 0; int n = 0;
        for (double x : v) if (x > 0) { sum += Math.log(x); n++; }
        return n == 0 ? 0 : Math.exp(sum / n);
    }
    private static double cv(double[] v) {
        if (v.length == 0) return 0;
        double m = 0; for (double x : v) m += x; m /= v.length;
        double s = 0; for (double x : v) s += (x - m) * (x - m); s = Math.sqrt(s / v.length);
        return m == 0 ? 0 : 100 * s / m;
    }
    private static String fmtStat(double v) {
        double a = Math.abs(v);
        return (a >= 1e5 || (a > 0 && a < 0.01)) ? String.format("%.2e", v) : String.format("%,.0f", v);
    }

    /** Right-click → "Change Statistics Displayed": pick which stats appear on the label. */
    private void configureStats(CytoPlot.Gate g) {
        javafx.scene.control.CheckBox parent = new javafx.scene.control.CheckBox("% of parent");
        javafx.scene.control.CheckBox total = new javafx.scene.control.CheckBox("% of total");
        javafx.scene.control.CheckBox count = new javafx.scene.control.CheckBox("Event count");
        javafx.scene.control.CheckBox mfi = new javafx.scene.control.CheckBox("Median (MFI)");
        javafx.scene.control.CheckBox gmean = new javafx.scene.control.CheckBox("Geometric mean");
        javafx.scene.control.CheckBox cvBox = new javafx.scene.control.CheckBox("CV");
        ComboBox<String> chan = new ComboBox<>(FXCollections.observableArrayList(rootData.channels()));
        chan.getSelectionModel().select(g.xChan);
        for (String k : g.statKeys) {
            if (k.equals("parent")) parent.setSelected(true);
            else if (k.equals("total")) total.setSelected(true);
            else if (k.equals("count")) count.setSelected(true);
            else if (k.startsWith("mfi:")) { mfi.setSelected(true); chan.getSelectionModel().select(k.substring(4)); }
            else if (k.startsWith("geomean:")) gmean.setSelected(true);
            else if (k.startsWith("cv:")) cvBox.setSelected(true);
        }
        GridPane gp = new GridPane(); gp.setHgap(8); gp.setVgap(6);
        gp.add(parent, 0, 0); gp.add(total, 0, 1); gp.add(count, 0, 2);
        gp.add(mfi, 0, 3); gp.add(gmean, 0, 4); gp.add(cvBox, 0, 5);
        gp.add(new Label("Channel:"), 0, 6); gp.add(chan, 1, 6);

        Dialog<Void> dlg = new Dialog<>();
        dlg.setTitle("Statistics — " + g.name); dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(gp);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.APPLY, ButtonType.CLOSE);
        dlg.showAndWait();

        List<String> keys = new ArrayList<>();
        if (parent.isSelected()) keys.add("parent");
        if (total.isSelected()) keys.add("total");
        if (count.isSelected()) keys.add("count");
        String ch = chan.getValue() == null ? g.xChan : chan.getValue();
        if (mfi.isSelected()) keys.add("mfi:" + ch);
        if (gmean.isSelected()) keys.add("geomean:" + ch);
        if (cvBox.isSelected()) keys.add("cv:" + ch);
        if (keys.isEmpty()) keys.add("parent");   // at least one must remain
        g.statKeys.clear(); g.statKeys.addAll(keys);
        PopNode n = nodeForGate(g);
        if (n != null) g.statLine = statLineFor(n);
        plot.selectGate(g);   // repaint label
    }

    private void rebuildTree() {
        TreeItem<PopNode> root = buildItem(rootNode);
        root.setExpanded(true);
        gateTree.setRoot(root);
    }
    private TreeItem<PopNode> buildItem(PopNode n) {
        TreeItem<PopNode> item = new TreeItem<>(n);
        item.setExpanded(true);
        for (PopNode ch : n.children) item.getChildren().add(buildItem(ch));
        return item;
    }
    private void refreshTreeLabels() { gateTree.refresh(); }

    private TreeItem<PopNode> findItem(TreeItem<PopNode> from, PopNode n) {
        if (from == null) return null;
        if (from.getValue() == n) return from;
        for (TreeItem<PopNode> ch : from.getChildren()) {
            TreeItem<PopNode> r = findItem(ch, n);
            if (r != null) return r;
        }
        return null;
    }

    private void refreshBreadcrumb() {
        breadcrumbBar.getChildren().clear();
        // sample file shown greyed so the mother population is always clear
        if (sampleFile != null && !sampleFile.isBlank()) {
            Label s = new Label(sampleFile); s.setStyle("-fx-opacity:0.6;");
            breadcrumbBar.getChildren().addAll(s, new Label("›"));
        }
        // full path from root, all clickable (same shared tree + full sample data)
        List<PopNode> path = new ArrayList<>();
        for (PopNode n = currentNode; n != null; n = n.parent) path.add(0, n);
        for (int i = 0; i < path.size(); i++) {
            PopNode n = path.get(i);
            Hyperlink link = new Hyperlink(n.isRoot() ? rootName : n.name());
            link.setOnAction(e -> selectNode(n));
            breadcrumbBar.getChildren().add(link);
            if (i < path.size() - 1) breadcrumbBar.getChildren().add(new Label("›"));
        }
    }

    // ---- child windows (Bug 6 / Bug 8) --------------------------------------

    private void openChildForGate(CytoPlot.Gate g) {
        PopNode n = nodeForGate(g);
        if (n != null) openChildForNode(n);
    }

    private void openChildForNode(PopNode node) {
        if (node == null || node.isRoot()) return;
        openChild(ctx, sampleFile, node);   // shares the workspace tree + cached events
    }

    /** Population path from just below root down to (and including) the node. */
    private List<String> pathNames(PopNode node) {
        List<String> p = new ArrayList<>();
        for (PopNode n = node; n != null && !n.isRoot(); n = n.parent) p.add(0, n.name());
        return p;
    }

    private String sampleIndexTitle() {
        if (sampleNames.isEmpty() || sampleIndex < 0) return null;
        return String.format("%s  (%d/%d)", sample, sampleIndex + 1, sampleNames.size());
    }

    private Stage currentStage() { return (Stage) plotHost.getScene().getWindow(); }

    private void applyAxes() {
        if (xAxisCombo.getValue() == null) return;
        boolean hist = HIST.equals(yAxisCombo.getValue()) || "histogram".equals(plotTypeCombo.getValue());
        plot.setView(xAxisCombo.getValue(), hist ? null : yAxisCombo.getValue(),
                scaleOf(xScaleCombo.getValue()), scaleOf(yScaleCombo.getValue()),
                plotTypeCombo.getValue());   // single coalesced re-render off the FX thread
        applyFmoLines(hist);
    }

    /** Push the stored FMO levels (if any) for the current channels onto the plot. */
    private void applyFmoLines(boolean hist) {
        if (ctx == null) return;
        double fx = ctx.fmo().level(xAxisCombo.getValue());
        double fy = hist ? Double.NaN : ctx.fmo().level(yAxisCombo.getValue());
        plot.setFmo(fx, fy);
    }

    /** "Set FMO" — record the current sample's 95th-percentile level for the shown channel(s). */
    private void setFmoFromCurrent() {
        if (ctx == null || currentData == null) return;
        boolean hist = HIST.equals(yAxisCombo.getValue());
        String xch = xAxisCombo.getValue();
        if (xch != null) ctx.fmo().set(xch, percentile(currentData, xch, 95), sampleFile);
        String msg = "FMO set from " + sampleFile + " for " + xch;
        if (!hist && yAxisCombo.getValue() != null) {
            ctx.fmo().set(yAxisCombo.getValue(), percentile(currentData, yAxisCombo.getValue(), 95), sampleFile);
            msg += " + " + yAxisCombo.getValue();
        }
        applyFmoLines(hist);
        statusLabel.setText(msg + " (95th pct). Clears via Shift-click.");
        ctx.auditLog().add(AuditLog.Type.GATE, sampleFile, msg);
    }

    private static double percentile(EventData d, String channel, double pct) {
        int c = d.indexOf(channel);
        if (c < 0 || d.rows() == 0) return Double.NaN;
        double[] v = new double[d.rows()];
        for (int r = 0; r < v.length; r++) v[r] = d.get(r, c);
        java.util.Arrays.sort(v);
        int idx = (int) Math.min(v.length - 1, Math.round(pct / 100.0 * (v.length - 1)));
        return v[idx];
    }

    private static String defaultScale(String channel) {
        if (channel == null) return "Linear";
        // FlowJo v10: scatter/Time/Width linear; fluorescence defaults to Logicle (biexponential).
        return channel.matches("(?i).*(FSC|SSC|Time|Width).*") ? "Linear" : "Logicle";
    }

    private static CytoPlot.Scale scaleOf(String s) {
        if ("Log".equals(s)) return CytoPlot.Scale.LOG;
        if ("Logicle".equals(s)) return CytoPlot.Scale.LOGICLE;
        if ("ArcSinh".equals(s)) return CytoPlot.Scale.ARCSINH;
        return CytoPlot.Scale.LINEAR;
    }

    // ---- axis adjustment popover (icon next to each axis) --------------------

    private PopOver axisPopOver;

    /** FlowJo-style axis customizer: a slider whose meaning adapts to the scale. */
    private void showAxisOptions(boolean xAxis, Node anchor) {
        if (axisPopOver != null && axisPopOver.isShowing()) { axisPopOver.hide(); return; }
        CytoPlot.Scale scale = xAxis ? plot.xScale() : plot.yScale();
        String chan = xAxis ? xAxisCombo.getValue() : yAxisCombo.getValue();

        Slider slider = new Slider();
        Label valLbl = new Label();
        Label hint = new Label();
        hint.setWrapText(true); hint.setMaxWidth(220);
        Runnable apply;

        switch (scale) {
            case LOGICLE -> {
                slider.setMin(0.05); slider.setMax(LG_W_MAX);
                slider.setValue(clamp(xAxis ? plot.xWidth() : plot.yWidth(), 0.05, LG_W_MAX));
                hint.setText("Logicle width (W): larger = wider linear region around zero.");
                apply = () -> {
                    double w = slider.getValue();
                    if (xAxis) plot.setXWidth(w); else plot.setYWidth(w);
                    valLbl.setText(String.format("W = %.2f", w));
                };
            }
            case ARCSINH -> {
                slider.setMin(10); slider.setMax(10000);
                slider.setValue(clamp(xAxis ? plot.xCof() : plot.yCof(), 10, 10000));
                hint.setText("ArcSinh cofactor: larger = more low-end compression.");
                apply = () -> {
                    double c = slider.getValue();
                    if (xAxis) plot.setXCof(c); else plot.setYCof(c);
                    valLbl.setText(String.format("cofactor = %.0f", c));
                };
            }
            default -> {
                double dmax = Math.max(1, xAxis ? plot.xDataMax() : plot.yDataMax());
                slider.setMin(dmax * 0.05); slider.setMax(dmax);
                slider.setValue(dmax);
                hint.setText("Axis maximum (zoom the displayed range).");
                apply = () -> {
                    double m = slider.getValue();
                    if (xAxis) plot.setXMax(m); else plot.setYMax(m);
                    valLbl.setText(String.format("max = %.0f", m));
                };
            }
        }
        apply.run();
        // Apply only when the drag settles (or on a discrete click/keyboard step), not on every
        // intermediate value — applying on every tick floods the render pipeline and freezes the UI.
        slider.valueProperty().addListener((o, a, b) -> { if (!slider.isValueChanging()) apply.run(); });
        slider.valueChangingProperty().addListener((o, was, changing) -> { if (!changing) apply.run(); });

        Button auto = new Button("Auto");
        auto.setOnAction(ev -> {
            if (xAxis) plot.resetXRange(); else plot.resetYRange();
            switch (scale) {
                case LOGICLE -> { if (xAxis) plot.setXWidth(-1); else plot.setYWidth(-1);
                                  slider.setValue(clamp(xAxis ? plot.xWidth() : plot.yWidth(), 0.05, LG_W_MAX)); }
                case ARCSINH -> slider.setValue(150);
                default -> slider.setValue(slider.getMax());
            }
        });

        // transform selector (Linear / Log / Logicle / ArcSinh) — switches the axis scale
        ComboBox<String> tf = new ComboBox<>(FXCollections.observableArrayList("Linear", "Log", "Logicle", "ArcSinh"));
        tf.getSelectionModel().select(scaleName(scale));
        tf.setOnAction(ev -> {
            (xAxis ? xScaleCombo : yScaleCombo).getSelectionModel().select(tf.getValue()); // -> applyAxes
            if (axisPopOver != null) axisPopOver.hide();
            showAxisOptions(xAxis, anchor);   // reopen with the new scale's control
        });

        // range extension: widen the visible axis by ~one decade each side
        Button extMinus = new Button("− Extend");
        Button extPlus = new Button("+ Extend");
        extMinus.setOnAction(ev -> {
            double lo = xAxis ? plot.xMin() : plot.yMin();
            double hi = xAxis ? plot.xMax() : plot.yMax();
            double newLo = lo < 0 ? lo * 10 : -Math.max(1000, Math.abs(hi) * 0.1);
            if (xAxis) plot.setXMin(newLo); else plot.setYMin(newLo);
        });
        extPlus.setOnAction(ev -> {
            double hi = xAxis ? plot.xMax() : plot.yMax();
            double newHi = hi > 0 ? hi * 10 : 1000;
            if (xAxis) plot.setXMax(newHi); else plot.setYMax(newHi);
        });
        HBox extend = new HBox(6, extMinus, extPlus);
        extend.setAlignment(Pos.CENTER_LEFT);

        Label title = new Label((xAxis ? "X" : "Y") + " axis · " + (chan == null ? "" : chan));
        title.getStyleClass().add("subtitle");
        HBox tfRow = new HBox(8, new Label("Transform:"), tf);
        tfRow.setAlignment(Pos.CENTER_LEFT);
        HBox footer = new HBox(8, valLbl, auto);
        footer.setAlignment(Pos.CENTER_LEFT);
        VBox box = new VBox(8, title, tfRow, hint, slider, extend, footer);
        box.setStyle("-fx-padding:12; -fx-min-width:240;");

        PopOver pop = new PopOver(box);
        // open AWAY from the plot: Y popover to the left of its button, X popover to the right,
        // so the graph stays visible while scaling.
        pop.setArrowLocation(xAxis ? PopOver.ArrowLocation.LEFT_CENTER : PopOver.ArrowLocation.RIGHT_CENTER);
        pop.setDetachable(false); pop.setAutoHide(true);
        pop.show(anchor);
        axisPopOver = pop;
    }

    // ---- channel aliases (Feature 4) ----------------------------------------

    /** Show "CD4 (BV711-A)" in the combo while the stored value stays the raw channel. */
    private void aliasCombo(ComboBox<String> combo) {
        java.util.function.Function<String, String> lbl = ch ->
                (ch == null || HIST.equals(ch)) ? ch : ctx.aliases().label(ch);
        combo.setButtonCell(new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(String s, boolean empty) { super.updateItem(s, empty); setText(empty ? null : lbl.apply(s)); }
        });
        combo.setCellFactory(c -> new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(String s, boolean empty) { super.updateItem(s, empty); setText(empty ? null : lbl.apply(s)); }
        });
    }

    @FXML
    private void onChannels() {
        if (rootData == null) { statusLabel.setText("Load a sample first."); return; }
        GridPane gp = new GridPane(); gp.setHgap(10); gp.setVgap(6);
        gp.add(new Label("Channel"), 0, 0); gp.add(new Label("Target"), 1, 0);
        List<String> chans = rootData.channels();
        List<javafx.scene.control.TextField> fields = new ArrayList<>();
        for (int i = 0; i < chans.size(); i++) {
            String ch = chans.get(i);
            javafx.scene.control.TextField tf = new javafx.scene.control.TextField(ctx.aliases().target(ch));
            tf.setPromptText("e.g. CD4");
            fields.add(tf);
            gp.add(new Label(ch), 0, i + 1); gp.add(tf, 1, i + 1);
        }
        Dialog<javafx.scene.control.ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Channel target names");
        dlg.setHeaderText("Label detectors with their target (applies to all samples).");
        javafx.scene.control.ScrollPane sp = new javafx.scene.control.ScrollPane(gp);
        sp.setFitToWidth(true); sp.setPrefViewportHeight(360);
        dlg.getDialogPane().setContent(sp);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;
        for (int i = 0; i < chans.size(); i++) ctx.aliases().set(chans.get(i), fields.get(i).getText());
        aliasCombo(xAxisCombo); aliasCombo(yAxisCombo);
        recomputeCounts(); refreshTreeLabels();
        applyAxes();   // redraw plot axis labels with new aliases
        statusLabel.setText("Channel target names updated.");
    }

    @FXML
    private void onCopy() {
        showExportOptionsDialog();
    }

    private void showExportOptionsDialog() {
        // Font size
        Slider fontSlider = new Slider(6, 36, ctx.settings().exportFontSize());
        fontSlider.setMajorTickUnit(6); fontSlider.setMinorTickCount(5); fontSlider.setShowTickLabels(true);
        Label fontLabel = new Label(String.format("%.1f pt", ctx.settings().exportFontSize()));
        fontSlider.valueProperty().addListener((o, a, b) -> fontLabel.setText(String.format("%.1f pt", b.doubleValue())));

        // DPI for scaling
        Slider dpiSlider = new Slider(72, 1200, ctx.settings().exportDpi());
        dpiSlider.setMajorTickUnit(200); dpiSlider.setMinorTickCount(4); dpiSlider.setShowTickLabels(true);
        Label dpiLabel = new Label(ctx.settings().exportDpi() + " DPI");
        dpiSlider.valueProperty().addListener((o, a, b) -> dpiLabel.setText((int)b.doubleValue() + " DPI"));

        // Gate labels toggle
        javafx.scene.control.CheckBox gateLabelsCheck = new javafx.scene.control.CheckBox("Show gate labels & statistics");
        gateLabelsCheck.setSelected(ctx.settings().exportGateLabels());

        GridPane gp = new GridPane(); gp.setHgap(12); gp.setVgap(10);
        gp.add(new Label("Axis font size:"), 0, 0); gp.add(fontSlider, 1, 0); gp.add(fontLabel, 2, 0);
        gp.add(new Label("Export DPI:"), 0, 1); gp.add(dpiSlider, 1, 1); gp.add(dpiLabel, 2, 1);
        gp.add(gateLabelsCheck, 0, 2);

        Dialog<Void> dlg = new Dialog<>();
        dlg.setTitle("Export options");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(gp);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);

        if (dlg.showAndWait().orElse(ButtonType.CANCEL) == ButtonType.OK) {
            // Save settings
            ctx.settings().setExportFontSize(fontSlider.getValue());
            ctx.settings().setExportDpi((int) Math.round(dpiSlider.getValue()));
            ctx.settings().setExportGateLabels(gateLabelsCheck.isSelected());

            // Export with temporary label visibility setting
            boolean prevLabels = labelsVisibleCheck != null && labelsVisibleCheck.isSelected();
            plot.setLabelsVisible(gateLabelsCheck.isSelected());
            javafx.scene.image.WritableImage img = plot.exportImage(ctx.settings().exportScale());
            plot.setLabelsVisible(prevLabels);   // restore

            // Copy to clipboard
            javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
            cc.putImage(img);
            javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
            statusLabel.setText("Copied " + ctx.settings().exportDpi() + " DPI plot image"
                    + (gateLabelsCheck.isSelected() ? " with labels" : "") + " — paste into PowerPoint.");
        }
    }

    @FXML
    private void onSaveSvg() {
        javafx.stage.FileChooser fc = new javafx.stage.FileChooser();
        fc.setTitle("Save plot as SVG");
        fc.setInitialFileName((sampleFile.isBlank() ? "plot" : sampleFile.replaceAll("\\.fcs$", "")) + ".svg");
        fc.getExtensionFilters().add(new javafx.stage.FileChooser.ExtensionFilter("SVG vector (*.svg)", "*.svg"));
        java.io.File f = fc.showSaveDialog(currentStage());
        if (f == null) return;
        try {
            java.nio.file.Files.writeString(f.toPath(), plot.exportSvg());
            statusLabel.setText("Saved SVG: " + f.getName());
        } catch (Exception ex) {
            statusLabel.setText("Could not save SVG: " + ex.getMessage());
        }
    }

    private static final double LG_W_MAX = 2.0;
    private static double clamp(double v, double lo, double hi) { return Math.max(lo, Math.min(hi, v)); }
    // ---- icons (Ikonli; safe fallback to a glyph if a pack literal is missing) ----------
    private static final javafx.scene.paint.Color ICON_C = javafx.scene.paint.Color.web("#CFE3F2");
    private static void iconOnly(javafx.scene.control.Labeled b, String literal, String fallback) {
        try {
            org.kordamp.ikonli.javafx.FontIcon fi = new org.kordamp.ikonli.javafx.FontIcon(literal);
            fi.setIconSize(15); fi.setIconColor(ICON_C);
            b.setGraphic(fi); b.setText("");
        } catch (Exception e) { b.setText(fallback); }
    }
    private static void withIcon(javafx.scene.control.Labeled b, String literal) {
        try {
            org.kordamp.ikonli.javafx.FontIcon fi = new org.kordamp.ikonli.javafx.FontIcon(literal);
            fi.setIconSize(14); fi.setIconColor(ICON_C);
            b.setGraphic(fi);
        } catch (Exception ignored) { }
    }

    private static String scaleName(CytoPlot.Scale s) {
        return switch (s) { case LOG -> "Log"; case LOGICLE -> "Logicle"; case ARCSINH -> "ArcSinh"; default -> "Linear"; };
    }

    // ---- gating -------------------------------------------------------------

    /** A gate was drawn on the current population — add it as a child population. */
    private void onGateDrawn(CytoPlot.Gate g) {
        if ("quadrant".equals(g.type)) { onQuadrantDrawn(g); return; }

        TextInputDialog d = new TextInputDialog("P" + ctx.workspace().nextSeq(sampleFile));
        d.setTitle("Name gate"); d.setHeaderText(null); d.setContentText("Population name:");
        Optional<String> r = d.showAndWait();
        if (r.isEmpty() || r.get().isBlank()) { toolCombo.getSelectionModel().select("None"); return; }
        g.name = r.get().trim();
        PopNode node = new PopNode(g, currentNode);
        currentNode.children.add(node);
        node.snapshot();   // history #18: capture initial geometry
        // remember the axes this gate was drawn on, so reopening the parent shows + edits it
        currentNode.viewX = xAxisCombo.getValue();
        currentNode.viewY = yAxisCombo.getValue();
        currentNode.viewXScale = xScaleCombo.getValue();
        currentNode.viewYScale = yScaleCombo.getValue();
        plot.addGate(g);
        recomputeCounts();
        TreeItem<PopNode> parentItem = findItem(gateTree.getRoot(), currentNode);
        if (parentItem != null) { parentItem.getChildren().add(buildItem(node)); parentItem.setExpanded(true); }
        refreshTreeLabels();
        statusLabel.setText(String.format("Gate '%s': %d events (%.1f%%)", node.name(), node.count, node.parentPct));
        toolCombo.getSelectionModel().select("None"); // one gate per draw (FlowJo)
        notifyTree();          // update Workstation view
        ctx.auditLog().add(AuditLog.Type.GATE, sampleFile,
                String.format("'%s' (%s on %s / %s)", g.name, g.type, g.xChan, g.yChan == null ? "—" : g.yChan));
        // §13 undo: remove the gate; redo: put it back
        final PopNode addedNode = node;
        final PopNode addedParent = currentNode;
        final int addedIdx = addedParent.children.indexOf(addedNode);
        pushUndo(
            () -> { removeNode(addedNode); },
            () -> { reAddNode(addedNode, addedParent, addedIdx); }
        );
    }

    private void onQuadrantDrawn(CytoPlot.Gate qg) {
        TextInputDialog d = new TextInputDialog("Q");
        d.setTitle("Quadrant gate prefix"); d.setHeaderText(null);
        d.setContentText("Prefix for Q1/Q2/Q3/Q4 (e.g. 'Annex' → Annex Q1 … Q4):");
        Optional<String> r = d.showAndWait();
        if (r.isEmpty() || r.get().isBlank()) { toolCombo.getSelectionModel().select("None"); return; }
        String prefix = r.get().trim();

        currentNode.viewX = xAxisCombo.getValue();
        currentNode.viewY = yAxisCombo.getValue();
        currentNode.viewXScale = xScaleCombo.getValue();
        currentNode.viewYScale = yScaleCombo.getValue();

        String[] qTypes  = {"q1", "q2", "q3", "q4"};
        String[] qLabels = {"Q1 (top-right)", "Q2 (top-left)", "Q3 (bottom-left)", "Q4 (bottom-right)"};
        TreeItem<PopNode> parentItem = findItem(gateTree.getRoot(), currentNode);
        for (int k = 0; k < 4; k++) {
            CytoPlot.Gate subGate = new CytoPlot.Gate(
                    prefix + " " + qLabels[k].split(" ")[0],
                    qTypes[k], qg.xChan, qg.yChan,
                    new double[]{qg.xs[0]}, new double[]{qg.ys[0]});
            subGate.border = qg.border;
            PopNode node = new PopNode(subGate, currentNode);
            currentNode.children.add(node);
            plot.addGate(subGate);
            if (parentItem != null) { parentItem.getChildren().add(buildItem(node)); }
        }
        if (parentItem != null) parentItem.setExpanded(true);
        recomputeCounts();
        refreshTreeLabels();
        toolCombo.getSelectionModel().select("None");
        notifyTree();
        statusLabel.setText("Quadrant gate '" + prefix + "' added (Q1–Q4).");
        ctx.auditLog().add(AuditLog.Type.GATE, sampleFile,
                String.format("Quadrant '%s' (Q1–Q4 on %s / %s)", prefix, qg.xChan, qg.yChan));
    }

    // ---- elastic gate templates (#13): copy / paste-and-snap-to-peak / undo --

    /** Ctrl+C — copy the selected gate as a reusable template. */
    private void copyGate() {
        CytoPlot.Gate sel = plot.selectedGate();
        if (sel == null) { statusLabel.setText("Select a gate first, then Ctrl+C to copy it."); return; }
        clipboardGate = sel;
        statusLabel.setText("Copied gate '" + sel.name + "'. Ctrl+V pastes it onto the current plot (snaps to nearest peak).");
    }

    /** Ctrl+V — paste the template onto the current plot, then elastically shift it to the
     *  nearest density peak with a short EASE_OUT animation. Ctrl+Z reverts. */
    private void pasteGateElastic() {
        if (clipboardGate == null) { statusLabel.setText("Nothing to paste — copy a gate with Ctrl+C first."); return; }
        if (currentNode == null) return;
        boolean needsY = !"interval".equals(clipboardGate.type);
        boolean histView = HIST.equals(yAxisCombo.getValue());
        if (needsY && histView) { statusLabel.setText("Can't paste a 2-D gate onto a histogram view."); return; }
        if (clipboardGate.xs == null) return;

        CytoPlot.Gate g = cloneGateForCurrentView(clipboardGate);
        PopNode node = new PopNode(g, currentNode);
        currentNode.children.add(node);
        currentNode.viewX = xAxisCombo.getValue();
        currentNode.viewY = yAxisCombo.getValue();
        currentNode.viewXScale = xScaleCombo.getValue();
        currentNode.viewYScale = yScaleCombo.getValue();
        plot.addGate(g);
        TreeItem<PopNode> parentItem = findItem(gateTree.getRoot(), currentNode);
        if (parentItem != null) { parentItem.getChildren().add(buildItem(node)); parentItem.setExpanded(true); }

        final PopNode pastedNode = node;
        final PopNode pastedParent = currentNode;
        final int pastedIdx = pastedParent.children.indexOf(pastedNode);
        pushUndo(
            () -> { removeNode(pastedNode); },
            () -> { reAddNode(pastedNode, pastedParent, pastedIdx); }
        );

        double[] off = plot.nearestPeakOffset(g);
        if (Math.hypot(off[0], off[1]) > 0 && (g.ys != null)) {
            animateGateOffset(g, off, node);
            statusLabel.setText("Pasted '" + g.name + "' — auto-adjusted to nearest density peak. Ctrl+Z to revert.");
            ctx.auditLog().add(AuditLog.Type.GATE, sampleFile, "Elastic paste '" + g.name + "' (auto-adjusted to peak)");
        } else {
            recomputeCounts(); refreshTreeLabels(); notifyTree();
            statusLabel.setText("Pasted '" + g.name + "'. Ctrl+Z to revert.");
        }
    }

    /** Delegates to the full undo stack (kept for compatibility). */
    private void undoPaste() { doUndo(); }

    /** Clone a template gate, retargeted to the current view's channels, with a unique name. */
    private CytoPlot.Gate cloneGateForCurrentView(CytoPlot.Gate s) {
        String xch = xAxisCombo.getValue();
        boolean hist = HIST.equals(yAxisCombo.getValue());
        String ych = "interval".equals(s.type) ? null : (hist ? null : yAxisCombo.getValue());
        CytoPlot.Gate g = new CytoPlot.Gate(uniqueGateName(s.name), s.type, xch, ych,
                s.xs.clone(), s.ys == null ? null : s.ys.clone());
        g.border = s.border; g.fill = s.fill;
        g.statKeys.clear(); g.statKeys.addAll(s.statKeys);
        return g;
    }

    private String uniqueGateName(String base) {
        String root = (base == null || base.isBlank()) ? "Gate" : base;
        java.util.Set<String> used = new java.util.HashSet<>();
        for (PopNode ch : currentNode.children) used.add(ch.name());
        if (!used.contains(root + " copy")) return root + " copy";
        int k = 2; while (used.contains(root + " copy " + k)) k++;
        return root + " copy " + k;
    }

    /** Interpolate a gate's vertices to a target offset over 300 ms (EASE_OUT), repainting live. */
    private void animateGateOffset(CytoPlot.Gate g, double[] off, PopNode node) {
        double[] ox = g.xs.clone(), oy = g.ys.clone();
        DoubleProperty p = new SimpleDoubleProperty(0);
        p.addListener((o, a, b) -> {
            double fr = b.doubleValue();
            for (int i = 0; i < ox.length; i++) {
                g.xs[i] = ox[i] + off[0] * fr;
                if (g.ys != null) g.ys[i] = oy[i] + off[1] * fr;
            }
            plot.refresh();
        });
        Timeline tl = new Timeline(
                new KeyFrame(Duration.ZERO, new KeyValue(p, 0)),
                new KeyFrame(Duration.millis(300), new KeyValue(p, 1, Interpolator.EASE_OUT)));
        tl.setOnFinished(e -> {
            recomputeCounts(); refreshTreeLabels(); notifyTree();
        });
        tl.play();
    }

    // ---- gate history replay (#18) ------------------------------------------

    private void snapshotGate(CytoPlot.Gate g) {
        PopNode n = nodeForGate(g);
        if (n != null) n.snapshot();
    }

    /** Scrub through a gate's saved geometry snapshots; preview live and optionally restore one. */
    private void openHistoryDialog(CytoPlot.Gate g) {
        PopNode node = nodeForGate(g);
        if (node == null || node.history.size() < 2) {
            statusLabel.setText("No earlier versions of this gate yet — edit it to build history.");
            return;
        }
        final double[] curXs = g.xs.clone();
        final double[] curYs = g.ys == null ? null : g.ys.clone();
        int last = node.history.size() - 1;

        javafx.scene.control.Slider slider = new javafx.scene.control.Slider(0, last, last);
        slider.setMajorTickUnit(1); slider.setMinorTickCount(0); slider.setSnapToTicks(true);
        slider.setShowTickMarks(true);
        Label info = new Label();
        java.time.format.DateTimeFormatter tf = java.time.format.DateTimeFormatter.ofPattern("HH:mm:ss");
        Runnable show = () -> {
            int i = (int) Math.round(slider.getValue());
            PopNode.GateSnapshot s = node.history.get(i);
            System.arraycopy(s.xs(), 0, g.xs, 0, Math.min(s.xs().length, g.xs.length));
            if (g.ys != null && s.ys() != null) System.arraycopy(s.ys(), 0, g.ys, 0, Math.min(s.ys().length, g.ys.length));
            plot.refresh();
            String stamp = java.time.Instant.ofEpochMilli(s.when())
                    .atZone(java.time.ZoneId.systemDefault()).toLocalTime().format(tf);
            info.setText(String.format("Version %d/%d — %s  (%.1f%% of parent)",
                    i + 1, node.history.size(), stamp,
                    Double.isNaN(s.parentPct()) ? 0 : s.parentPct()));
        };
        slider.valueProperty().addListener((o, a, b) -> show.run());

        javafx.scene.layout.VBox box = new javafx.scene.layout.VBox(10,
                new Label("Drag to scrub through this gate's history:"), slider, info);
        box.setPadding(new javafx.geometry.Insets(14));
        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Gate history — " + node.name());
        dlg.getDialogPane().setContent(box);
        ButtonType restore = new ButtonType("Restore this version", javafx.scene.control.ButtonBar.ButtonData.OK_DONE);
        dlg.getDialogPane().getButtonTypes().addAll(restore, ButtonType.CANCEL);
        show.run();

        ButtonType result = dlg.showAndWait().orElse(ButtonType.CANCEL);
        if (result == restore) {
            int i = (int) Math.round(slider.getValue());
            PopNode.GateSnapshot s = node.history.get(i);
            g.xs = s.xs().clone();
            g.ys = s.ys() == null ? null : s.ys().clone();
            node.snapshot();
            recomputeCounts(); refreshTreeLabels(); plot.refresh();
            notifyTree();
            statusLabel.setText("Restored '" + node.name() + "' to version " + (i + 1) + ".");
        } else {
            g.xs = curXs; g.ys = curYs;   // revert preview
            plot.refresh();
        }
    }

    // ---- sample comparison strip (#6) ---------------------------------------

    /** Show the current population on the same axes across every opened sample, side by side. */
    @FXML
    private void onCompareStrip() {
        if (ctx == null || currentNode == null) return;
        WorkspaceModel ws = ctx.workspace();
        List<String> samples = new ArrayList<>();
        for (String s : ws.sampleNames()) { EventData d = ws.data(s); if (d != null && d.rows() > 0) samples.add(s); }
        if (samples.size() < 2) { statusLabel.setText("Open at least 2 samples (graph windows) to compare."); return; }

        List<CytoPlot.Gate> chain = currentNode.chain();
        boolean hist = HIST.equals(yAxisCombo.getValue()) || "histogram".equals(plotTypeCombo.getValue());
        CytoPlot.Scale xs = scaleOf(xScaleCombo.getValue()), ysc = scaleOf(yScaleCombo.getValue());
        String xch = xAxisCombo.getValue(), ych = hist ? null : yAxisCombo.getValue(), type = plotTypeCombo.getValue();

        HBox strip = new HBox(8);
        strip.setPadding(new javafx.geometry.Insets(10));
        strip.setStyle("-fx-background-color:#0D1B2A;");
        for (String s : samples) {
            EventData root = ws.data(s);
            boolean[] keep = new boolean[root.rows()];
            java.util.Arrays.fill(keep, true);
            for (CytoPlot.Gate g : chain) {
                boolean[] m = CytoPlot.mask(root, g);
                for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
            }
            EventData sub = root.subset(keep);

            CytoPlot mini = new CytoPlot();
            mini.setMinSize(230, 230); mini.setPrefSize(230, 230); mini.setMaxSize(230, 230);
            mini.setChannelLabeler(ch -> ctx.aliases().label(ch));
            mini.setData(sub);
            mini.setView(xch, ych, xs, ysc, type);
            for (PopNode chn : currentNode.children) mini.addGate(chn.gate);

            Label cap = new Label(shortLabel(s) + "  (" + sub.rows() + ")");
            cap.setStyle("-fx-font-size:11; -fx-text-fill:#cfe;");
            VBox cell = new VBox(4, cap, mini);
            cell.setStyle("-fx-border-color:#33425a; -fx-padding:4;");
            cell.setOnMouseClicked(ev -> {
                if (ev.getClickCount() == 2) {
                    int idx = sampleNames.indexOf(s);
                    GraphWindowController.open(ctx, sampleNames.isEmpty() ? List.of(s) : sampleNames,
                            Math.max(0, idx), root.channels());
                }
            });
            strip.getChildren().add(cell);
        }
        javafx.scene.control.ScrollPane sp = new javafx.scene.control.ScrollPane(strip);
        sp.setFitToHeight(true);
        sp.setStyle("-fx-background:#0D1B2A; -fx-background-color:#0D1B2A;");
        String pop = currentNode.isRoot() ? rootName : currentNode.name();
        Stage stage = new Stage();
        stage.setTitle("Compare — " + pop + " across " + samples.size() + " samples");
        Scene scene = new Scene(sp, Math.min(1200, 60 + samples.size() * 250), 320);
        scene.getStylesheets().add(GraphWindowController.class
                .getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
        stage.setScene(scene);
        stage.show();
        statusLabel.setText("Comparison strip: " + pop + " across " + samples.size()
                + " sample(s). Double-click a thumbnail to open it.");
    }

    private static String shortLabel(String s) {
        String n = s.replaceAll("(?i)\\.fcs$", "");
        return n.length() > 22 ? n.substring(0, 21) + "…" : n;
    }

    @FXML
    private void onRemoveGate() {
        TreeItem<PopNode> sel = gateTree.getSelectionModel().getSelectedItem();
        if (sel != null && sel.getValue() != null && !sel.getValue().isRoot()) removeNode(sel.getValue());
    }

    /** Delete key: remove the gate selected on the plot, else the population selected in the tree
     *  (no confirm; undo via Ctrl+Z / the Undo button). */
    void deleteSelectedPopulation() {
        if (plot.selectedGate() != null) { plot.deleteSelected(); return; }
        TreeItem<PopNode> sel = gateTree.getSelectionModel().getSelectedItem();
        if (sel != null && sel.getValue() != null && !sel.getValue().isRoot()) removeNode(sel.getValue());
    }

    /** Plot deleted a gate (Delete key / context menu) — drop its population subtree. */
    private void onGateDeletedFromPlot(CytoPlot.Gate g) {
        PopNode n = nodeForGate(g);
        if (n != null) removeNode(n);
    }

    private void removeNode(PopNode node) {
        if (node == null || node.isRoot()) return;
        PopNode parent = node.parent;
        int savedIdx = parent.children.indexOf(node);
        pushUndo(
            () -> { reAddNode(node, parent, savedIdx); },
            () -> { removeNode(node); }
        );
        parent.children.remove(node);
        TreeItem<PopNode> item = findItem(gateTree.getRoot(), node);
        if (item != null && item.getParent() != null) item.getParent().getChildren().remove(item);
        recomputeCounts();
        refreshTreeLabels();
        selectNode(isAncestor(node, currentNode) ? parent : currentNode);
        statusLabel.setText("Deleted population '" + node.name() + "'");
        notifyTree();   // update Workstation view
    }

    private static boolean isAncestor(PopNode ancestor, PopNode of) {
        for (PopNode n = of; n != null; n = n.parent) if (n == ancestor) return true;
        return false;
    }

    // ---- undo/redo management -----------------------------------------------

    private void pushUndo(Runnable undo, Runnable redo) {
        if (suppressUndo) return;
        if (undoStack.size() >= UNDO_LIMIT) undoStack.removeFirst();
        undoStack.addLast(new UndoEntry(undo, redo));
        redoStack.clear();
    }

    @FXML private void onUndo() { doUndo(); }
    @FXML private void onRedo() { doRedo(); }

    void doUndo() {
        if (undoStack.isEmpty()) { statusLabel.setText("Nothing to undo."); return; }
        UndoEntry e = undoStack.removeLast();
        suppressUndo = true;
        try { e.undo().run(); } finally { suppressUndo = false; }
        if (redoStack.size() < UNDO_LIMIT) redoStack.addLast(e);
        statusLabel.setText("Undo. " + undoStack.size() + " action(s) remaining.");
    }

    void doRedo() {
        if (redoStack.isEmpty()) { statusLabel.setText("Nothing to redo."); return; }
        UndoEntry e = redoStack.removeLast();
        suppressUndo = true;
        try { e.redo().run(); } finally { suppressUndo = false; }
        if (undoStack.size() < UNDO_LIMIT) undoStack.addLast(e);
        statusLabel.setText("Redo.");
    }

    /** Re-insert a node that was previously removed (for undo). */
    private void reAddNode(PopNode node, PopNode parent, int idx) {
        if (parent == null || node == null) return;
        int pos = Math.min(idx, parent.children.size());
        parent.children.add(pos, node);
        rebuildTree();
        if (parent == currentNode) plot.addGate(node.gate);
        recomputeCounts(); refreshTreeLabels();
        selectNode(currentNode != null ? currentNode : rootNode);
        notifyTree();
    }

    private PopNode nodeForGate(CytoPlot.Gate g) {
        if (rootNode == null) return null;
        for (PopNode n : rootNode.selfAndDescendants()) if (n.gate == g) return n;
        return null;
    }

    /** Rename a gate/population (double-click label / context menu). */
    private void renameGate(CytoPlot.Gate g) {
        TextInputDialog d = new TextInputDialog(g.name);
        d.setTitle("Rename population"); d.setHeaderText(null); d.setContentText("Population name:");
        Optional<String> r = d.showAndWait();
        if (r.isEmpty() || r.get().isBlank()) return;
        g.name = r.get().trim();
        plot.selectGate(g);
        refreshTreeLabels();
    }

    /** Per-gate border/fill colour + fill opacity (context menu → Color…). */
    private void pickGateColor(CytoPlot.Gate g) {
        ColorPicker border = new ColorPicker(g.border);
        ColorPicker fill = new ColorPicker(opaque(g.fill));
        Slider opacity = new Slider(0, 1, g.fill.getOpacity());
        opacity.setShowTickLabels(true); opacity.setMajorTickUnit(0.25);
        Runnable apply = () -> {
            g.border = border.getValue();
            Color fc = fill.getValue();
            g.fill = Color.color(fc.getRed(), fc.getGreen(), fc.getBlue(), opacity.getValue());
            plot.selectGate(g);   // redraws
        };
        border.setOnAction(e -> apply.run());
        fill.setOnAction(e -> apply.run());
        opacity.valueProperty().addListener((o, a, b) -> apply.run());

        GridPane gp = new GridPane(); gp.setHgap(8); gp.setVgap(8);
        gp.addRow(0, new Label("Border"), border);
        gp.addRow(1, new Label("Fill"), fill);
        gp.addRow(2, new Label("Fill opacity"), opacity);
        Dialog<Void> dlg = new Dialog<>();
        dlg.setTitle("Gate colour — " + g.name);
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(gp);
        dlg.getDialogPane().getButtonTypes().add(ButtonType.CLOSE);
        dlg.showAndWait();
    }

    private static Color opaque(Color c) { return Color.color(c.getRed(), c.getGreen(), c.getBlue()); }

    private static void selectPreferred(ComboBox<String> combo, List<String> channels, String pref) {
        String m = channels.stream().filter(c -> c.equalsIgnoreCase(pref)).findFirst()
                .orElse(channels.isEmpty() ? null : channels.get(0));
        if (m != null) combo.getSelectionModel().select(m);
    }
}
