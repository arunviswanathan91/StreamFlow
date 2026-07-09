package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.application.Platform;
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
import javafx.scene.control.MenuButton;
import javafx.scene.control.MenuItem;
import javafx.scene.control.Slider;
import javafx.scene.control.TextInputDialog;
import javafx.scene.control.Toggle;
import javafx.scene.control.ToggleButton;
import javafx.scene.control.ToggleGroup;
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

    @FXML private ComboBox<String> plotTypeCombo, xAxisCombo, yAxisCombo;
    @FXML private ToggleGroup toolGroup;
    @FXML private javafx.scene.control.ToggleButton toolNoneBtn, toolPolyBtn, toolRectBtn,
                                                     toolEllipseBtn, toolIntervalBtn, toolQuadBtn;
    @FXML private ComboBox<String> xScaleCombo, yScaleCombo;
    @FXML private Button xAxisOptsButton, yAxisOptsButton;
    @FXML private Button prevSampleButton, nextSampleButton, copyButton, copySettingsButton, channelsButton;
    @FXML private Button fmoButton;
    @FXML private Button compareButton;
    @FXML private MenuButton jumpButton;
    @FXML private ComboBox<String> sampleJumpCombo;
    @FXML private javafx.scene.control.CheckBox smoothCheck;
    @FXML private javafx.scene.control.CheckBox smoothDensityCheck, contourSmoothCheck;
    @FXML private javafx.scene.control.Slider contourSmoothSlider;
    @FXML private Label contourSmoothLabel;
    @FXML private javafx.scene.control.Slider contourLevelsSlider;
    @FXML private Label contourLevelsLabel;
    @FXML private javafx.scene.control.CheckBox contourOutliersCheck;
    @FXML private javafx.scene.control.Slider outlierSizeSlider;
    @FXML private Label outlierSizeLabel;
    @FXML private javafx.scene.control.Slider sizeControlSlider;
    @FXML private Label sizeControlLabel;
    @FXML private ComboBox<String> paletteCombo;
    @FXML private javafx.scene.control.ColorPicker contourColorPicker, dotColorPicker, backgateColorPicker;
    @FXML private javafx.scene.control.ColorPicker gateBorderColorPicker, gateFillColorPicker, gateTextColorPicker, gateShadowColorPicker;
    @FXML private javafx.scene.control.Slider gateOpacitySlider, gateShadowOpacitySlider;
    @FXML private javafx.scene.control.CheckBox gateTextShadowCheck;
    @FXML private javafx.scene.control.CheckBox interceptCheck;
    @FXML private javafx.scene.control.TextField interceptXField, interceptYField;
    @FXML private javafx.scene.control.TitledPane histGroup, densityGroup, contourGroup;
    @FXML private javafx.scene.control.CheckBox snapCheck;
    @FXML private javafx.scene.control.CheckBox confidenceCheck;
    @FXML private javafx.scene.control.CheckBox backgateCheck;
    @FXML private ComboBox<String> histModeCombo;
    @FXML private Slider bandwidthSlider;
    private boolean fmoShiftDown = false;   // tracked so a plain vs shift click on "Set FMO" can differ
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
        UiFx.fadeSlideIn(w.stage().getScene().getRoot());
        w.stage().toFront();
        ctx.workspace().registerWindow(name, w.stage());  // §14: track for focus-existing behaviour
        w.stage().getProperties().put("gwc", w.controller());  // let openChild reuse this one window
    }

    /** Drill-down window: another view of the SAME sample/tree, initially focused on {@code focus}.
     *  @param stealFocus true when the user explicitly navigated here (double-click); false for
     *                    background auto-opens (gate draw) so the parent window keeps focus. */
    public static void openChild(AppContext ctx, String sampleFile, PopNode focus, boolean stealFocus) {
        // §14/BUG-10: one window per sample. If a window for this sample is already open (opened from
        // the Workstation OR a previous drill-down/export double-click), navigate it to the requested
        // population instead of spawning a duplicate — so edits there always reflect back everywhere.
        javafx.stage.Stage existing = ctx.workspace().openWindowFor(sampleFile);
        if (existing != null && existing.getProperties().get("gwc") instanceof GraphWindowController gwc) {
            gwc.navigateTo(focus);
            if (stealFocus) { existing.toFront(); existing.requestFocus(); }
            return;
        }
        Win w = build(ctx, sampleFile);
        GraphWindowController c = w.controller();
        c.sampleFile = sampleFile;
        c.sampleNames = new ArrayList<>(ctx.workspace().sampleNames());
        c.sampleIndex = c.sampleNames.indexOf(sampleFile);
        c.initialFocus = focus;
        c.loadFromEngine(true);
        w.stage().show();
        UiFx.fadeSlideIn(w.stage().getScene().getRoot());
        ctx.workspace().registerWindow(sampleFile, w.stage());   // register so future opens reuse it
        w.stage().getProperties().put("gwc", c);
        if (stealFocus) { w.stage().toFront(); w.stage().requestFocus(); }
    }

    /** Convenience overload — explicit navigation always steals focus. */
    public static void openChild(AppContext ctx, String sampleFile, PopNode focus) {
        openChild(ctx, sampleFile, focus, true);
    }

    private record Win(Stage stage, GraphWindowController controller) {}

    private static Win build(AppContext ctx, String title) {
        try {
            FXMLLoader loader = new FXMLLoader(
                    GraphWindowController.class.getResource("/org/streamflow/ui/graph-window.fxml"));
            BorderPane root = loader.load();
            GraphWindowController c = loader.getController();
            c.ctx = ctx;
            c.applyDefaultPreferences();
            ctx.workspace().addTreeChangeListener(c::onExternalTreeChange);   // live sync across windows
            c.sample = title;
            c.plot.setChannelLabeler(ch -> ctx.aliases().label(ch));
            // App-wide Copy format: apply current settings to this plot and stay in sync as they change.
            c.applyExportFormat();
            ctx.settings().addChangeListener(c.exportFormatListener);
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
                    case ESCAPE -> { c.plot.cancelDrawing(); c.selectGateTool("None"); }
                    case DELETE, BACK_SPACE -> c.deleteSelectedPopulation();
                    case LEFT  -> c.gotoSample(c.sampleIndex - 1);
                    case RIGHT -> c.gotoSample(c.sampleIndex + 1);
                    case P -> c.selectGateTool("Polygon");
                    case R -> c.selectGateTool("Rectangle");
                    case E -> c.selectGateTool("Ellipse");
                    case I -> c.selectGateTool("Interval");
                    case Q -> c.selectGateTool("Quadrant");
                    case N -> c.selectGateTool("None");
                    default -> { }
                }
            });
            root.setOpacity(0);   // faded in by UiFx.fadeSlideIn right after each call site's stage.show()
            Stage stage = new Stage();
            stage.setTitle("StreamFLOW — " + title);
            stage.setScene(scene);
            AppIcons.apply(stage);
            stage.setOnHidden(e -> {
                ctx.settings().removeChangeListener(c.exportFormatListener);
                ctx.workspace().unregisterWindow(c.sampleFile);  // §14: deregister so focus check works
            });
            return new Win(stage, c);
        } catch (Exception e) {
            throw new RuntimeException("Could not open graph window: " + e.getMessage(), e);
        }
    }

    /** Seed per-window defaults from Settings ▸ General right after ctx is assigned (initialize()
     *  runs during FXML load, before ctx exists, so this can't live there). */
    private void applyDefaultPreferences() {
        if (ctx == null || ctx.settings() == null) return;
        boolean lb = ctx.settings().defaultLightBackground();
        lightModeCheck.setSelected(lb);
        plot.setLightMode(lb);
        boolean iv = ctx.settings().defaultInterceptLines();
        interceptCheck.setSelected(iv);
        plot.setInterceptVisible(iv);
    }

    @FXML
    public void initialize() {
        plotTypeCombo.setItems(FXCollections.observableArrayList(
                "pseudocolor", "dot", "contour", "density", "zebra", "histogram"));
        plotTypeCombo.getSelectionModel().select("pseudocolor");
        toolGroup.selectedToggleProperty().addListener((obs, o, n) -> {
            if (n == null) { toolNoneBtn.setSelected(true); return; }
            plot.setTool(toolNameOf(n));
        });
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
            PopNode en = nodeForGate(g); if (en != null) en.edited = true;   // flag for Workstation indicator
            notifyTree();   // gate moved/resized → mark workspace dirty
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
        plot.setOnApplyToAll(this::applyGateToAllSamples);
        plot.setOnLabelMoved(g -> {   // BUG-14: label position is universal per population
            ctx.workspace().setPopLabelOffset(g.name, g.lblDx, g.lblDy);
            notifyTree();
        });
        plot.setOnBusy(busy -> {                 // disable axis controls while a render runs
            // Never disable the combos while the axis-options popover is visible: the popover
            // itself triggers invalidate() (e.g. dragging the Logicle-W or zoom slider), and
            // disabling the controls at that moment causes a permanent freeze if the render is
            // cancelled before it can call setBusy(false).  The popover is lightweight enough
            // that it runs correctly against a live background render.
            if (!axisPopoverActive) {
                xAxisCombo.setDisable(busy); yAxisCombo.setDisable(busy);
                xScaleCombo.setDisable(busy); yScaleCombo.setDisable(busy);
                plotTypeCombo.setDisable(busy);
            }
        });

        // On a USER axis/scale change, apply then broadcast (notifyTree) so the gating-strategy figure
        // and any other open window re-sync to the new view. Safe from cascade: programmatic combo sets
        // during selectNode run with suppressAxisEvents=true, so these handlers don't fire there, and the
        // originating window is skipped via notifyTree's selfNotifying guard. See ui-bug-log BUG-10.
        plotTypeCombo.setOnAction(e -> { if (!suppressAxisEvents) { applyAxes(); notifyTree(); } });
        // Switching a channel adopts THAT marker's universal scale (not a per-file default).
        xAxisCombo.setOnAction(e -> { if (suppressAxisEvents) return; xScaleCombo.getSelectionModel().select(scaleForChannel(xAxisCombo.getValue())); applyAxes(); notifyTree(); });
        yAxisCombo.setOnAction(e -> { if (suppressAxisEvents) return; yScaleCombo.getSelectionModel().select(scaleForChannel(yAxisCombo.getValue())); applyAxes(); notifyTree(); });
        // Changing a scale sets it UNIVERSALLY for that marker, then broadcasts so every sample/window syncs.
        xScaleCombo.setOnAction(e -> { if (!suppressAxisEvents) { ctx.workspace().setChannelScale(xAxisCombo.getValue(), xScaleCombo.getValue()); applyAxes(); notifyTree(); } });
        yScaleCombo.setOnAction(e -> { if (!suppressAxisEvents) { ctx.workspace().setChannelScale(yAxisCombo.getValue(), yScaleCombo.getValue()); applyAxes(); notifyTree(); } });
        // tool wired via toolGroup listener above

        // real Ikonli icons (MD2 cog for axis settings, FontAwesome5 for the rest)
        iconOnly(prevSampleButton, "fas-chevron-left", "◀");
        iconOnly(nextSampleButton, "fas-chevron-right", "▶");
        iconOnly(xAxisOptsButton, "mdi2c-cog", "⚙");
        iconOnly(yAxisOptsButton, "mdi2c-cog", "⚙");
        withIcon(copyButton, "fas-copy");
        iconOnly(copySettingsButton, "fas-cog", "⚙");
        withIcon(channelsButton, "fas-tags");

        histModeCombo.setItems(FXCollections.observableArrayList("Filled Smooth", "Raw Bars", "Line Only", "CDF"));
        histModeCombo.getSelectionModel().select("Filled Smooth");
        // Histogram "Smooth" only ever affected computeHistogram's own KDE blur; it was displayed next
        // to Hist/bandwidth but used to be wired to the SAME flag as density/contour smoothing, so
        // unchecking it silently killed contour lines too. Now split: this one is histogram-only.
        smoothCheck.setOnAction(e -> plot.setSmoothHistogram(smoothCheck.isSelected()));
        // Density and Contour smoothing are now INDEPENDENT flags (a contour can be smoothed without
        // touching density/zebra and vice-versa). The mutual "only one active at a time" is handled by
        // refreshOptionGroups() disabling whichever group isn't the current plot type.
        smoothDensityCheck.setOnAction(e -> plot.setSmoothDensity(smoothDensityCheck.isSelected()));
        contourSmoothCheck.setOnAction(e -> {
            boolean on = contourSmoothCheck.isSelected();
            plot.setSmoothContour(on);
            contourSmoothSlider.setDisable(!on);   // strength only matters while smoothing is on
        });
        // Contour has its own blur-radius slider: the Density group (which owns the density/zebra
        // smoothing strength) is disabled while contour is the active plot type, so it'd be unreachable.
        contourSmoothSlider.valueProperty().addListener((o, a, b) -> {
            int s = (int) Math.round(b.doubleValue());
            contourSmoothLabel.setText("Contour smoothness: " + s);
            plot.setContourSmoothStrength(s);
        });
        contourLevelsSlider.valueProperty().addListener((o, a, b) -> {
            int n = (int) Math.round(b.doubleValue());
            contourLevelsLabel.setText("Contour lines: " + n);
            plot.setContourLevels(n);
        });
        contourOutliersCheck.setOnAction(e -> {
            boolean on = contourOutliersCheck.isSelected();
            plot.setContourOutliers(on);
            outlierSizeSlider.setDisable(!on);   // only meaningful once outliers are shown
        });
        outlierSizeSlider.valueProperty().addListener((o, a, b) -> plot.setOutlierSize((int) Math.round(b.doubleValue())));
        // Context-sensitive size/smoothing control: "Point size" for pseudocolor/dot, "Smoothing" for density plots.
        sizeControlSlider.valueProperty().addListener((o, a, b) -> applySizeControl((int) Math.round(b.doubleValue())));
        plotTypeCombo.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> { refreshSizeControl(); refreshOptionGroups(); });
        // Editable colours + palette
        paletteCombo.getItems().setAll(CytoPlot.paletteNames());
        paletteCombo.getSelectionModel().select(plot.palette());
        paletteCombo.setOnAction(e -> plot.setPalette(paletteCombo.getValue()));
        contourColorPicker.setValue(plot.contourColor());
        contourColorPicker.valueProperty().addListener((o, a, b) -> plot.setContourColor(b));
        dotColorPicker.setValue(plot.dotColor());
        dotColorPicker.valueProperty().addListener((o, a, b) -> plot.setDotColor(b));
        backgateColorPicker.setValue(plot.backgateColor());
        backgateColorPicker.valueProperty().addListener((o, a, b) -> plot.setBackgateColor(b));
        refreshSizeControl();
        refreshOptionGroups();
        snapCheck.setOnAction(e -> {
            plot.setSnapEnabled(snapCheck.isSelected());
            statusLabel.setText(snapCheck.isSelected()
                    ? "Snap on — while drawing a gate, its edges/vertices jump to the nearest density valley (pink guide)."
                    : "Snap off.");
        });
        confidenceCheck.setOnAction(e -> {
            plot.setShowConfidence(confidenceCheck.isSelected());
            statusLabel.setText(confidenceCheck.isSelected()
                    ? "Confidence on — gate fill turns green (boundary on a valley = clean) or yellow/red (boundary through dense region = overlap). Updates live as you move gates."
                    : "Confidence off — gates show their normal colors.");
        });
        backgateCheck.setOnAction(e -> {
            if (!backgateCheck.isSelected()) { plot.setHighlight(null); statusLabel.setText("Backgating off."); }
            else statusLabel.setText("Backgating on — select a descendant population in the tree to overlay it.");
        });
        // Track Shift purely so onSetFmo() (an ActionEvent, which carries no modifier keys) can tell a
        // plain click from a Shift-click. A plain click now ALWAYS (re)sets FMO from the current sample
        // — it used to silently clear an already-set value instead, which read as "the button does
        // nothing". onAction (not setOnMouseClicked) also means the button responds to Enter/Space.
        fmoButton.setOnMousePressed(e -> fmoShiftDown = e.isShiftDown());
        histModeCombo.setOnAction(e -> plot.setHistMode(histModeCombo.getValue()));
        // Apply bandwidth only when the drag settles (or on a discrete step), not on every tick —
        // per-tick re-renders during a drag were a source of the histogram render storm.
        bandwidthSlider.valueProperty().addListener((o, a, b) -> { if (!bandwidthSlider.isValueChanging()) plot.setHistBandwidth(b.doubleValue()); });
        bandwidthSlider.valueChangingProperty().addListener((o, was, changing) -> { if (!changing) plot.setHistBandwidth(bandwidthSlider.getValue()); });

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

        // "Jump" — list every currently-open Graph Window and bring the chosen one to front. Built
        // lazily on open (not maintained live) since window-open/close is infrequent relative to clicks.
        jumpButton.setOnShowing(e -> refreshJumpMenu());

        // Gate color panel: mirrors the right-click "Color…" dialog but lives in Options for quick,
        // repeated tweaks. Disabled with no live gate selected; refreshed via CytoPlot's selection hook.
        plot.setOnSelectionChanged(this::refreshGateColorControls);
        refreshGateColorControls(null);
        gateBorderColorPicker.setOnAction(e -> {
            CytoPlot.Gate g = plot.selectedGate();
            if (g != null) { g.border = gateBorderColorPicker.getValue(); plot.selectGate(g); notifyTree(); }
        });
        gateFillColorPicker.setOnAction(e -> applyGateFillFromOptions());
        gateOpacitySlider.valueProperty().addListener((o, a, b) -> { if (!gateOpacitySlider.isValueChanging()) applyGateFillFromOptions(); });
        gateOpacitySlider.valueChangingProperty().addListener((o, was, changing) -> { if (!changing) applyGateFillFromOptions(); });
        gateTextColorPicker.setOnAction(e -> {
            CytoPlot.Gate g = plot.selectedGate();
            if (g != null) { g.textColor = gateTextColorPicker.getValue(); plot.selectGate(g); notifyTree(); }
        });
        gateTextShadowCheck.setOnAction(e -> {
            CytoPlot.Gate g = plot.selectedGate();
            if (g != null) { g.textShadow = gateTextShadowCheck.isSelected(); plot.selectGate(g); notifyTree(); }
            boolean on = gateTextShadowCheck.isSelected() && plot.selectedGate() != null;
            gateShadowColorPicker.setDisable(!on);
            gateShadowOpacitySlider.setDisable(!on);
        });
        gateShadowColorPicker.setOnAction(e -> {
            CytoPlot.Gate g = plot.selectedGate();
            if (g != null) { g.shadowColor = gateShadowColorPicker.getValue(); plot.selectGate(g); notifyTree(); }
        });
        gateShadowOpacitySlider.valueProperty().addListener((o, a, b) -> {
            CytoPlot.Gate g = plot.selectedGate();
            if (g != null) { g.shadowOpacity = b.doubleValue(); plot.selectGate(g); notifyTree(); }
        });

        // X/Y intercept lines (FlowJo-style draggable crosshair). On enable, the crosshair is placed at
        // the VISUAL CENTRE of the current view (axis/scale-aware, not a raw clamped 0), and the fields
        // are seeded from that. Dragging a line updates the fields live; typing still works (clamped).
        interceptCheck.setOnAction(e -> {
            boolean on = interceptCheck.isSelected();
            plot.setInterceptVisible(on);
            if (on) { plot.centerIntercept(); syncInterceptFields(); }
        });
        interceptXField.setOnAction(e -> commitInterceptX());
        interceptXField.focusedProperty().addListener((o, was, isNow) -> { if (!isNow) commitInterceptX(); });
        interceptYField.setOnAction(e -> commitInterceptY());
        interceptYField.focusedProperty().addListener((o, was, isNow) -> { if (!isNow) commitInterceptY(); });
        plot.setOnInterceptChanged((x, y) -> syncInterceptFields());   // drag → fields track live
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
        xScaleCombo.getSelectionModel().select(scaleForChannel(xAxisCombo.getValue()));
        yScaleCombo.getSelectionModel().select(scaleForChannel(yAxisCombo.getValue()));
        rootNode = ctx.workspace().treeFor(sampleFile);   // shared, persistent gating tree
        rebuildTree();
        resolveSubsamples();   // set subsample row indices for this sample (cached sync, else async)
        recomputeCounts();
        PopNode focus = (initialFocus != null && isInTree(initialFocus)) ? initialFocus : rootNode;
        selectNode(focus);
    }

    private boolean isInTree(PopNode n) {
        return rootNode != null && rootNode.selfAndDescendants().contains(n);
    }

    /** Navigate an ALREADY-OPEN window to a population (used when openChild reuses this window). */
    void navigateTo(PopNode focus) {
        if (rootNode == null) { initialFocus = focus; return; }   // not loaded yet; useData will honour it
        PopNode target = (focus != null && isInTree(focus)) ? focus : rootNode;
        selectNode(target);
    }

    /** Navigating to another sample: switch to THAT sample's own gating tree, keep axes/scales,
     *  and stay on the same population (matched by name path) so you can scroll P1 across samples. */
    private void reapplyData(EventData d) {
        rootData = d;
        List<String> keepPath = currentNode != null ? pathNames(currentNode) : new ArrayList<>();
        // Keep the axes the user is currently viewing so Prev/Next stays on the SAME plot (FlowJo-style)
        // instead of snapping back to each node's own gating axes. See ui-bug-log BUG-19.
        String keepX = xAxisCombo.getValue(), keepY = yAxisCombo.getValue(),
               keepXs = xScaleCombo.getValue(), keepYs = yScaleCombo.getValue();
        rootNode = ctx.workspace().treeFor(sampleFile);   // this sample's own persistent tree
        rebuildTree();
        resolveSubsamples();   // set subsample row indices for this sample (cached sync, else async)
        recomputeCounts();
        PopNode target = resolveByNames(rootNode, keepPath);
        selectNode(target != null ? target : rootNode);
        restoreAxes(keepX, keepY, keepXs, keepYs);
        refreshTreeLabels();
    }

    /** Re-select the given axes/scales (if this sample has them) and re-render — used to carry the user's
     *  chosen view across Prev/Next navigation instead of resetting to the node's default axes. */
    private void restoreAxes(String x, String y, String xs, String ys) {
        if (x == null || !xAxisCombo.getItems().contains(x)) return;
        suppressAxisEvents = true;
        xAxisCombo.getSelectionModel().select(x);
        if (y != null && yAxisCombo.getItems().contains(y)) yAxisCombo.getSelectionModel().select(y);
        if (xs != null) xScaleCombo.getSelectionModel().select(xs);
        if (ys != null) yScaleCombo.getSelectionModel().select(ys);
        suppressAxisEvents = false;
        applyAxes();
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
        String ax = node.viewX, ay = node.viewY;   // scale is universal per marker (below), not per-node
        // BUG-06: viewX is transient (wiped on workspace reload). When it's null, prefer the axes the
        // CHILD gates were drawn on — otherwise the children don't render (they live on different axes
        // than this node's own gate). Only fall back to this node's own gate axes if it has no children.
        // This ordering (viewX → child-gate → own-gate) MUST match ExportController.makeStepCell.
        if (ax == null && !node.children.isEmpty() && node.children.get(0).gate != null
                && node.children.get(0).gate.xChan != null) {
            CytoPlot.Gate g0 = node.children.get(0).gate;
            ax = g0.xChan; ay = g0.yChan;
        }
        if (ax == null && !node.isRoot() && node.gate != null) {
            ax = node.gate.xChan;
            ay = node.gate.yChan;
        }
        if (ax != null) {
            suppressAxisEvents = true;
            if (xAxisCombo.getItems().contains(ax)) xAxisCombo.getSelectionModel().select(ax);
            if (ay != null && yAxisCombo.getItems().contains(ay)) yAxisCombo.getSelectionModel().select(ay);
            // Scale is UNIVERSAL per marker (BUG-11): always derive from the channel, so navigating
            // Prev/Next or across populations never reverts a marker's scale to a per-file default.
            xScaleCombo.getSelectionModel().select(scaleForChannel(ax));
            if (ay != null) yScaleCombo.getSelectionModel().select(scaleForChannel(ay));
            suppressAxisEvents = false;
        }
        plot.clearGates();
        currentData = dataForNode(node);
        plot.setData(currentData);                 // also clears any backgate highlight
        // Subsample children are tree populations, not drawn shapes (no geometry) — don't add them as gates.
        for (PopNode ch : node.children) if (!"subsample".equals(ch.gate.type)) plot.addGate(ch.gate);
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
            // Universal stats-displayed (BUG-13/18): a population's own override wins; otherwise the
            // global default (last-chosen stats) applies to every gate + child until the user changes it.
            List<String> cfg = ctx.workspace().popStatConfig(n.name());
            if (cfg == null) cfg = ctx.workspace().defaultStatConfig();
            if (cfg != null) { n.gate.statKeys.clear(); n.gate.statKeys.addAll(cfg); }
            // Universal label position (BUG-14): keep the label where the user dragged it, on every sample.
            double[] off = ctx.workspace().popLabelOffset(n.name());
            if (off != null) { n.gate.lblDx = off[0]; n.gate.lblDy = off[1]; }
            n.gate.statLine = statLineFor(n);
        }
    }

    /** For every subsample population in this sample's tree, set its per-sample row indices: cached
     *  ones apply synchronously; uncached ones are fetched from the engine (FlowKit selection) and
     *  applied when they arrive, re-running counts + re-rendering the current view. Called on load. */
    private void resolveSubsamples() {
        if (rootNode == null || rootData == null) return;
        List<CytoPlot.Gate> pending = new ArrayList<>();
        for (PopNode n : rootNode.selfAndDescendants()) {
            CytoPlot.Gate g = n.gate;
            if (g == null || !"subsample".equals(g.type)) continue;
            int[] cached = g.subBySample.get(sampleFile);
            if (cached != null) g.subSelected = cached; else pending.add(g);
        }
        for (CytoPlot.Gate g : pending) {
            ObjectNode args = JSON.createObjectNode();
            args.put("sample", sampleFile);
            args.put("n", g.subN);
            args.put("seed", g.subSeed);
            args.put("total", rootData.rows());
            Task<JsonNode> task = ctx.bridge().command("subsample", args);
            task.setOnSucceeded(e -> {
                int[] idx = toIntArray(task.getValue().path("indices"));
                g.subBySample.put(sampleFile, idx);
                g.subSelected = idx;
                recomputeCounts(); refreshTreeLabels();
                if (currentNode != null) { currentData = dataForNode(currentNode); plot.setData(currentData); }
            });
            Thread t = new Thread(task, "subsample"); t.setDaemon(true); t.start();
        }
    }

    private static int[] toIntArray(JsonNode arr) {
        if (arr == null || !arr.isArray()) return new int[0];
        int[] out = new int[arr.size()];
        for (int i = 0; i < out.length; i++) out[i] = arr.get(i).asInt();
        return out;
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
    /** Right-click a gate → "Apply gate → all samples": clone this population's subtree onto every other
     *  sample's tree, so a gate drawn/adjusted here propagates everywhere. Clears the "edited" flag. */
    private void applyGateToAllSamples(CytoPlot.Gate g) {
        PopNode n = nodeForGate(g);
        if (n == null) return;
        List<String> others = ctx.workspace().sampleNames().stream()
                .filter(s -> !s.equals(sampleFile)).toList();
        if (others.isEmpty()) { statusLabel.setText("Only one sample — nothing to apply to."); return; }
        for (String o : others) {
            PopNode root = ctx.workspace().treeFor(o);
            // Replace an existing same-named top-level gate instead of stacking a duplicate (BUG-15).
            root.children.removeIf(c -> java.util.Objects.equals(c.name(), n.name()));
            root.children.add(n.cloneTree(root));
        }
        n.edited = false;
        notifyTree();
        statusLabel.setText("Applied gate '" + n.name() + "' to " + others.size() + " other sample(s).");
    }

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

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Statistics — " + g.name); dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(gp);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.APPLY, ButtonType.CLOSE);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, currentStage());
        java.util.Optional<ButtonType> res = dlg.showAndWait();
        if (res.isEmpty() || res.get() != ButtonType.APPLY) return;   // Close/✕ = cancel, don't change

        List<String> keys = new ArrayList<>();
        if (parent.isSelected()) keys.add("parent");
        if (total.isSelected()) keys.add("total");
        if (count.isSelected()) keys.add("count");
        String ch = chan.getValue() == null ? g.xChan : chan.getValue();
        if (mfi.isSelected()) keys.add("mfi:" + ch);
        if (gmean.isSelected()) keys.add("geomean:" + ch);
        if (cvBox.isSelected()) keys.add("cv:" + ch);
        // Allow ZERO stats (label shows just the gate name). Do NOT force "parent" back — that made
        // "% of parent" impossible to remove and re-checked itself on reopen. See ui-bug-log BUG-12.
        g.statKeys.clear(); g.statKeys.addAll(keys);
        ctx.workspace().setPopStatConfig(g.name, keys);      // this gate's own override
        ctx.workspace().setDefaultStatConfig(keys);          // BUG-18: also the new universal default for all gates
        PopNode n = nodeForGate(g);
        if (n != null) g.statLine = statLineFor(n);
        plot.selectGate(g);   // repaint label
        notifyTree();         // sync the label change to the export figure + other windows
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
        if (n != null) openChildForNode(n, true);
    }

    private void openChildForNode(PopNode node) { openChildForNode(node, true); }

    private void openChildForNode(PopNode node, boolean stealFocus) {
        if (node == null || node.isRoot()) return;
        openChild(ctx, sampleFile, node, stealFocus);
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
        applyUniversalRange(hist);   // BUG-17: lock each marker's axis range so gates don't shift across samples
        // Persist view config so the gating-strategy figure always reflects the current axes/scale.
        if (currentNode != null) {
            currentNode.viewX = xAxisCombo.getValue();
            currentNode.viewY = hist ? null : yAxisCombo.getValue();
            currentNode.viewXScale = xScaleCombo.getValue();
            currentNode.viewYScale = yScaleCombo.getValue();
        }
    }

    /** Lock the shown channels to their UNIVERSAL range (first sample to display a marker sets it from its
     *  All-Events data; every later sample reuses it), so a gate at fixed coordinates lands in the same
     *  place on every sample. See ui-bug-log BUG-17. */
    private void applyUniversalRange(boolean hist) {
        if (ctx == null || rootData == null) return;
        String xch = xAxisCombo.getValue();
        if (xch != null) {
            double[] r = ctx.workspace().channelRange(xch);
            if (r == null) {
                int xc = rootData.indexOf(xch);
                if (xc >= 0) { double[] dr = rootData.range(xc); ctx.workspace().setChannelRange(xch, dr[0], dr[1]); r = ctx.workspace().channelRange(xch); }
            }
            if (r != null) { plot.setXMin(r[0]); plot.setXMax(r[1]); }
        }
        String ych = hist ? null : yAxisCombo.getValue();
        if (ych != null) {
            double[] r = ctx.workspace().channelRange(ych);
            if (r == null) {
                int yc = rootData.indexOf(ych);
                if (yc >= 0) { double[] dr = rootData.range(yc); ctx.workspace().setChannelRange(ych, dr[0], dr[1]); r = ctx.workspace().channelRange(ych); }
            }
            if (r != null) { plot.setYMin(r[0]); plot.setYMax(r[1]); }
        }
    }

    /** Push the stored FMO levels (if any) for the current channels onto the plot. */
    private void applyFmoLines(boolean hist) {
        if (ctx == null) return;
        double fx = ctx.fmo().level(xAxisCombo.getValue());
        double fy = hist ? Double.NaN : ctx.fmo().level(yAxisCombo.getValue());
        plot.setFmo(fx, fy);
    }

    /** Set-FMO toolbar button: plain click (re)sets from the current sample; Shift-click clears. */
    @FXML
    private void onSetFmo() {
        if (ctx == null) return;
        String xch = xAxisCombo.getValue();
        boolean alreadySet = xch != null && ctx.fmo().has(xch);
        // Plain click TOGGLES: sets the FMO reference (lines appear) if none is set for the current
        // channel(s), or clears it (lines disappear) if one already is. Shift-click always clears.
        // The Options ▸ "Show FMO lines" checkbox is a SEPARATE, independent visibility toggle that
        // hides/shows the lines without discarding the stored level — do not conflate the two.
        if (fmoShiftDown || alreadySet) {
            if (xch != null) ctx.fmo().clear(xch);
            if (yAxisCombo.getValue() != null) ctx.fmo().clear(yAxisCombo.getValue());
            applyFmoLines(HIST.equals(yAxisCombo.getValue()));
            statusLabel.setText("FMO reference cleared for current channel(s).");
        } else {
            setFmoFromCurrent();
        }
    }

    /** Populate the "Jump" menu with every other currently-open Graph Window. */
    private void refreshJumpMenu() {
        jumpButton.getItems().clear();
        if (ctx == null) return;
        for (var entry : ctx.workspace().openWindowsView().entrySet()) {
            javafx.stage.Stage st = entry.getValue();
            if (st == null || !st.isShowing() || entry.getKey().equals(sampleFile)) continue;
            MenuItem mi = new MenuItem(entry.getKey());
            mi.setOnAction(ev -> { st.toFront(); st.requestFocus(); });
            jumpButton.getItems().add(mi);
        }
        if (jumpButton.getItems().isEmpty()) {
            MenuItem none = new MenuItem("(no other open windows)");
            none.setDisable(true);
            jumpButton.getItems().add(none);
        }
    }

    /** Options panel "Gate color" section: reflects/edits whichever gate is currently selected. */
    private void refreshGateColorControls(CytoPlot.Gate g) {
        boolean has = g != null;
        gateBorderColorPicker.setDisable(!has);
        gateFillColorPicker.setDisable(!has);
        gateOpacitySlider.setDisable(!has);
        gateTextColorPicker.setDisable(!has);
        gateTextShadowCheck.setDisable(!has);
        boolean shadowOn = has && g.textShadow;
        gateShadowColorPicker.setDisable(!shadowOn);   // shadow color/transparency only when shadow is on
        gateShadowOpacitySlider.setDisable(!shadowOn);
        if (!has) return;
        gateBorderColorPicker.setValue(g.border);
        gateFillColorPicker.setValue(opaque(g.fill));
        gateOpacitySlider.setValue(g.fill.getOpacity());
        gateTextColorPicker.setValue(g.textColor != null ? g.textColor : g.border);
        gateTextShadowCheck.setSelected(g.textShadow);
        gateShadowColorPicker.setValue(g.shadowColor);
        gateShadowOpacitySlider.setValue(g.shadowOpacity);
    }

    private void applyGateFillFromOptions() {
        CytoPlot.Gate g = plot.selectedGate();
        if (g == null) return;
        Color fc = gateFillColorPicker.getValue();
        g.fill = Color.color(fc.getRed(), fc.getGreen(), fc.getBlue(), gateOpacitySlider.getValue());
        plot.selectGate(g);
        notifyTree();
    }

    /** Clamp typed text to [lo,hi] (swapped if inverted); invalid text reverts to the current value. */
    private static double parseClamp(String s, double lo, double hi, double fallback) {
        double v;
        try { v = Double.parseDouble(s.trim()); } catch (Exception e) { return fallback; }
        if (hi < lo) { double t = lo; lo = hi; hi = t; }
        return Math.max(lo, Math.min(hi, v));
    }
    private static String fmtNum(double v) { return String.format("%.2f", v); }

    private void commitInterceptX() {
        double v = parseClamp(interceptXField.getText(), plot.xMin(), plot.xMax(), plot.interceptX());
        interceptXField.setText(fmtNum(v));
        plot.setInterceptX(v);
    }
    private void commitInterceptY() {
        double v = parseClamp(interceptYField.getText(), plot.yMin(), plot.yMax(), plot.interceptY());
        interceptYField.setText(fmtNum(v));
        plot.setInterceptY(v);
    }
    /** Mirror the plot's current intercept position into the number fields (after centring or a drag). */
    private void syncInterceptFields() {
        interceptXField.setText(fmtNum(plot.interceptX()));
        interceptYField.setText(fmtNum(plot.interceptY()));
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

    /** UNIVERSAL scale for a marker: the workspace-wide choice for this channel if the user has set
     *  one, else the sensible default. This is what keeps a marker's scale identical across every
     *  sample (incl. Prev/Next navigation) and in the export figure. See ui-bug-log BUG-11. */
    private String scaleForChannel(String channel) {
        if (channel == null) return "Linear";
        String s = ctx != null ? ctx.workspace().channelScale(channel) : null;
        return s != null ? s : defaultScale(channel);
    }

    private static CytoPlot.Scale scaleOf(String s) {
        if ("Log".equals(s)) return CytoPlot.Scale.LOG;
        if ("Logicle".equals(s)) return CytoPlot.Scale.LOGICLE;
        if ("ArcSinh".equals(s)) return CytoPlot.Scale.ARCSINH;
        return CytoPlot.Scale.LINEAR;
    }

    // ---- axis adjustment popover (icon next to each axis) --------------------

    private PopOver axisPopOver;
    /** True while an axis-options popover is showing — prevents setOnBusy from disabling
     *  the axis combos and locking the user out of the popover during renders it triggered. */
    private boolean axisPopoverActive = false;

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
        // Revert just the range (undo Extend) without resetting the transform width like Auto does.
        Button resetRange = new Button("Reset range");
        resetRange.setOnAction(ev -> { if (xAxis) plot.resetXRange(); else plot.resetYRange(); });
        HBox extend = new HBox(6, extMinus, extPlus, resetRange);
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
        // Track popover visibility so setOnBusy doesn't disable controls during axis tweaks.
        axisPopoverActive = true;
        pop.showingProperty().addListener((obs, wasShowing, isShowing) -> {
            if (!isShowing) {
                axisPopoverActive = false;
                // Re-enable any combos that got stuck disabled while the popover was open.
                boolean busy = false;   // popover just closed — no render should have them disabled
                xAxisCombo.setDisable(busy); yAxisCombo.setDisable(busy);
                xScaleCombo.setDisable(busy); yScaleCombo.setDisable(busy);
                plotTypeCombo.setDisable(busy);
            }
        });
        pop.show(anchor);
        axisPopOver = pop;
        // ControlsFX PopOver applies a DropShadow in its skin's Java code (not CSS). When the
        // popover is showing, every 60Hz pulse computes the shadow extent → reads the PopOver's
        // VLineTo-based arrow path geometry → fires binding invalidations → marks layout dirty →
        // next pulse: 28% FX thread CPU saturation. Clear all effects after the skin is created.
        Platform.runLater(() -> clearPopOverEffects(pop));
    }

    private static void clearPopOverEffects(javafx.stage.PopupWindow pop) {
        if (pop.getScene() == null || pop.getScene().getRoot() == null) return;
        clearNodeEffects(pop.getScene().getRoot());
    }

    private static void clearNodeEffects(javafx.scene.Node node) {
        node.setEffect(null);
        if (node instanceof javafx.scene.Parent p) {
            for (javafx.scene.Node child : p.getChildrenUnmodifiable()) clearNodeEffects(child);
        }
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

    private void refreshCombo(ComboBox<String> combo) {
        String sel = combo.getValue();
        javafx.collections.ObservableList<String> items = javafx.collections.FXCollections.observableArrayList(combo.getItems());
        combo.setItems(items);
        combo.setValue(sel);
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
        AppIcons.theme(dlg, currentStage());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;
        for (int i = 0; i < chans.size(); i++) ctx.aliases().set(chans.get(i), fields.get(i).getText());
        refreshCombo(xAxisCombo); refreshCombo(yAxisCombo);
        recomputeCounts(); refreshTreeLabels();
        applyAxes();   // redraw plot axis labels with new aliases
        statusLabel.setText("Channel target names updated.");
    }

    /** Listener so this plot tracks the app-wide Copy format live (held for clean removal on close). */
    private final Runnable exportFormatListener = this::applyExportFormat;

    /** Push the current app-wide Copy/export format (point size, axis & label fonts) onto this plot. */
    private void applyExportFormat() {
        if (ctx == null) return;
        plot.setPointRadius(ctx.settings().exportPointSize());
        plot.setAxisFontSize(ctx.settings().exportAxisFontSize());
        plot.setLabelFontSize(ctx.settings().exportFontSize());
        if (sizeControlSlider != null) refreshSizeControl();
    }

    private boolean isDensityPlotType() {
        String t = plotTypeCombo.getValue();
        return "contour".equals(t) || "density".equals(t) || "zebra".equals(t);
    }

    /** Enable only the Options group relevant to the current plot type: Histogram for histograms,
     *  Density for pseudocolor/dot/density/zebra, Contour for contour. A disabled group is greyed and
     *  collapsed. This also gives the "density smooth vs contour smooth are never both active" behaviour. */
    private void refreshOptionGroups() {
        if (histGroup == null) return;
        String t = plotTypeCombo.getValue();
        boolean hist = "histogram".equals(t);
        boolean contour = "contour".equals(t);
        boolean density = "pseudocolor".equals(t) || "dot".equals(t) || "density".equals(t) || "zebra".equals(t);
        setGroupActive(histGroup, hist);
        setGroupActive(densityGroup, density);
        setGroupActive(contourGroup, contour);
    }
    private static void setGroupActive(javafx.scene.control.TitledPane pane, boolean active) {
        if (pane == null) return;
        pane.setDisable(!active);
        if (!active) pane.setExpanded(false);
    }

    /** Point-size for pseudocolor/dot; smoothing strength for density-based plots. */
    private void applySizeControl(int v) {
        if (isDensityPlotType()) { plot.setSmoothStrength(v); sizeControlLabel.setText("Smoothing: " + plot.smoothStrength()); }
        else { plot.setPointRadius(v); sizeControlLabel.setText("Point size: " + plot.getPointRadius()); }
    }

    /** Re-point the shared size slider at the current plot type's parameter (label + value + range). */
    private void refreshSizeControl() {
        if (sizeControlSlider == null) return;
        if (isDensityPlotType()) {
            sizeControlSlider.setMax(8);
            sizeControlSlider.setValue(plot.smoothStrength());
            sizeControlLabel.setText("Smoothing: " + plot.smoothStrength());
        } else {
            sizeControlSlider.setMax(10);
            sizeControlSlider.setValue(plot.getPointRadius());
            sizeControlLabel.setText("Point size: " + plot.getPointRadius());
        }
    }

    /** Copy button — copy the plot straight to the clipboard using the current app-wide format. */
    @FXML
    private void onCopy() {
        boolean prevLabels = plot.labelsVisible();
        plot.setLabelsVisible(ctx.settings().exportGateLabels());
        javafx.scene.image.WritableImage img = plot.exportImage(ctx.settings().exportScale());
        plot.setLabelsVisible(prevLabels);
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied " + ctx.settings().exportDpi() + " DPI plot image — paste into PowerPoint."
                + " (Adjust format with the gear button.)");
    }

    /** Gear button beside Copy — open the app-wide Copy/Export format window. */
    @FXML
    private void onCopySettings() {
        CopySettingsController.open(ctx.settings());
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
            UiFx.hoverPulse(b);
        } catch (Exception e) { b.setText(fallback); }
    }
    private static void withIcon(javafx.scene.control.Labeled b, String literal) {
        try {
            org.kordamp.ikonli.javafx.FontIcon fi = new org.kordamp.ikonli.javafx.FontIcon(literal);
            fi.setIconSize(14); fi.setIconColor(ICON_C);
            b.setGraphic(fi);
            UiFx.hoverPulse(b);
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
        AppIcons.theme(d, currentStage());
        Optional<String> r = d.showAndWait();
        if (r.isEmpty() || r.get().isBlank()) { selectGateTool("None"); return; }
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
        selectGateTool("None"); // one gate per draw (FlowJo)
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
        openChildForNode(node, false);   // auto-open without stealing focus from the drawing window
    }

    private void onQuadrantDrawn(CytoPlot.Gate qg) {
        TextInputDialog d = new TextInputDialog("Q");
        d.setTitle("Quadrant gate prefix"); d.setHeaderText(null);
        d.setContentText("Prefix for Q1/Q2/Q3/Q4 (e.g. 'Annex' → Annex Q1 … Q4):");
        AppIcons.theme(d, currentStage());
        Optional<String> r = d.showAndWait();
        if (r.isEmpty() || r.get().isBlank()) { selectGateTool("None"); return; }
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
                    java.util.Arrays.copyOf(qg.xs, qg.xs.length),
                    java.util.Arrays.copyOf(qg.ys, qg.ys.length));
            subGate.border = qg.border;
            PopNode node = new PopNode(subGate, currentNode);
            currentNode.children.add(node);
            plot.addGate(subGate);
            if (parentItem != null) { parentItem.getChildren().add(buildItem(node)); }
        }
        if (parentItem != null) parentItem.setExpanded(true);
        recomputeCounts();
        refreshTreeLabels();
        selectGateTool("None");
        notifyTree();
        statusLabel.setText("Quadrant gate '" + prefix + "' added (Q1–Q4).");
        ctx.auditLog().add(AuditLog.Type.GATE, sampleFile,
                String.format("Quadrant '%s' (Q1–Q4 on %s / %s)", prefix, qg.xChan, qg.yChan));
        // §13 undo: remove all 4 quadrant nodes; redo puts them all back
        final PopNode addedParent = currentNode;
        final List<PopNode> quadNodes = new ArrayList<>(addedParent.children.subList(
                addedParent.children.size() - 4, addedParent.children.size()));
        final List<Integer> quadIdxs = new ArrayList<>();
        for (PopNode qn : quadNodes) quadIdxs.add(addedParent.children.indexOf(qn));
        pushUndo(
            () -> { suppressUndo = true; try { for (PopNode qn : quadNodes) removeNode(qn); } finally { suppressUndo = false; } },
            () -> { suppressUndo = true; try { for (int i = 0; i < quadNodes.size(); i++) reAddNode(quadNodes.get(i), addedParent, quadIdxs.get(i)); } finally { suppressUndo = false; } }
        );
        for (PopNode qn : quadNodes) openChildForNode(qn, false);   // auto-open without stealing focus
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
        AppIcons.theme(dlg, currentStage());

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
        AppIcons.apply(stage);
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

    // ---- gate tool helpers --------------------------------------------------

    /** Programmatically select a gate tool by name (None/Polygon/Rectangle/Ellipse/Interval/Quadrant). */
    void selectGateTool(String name) {
        ToggleButton btn = switch (name) {
            case "Polygon"   -> toolPolyBtn;
            case "Rectangle" -> toolRectBtn;
            case "Ellipse"   -> toolEllipseBtn;
            case "Interval"  -> toolIntervalBtn;
            case "Quadrant"  -> toolQuadBtn;
            default          -> toolNoneBtn;
        };
        btn.setSelected(true);
    }

    private String toolNameOf(Toggle t) {
        if (t == toolPolyBtn)     return "Polygon";
        if (t == toolRectBtn)     return "Rectangle";
        if (t == toolEllipseBtn)  return "Ellipse";
        if (t == toolIntervalBtn) return "Interval";
        if (t == toolQuadBtn)     return "Quadrant";
        return "None";
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
        AppIcons.theme(d, currentStage());
        Optional<String> r = d.showAndWait();
        if (r.isEmpty() || r.get().isBlank()) return;
        final String oldName = g.name;
        final String newName = r.get().trim();
        g.name = newName;
        plot.selectGate(g);
        refreshTreeLabels();
        pushUndo(
            () -> { g.name = oldName; plot.selectGate(g); refreshTreeLabels(); notifyTree(); },
            () -> { g.name = newName; plot.selectGate(g); refreshTreeLabels(); notifyTree(); }
        );
    }

    /** Per-gate border/fill colour + fill opacity (context menu → Color…). */
    private void pickGateColor(CytoPlot.Gate g) {
        final Color oldBorder = g.border;
        final Color oldFill   = g.fill;

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
        AppIcons.theme(dlg, currentStage());
        dlg.showAndWait();

        final Color newBorder = g.border;
        final Color newFill   = g.fill;
        if (!newBorder.equals(oldBorder) || !newFill.equals(oldFill)) {
            pushUndo(
                () -> { g.border = oldBorder; g.fill = oldFill; plot.selectGate(g); },
                () -> { g.border = newBorder; g.fill = newFill; plot.selectGate(g); }
            );
        }
    }

    private static Color opaque(Color c) { return Color.color(c.getRed(), c.getGreen(), c.getBlue()); }

    private static void selectPreferred(ComboBox<String> combo, List<String> channels, String pref) {
        String m = channels.stream().filter(c -> c.equalsIgnoreCase(pref)).findFirst()
                .orElse(channels.isEmpty() ? null : channels.get(0));
        if (m != null) combo.getSelectionModel().select(m);
    }
}
