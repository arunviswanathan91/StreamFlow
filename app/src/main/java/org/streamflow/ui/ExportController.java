package org.streamflow.ui;

import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.geometry.Insets;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ContextMenu;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.Menu;
import javafx.scene.control.MenuItem;
import javafx.scene.control.ScrollPane;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.SeparatorMenuItem;
import javafx.scene.control.Slider;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.TextArea;
import javafx.scene.control.ToggleButton;
import javafx.scene.control.cell.CheckBoxTableCell;
import javafx.scene.input.MouseButton;
import javafx.scene.layout.Pane;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.scene.shape.Line;
import javafx.scene.shape.Polygon;
import javafx.stage.FileChooser;

import java.io.File;
import java.nio.file.Files;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;

/**
 * Export module — Tab 1 (Data Export): build a tidy statistics table (one row per sample × gated
 * population, columns = the chosen statistics: % of parent/total, count, median MFI / geometric mean
 * / CV per channel) from the workspace gating trees, loading sample events on demand, then copy it
 * as TSV (paste into Excel/Prism) or save CSV. Tabs 2–3 (Graph Export, Gating-Strategy figure) follow.
 */
public class ExportController implements ContextAware, Refreshable {

    @FXML private Button selAllButton, selNoneButton, computeButton, copyTsvButton, saveCsvButton;
    @FXML private TableView<SampleSel> sampleTable;
    @FXML private TableColumn<SampleSel, Boolean> useCol;
    @FXML private TableColumn<SampleSel, String> nameCol;
    @FXML private CheckBox statParent, statTotal, statCount, statMedian, statGeoMean, statCV;
    @FXML private ListView<String> channelList;
    @FXML private TableView<String[]> dataTable;
    @FXML private Label dataStatusLabel;

    // ---- Graph Export tab (superimposed histograms) ----
    @FXML private javafx.scene.control.ComboBox<String> gePopCombo, geChannelCombo, geScaleCombo;
    @FXML private CheckBox geModalCheck;
    @FXML private Button geSelAllButton, geSelNoneButton, gePlotButton, geCopyButton, geSavePngButton;
    @FXML private TableView<SampleSel> geSampleTable;
    @FXML private TableColumn<SampleSel, Boolean> geUseCol;
    @FXML private TableColumn<SampleSel, String> geNameCol;
    @FXML private AnalysisChart geChart;
    @FXML private Label geStatusLabel;
    private final ObservableList<SampleSel> geSamples = FXCollections.observableArrayList();

    // ---- Gating Strategy canvas tab ----
    @FXML private javafx.scene.control.ComboBox<String> gsSampleCombo;
    @FXML private Button gsBuildButton, gsCopyButton, gsSavePngButton;
    @FXML private Button gsAddTextButton, gsZoomFitButton, gsClearCanvasButton;
    @FXML private ToggleButton gsConnectBtn;
    @FXML private javafx.scene.control.ComboBox<String> gsArrowStyleCombo;
    @FXML private Pane gsCanvas;
    @FXML private ScrollPane gsScrollPane;
    @FXML private Label gsStatusLabel;

    // ---- Layout Editor tab (drag-drop plot tiles + batch multi-page PDF) ----
    @FXML private javafx.scene.control.ComboBox<String> layoutSampleCombo;
    @FXML private javafx.scene.control.TreeView<PopNode> layoutTree;
    @FXML private javafx.scene.control.Spinner<Integer> layoutColsSpinner, layoutRowsSpinner;
    @FXML private CheckBox layoutPaginateCheck;
    @FXML private Button layoutSavePdfButton, layoutBatchButton, layoutClearButton;
    @FXML private Pane layoutCanvas;
    @FXML private ScrollPane layoutScroll;
    @FXML private Label layoutStatusLabel;
    /** Each dropped tile is a blueprint: the population's name-path from root, re-resolved per sample
     *  at render/batch time. Tiles are auto-arranged into the Cols×Rows grid. */
    private final List<java.util.List<String>> layoutTiles = new ArrayList<>();
    private static final javafx.scene.input.DataFormat POP_PATH = new javafx.scene.input.DataFormat("streamflow/pop-path");
    private static final double L_CELL_W = 240, L_CELL_H = 270, L_GAP = 18, L_MARGIN = 24;

    // canvas state
    private final List<VBox> gsCells = new ArrayList<>();
    private final java.util.IdentityHashMap<VBox, PopNode> cellPopMap = new java.util.IdentityHashMap<>();
    private final java.util.IdentityHashMap<VBox, CytoPlot> cellPlotMap = new java.util.IdentityHashMap<>();
    private static final class GsLink {
        VBox from, to;
        GsLink(VBox from, VBox to) { this.from = from; this.to = to; }
    }
    private final List<GsLink> gsLinks = new ArrayList<>();
    private GsLink gsSelectedArrow = null;
    private final Map<GsLink, List<javafx.scene.Node>> arrowShapesByLink = new java.util.HashMap<>();
    private boolean gsConnectMode = false;
    private VBox gsConnectSource = null;
    private String currentGsSample = null;
    private javafx.scene.Node gsSelected = null;
    private final List<javafx.scene.shape.Rectangle> gsHandles = new ArrayList<>();
    private javafx.scene.shape.Rectangle gsSelectionOutline = null;
    private final java.util.IdentityHashMap<VBox, Boolean> gsCellBorder = new java.util.IdentityHashMap<>();
    private final java.util.IdentityHashMap<TextArea, TextStyleState> textStyles = new java.util.IdentityHashMap<>();

    private static final class TextStyleState {
        boolean bold, italic;
        boolean showBorder = false;   // PowerPoint-style: no border by default, toggle on demand
        double fontSize = 13;
        Color color = Color.BLACK;
        String toCss() {
            // Explicit values with units so the inline style reliably beats the .text-area stylesheet
            // rules (monospace font, dark bg). The .content dark fill is cleared via the .gs-textbox CSS
            // class instead — inline styles cannot reach that sub-node. See ui-bug-log BUG-07.
            return "-fx-background-color:transparent; -fx-control-inner-background:transparent;"
                    + (showBorder ? " -fx-border-color:#AAAAAA; -fx-border-width:1;"
                                  : " -fx-border-color:transparent; -fx-border-width:1;")
                    + " -fx-font-family:'System';"
                    + " -fx-font-size:" + (int) Math.round(fontSize) + "px;"
                    + " -fx-font-weight:" + (bold ? "bold" : "normal") + ";"
                    + " -fx-font-style:" + (italic ? "italic" : "normal") + ";"
                    + " -fx-text-fill:" + toWeb(color) + ";";
        }
        private static String toWeb(Color c) {
            return String.format("#%02X%02X%02X", (int) Math.round(c.getRed() * 255),
                    (int) Math.round(c.getGreen() * 255), (int) Math.round(c.getBlue() * 255));
        }
    }

    private final ObservableList<SampleSel> samples = FXCollections.observableArrayList();
    private final ObservableList<String[]> rows = FXCollections.observableArrayList();
    private List<String> headers = new ArrayList<>();
    private AppContext ctx;

    public static final class SampleSel {
        final String name;
        final SimpleBooleanProperty use = new SimpleBooleanProperty(true);
        SampleSel(String name) { this.name = name; }
    }

    @FXML
    public void initialize() {
        useCol.setCellValueFactory(c -> c.getValue().use);
        useCol.setCellFactory(CheckBoxTableCell.forTableColumn(useCol));
        useCol.setEditable(true);
        nameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(shortName(c.getValue().name)));
        sampleTable.setItems(samples);
        sampleTable.setEditable(true);
        channelList.getSelectionModel().setSelectionMode(SelectionMode.MULTIPLE);
        dataTable.setItems(rows);

        // Graph Export tab
        geUseCol.setCellValueFactory(c -> c.getValue().use);
        geUseCol.setCellFactory(CheckBoxTableCell.forTableColumn(geUseCol));
        geUseCol.setEditable(true);
        geNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(shortName(c.getValue().name)));
        geSampleTable.setItems(geSamples);
        geSampleTable.setEditable(true);
        geScaleCombo.setItems(FXCollections.observableArrayList("Linear", "Log", "ArcSinh"));
        geScaleCombo.getSelectionModel().select("ArcSinh");
        geChart.setAxisLabels("", "Normalized");

        // Gating Strategy canvas toolbar
        gsArrowStyleCombo.setItems(FXCollections.observableArrayList(
                "Solid arrow", "Dashed arrow", "Thick arrow", "Line only"));
        gsArrowStyleCombo.getSelectionModel().selectFirst();
        gsCanvas.setOnMousePressed(e -> { if (e.getTarget() == gsCanvas) { clearGsSelection(); clearArrowSelection(); hideGsPopup(); } });
        gsScrollPane.addEventFilter(javafx.scene.input.KeyEvent.KEY_PRESSED, this::onGsKeyPressed);

        initLayoutTab();
        setDisabled(true);
    }

    private boolean gsTreeListenerAdded = false;

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        if (!gsTreeListenerAdded) {
            // Auto-refresh the gating-strategy figure when gates/axes change anywhere (e.g. the user
            // edits axes in the graph window). See ui-bug-log BUG-10.
            context.workspace().addTreeChangeListener(this::onWorkspaceTreeChanged);
            gsTreeListenerAdded = true;
        }
        refreshFromWorkspace();
    }

    /** Rebuild the currently-shown figure so it reflects the latest gates/axes from the shared tree. */
    private void onWorkspaceTreeChanged() {
        if (ctx == null || currentGsSample == null || gsCells.isEmpty()) return;
        if (ctx.workspace().data(currentGsSample) == null) return;   // data evicted — don't wipe the figure
        buildCanvas(currentGsSample);
    }

    @Override
    public void refreshFromWorkspace() {
        if (ctx == null) return;
        Map<String, Boolean> prev = new LinkedHashMap<>();
        for (SampleSel s : samples) prev.put(s.name, s.use.get());
        samples.clear();
        for (String s : ctx.workspace().sampleNames()) {
            SampleSel row = new SampleSel(s);
            if (prev.containsKey(s)) row.use.set(prev.get(s));
            samples.add(row);
        }
        List<String> chans = new ArrayList<>(ctx.workspace().channelNames());
        chans.removeIf(c -> c.toLowerCase().matches(".*(fsc|ssc|time).*"));
        if (chans.isEmpty()) chans = new ArrayList<>(ctx.workspace().channelNames());
        channelList.setItems(FXCollections.observableArrayList(chans));

        // ---- Graph Export tab: mirror sample list, populate population + channel pickers ----
        Map<String, Boolean> gePrev = new LinkedHashMap<>();
        for (SampleSel s : geSamples) gePrev.put(s.name, s.use.get());
        geSamples.clear();
        for (String s : ctx.workspace().sampleNames()) {
            SampleSel row = new SampleSel(s);
            if (gePrev.containsKey(s)) row.use.set(gePrev.get(s));
            geSamples.add(row);
        }
        TreeSet<String> gates = new TreeSet<>();
        for (String s : ctx.workspace().samples())
            for (PopNode n : ctx.workspace().treeFor(s).selfAndDescendants()) if (!n.isRoot()) gates.add(n.name());
        List<String> popOpts = new ArrayList<>();
        popOpts.add("All Events");
        popOpts.addAll(gates);
        String keepPop = gePopCombo.getValue();
        gePopCombo.setItems(FXCollections.observableArrayList(popOpts));
        gePopCombo.getSelectionModel().select(keepPop != null && popOpts.contains(keepPop) ? keepPop : "All Events");
        List<String> allChans = new ArrayList<>(ctx.workspace().channelNames());
        String keepCh = geChannelCombo.getValue();
        geChannelCombo.setItems(FXCollections.observableArrayList(allChans));
        if (keepCh != null && allChans.contains(keepCh)) geChannelCombo.getSelectionModel().select(keepCh);
        else if (!chans.isEmpty()) geChannelCombo.getSelectionModel().select(chans.get(0));

        // ---- Gating Strategy tab: sample picker ----
        String keepGs = gsSampleCombo.getValue();
        gsSampleCombo.setItems(FXCollections.observableArrayList(ctx.workspace().sampleNames()));
        if (keepGs != null && ctx.workspace().sampleNames().contains(keepGs)) gsSampleCombo.getSelectionModel().select(keepGs);
        else if (!ctx.workspace().sampleNames().isEmpty()) gsSampleCombo.getSelectionModel().selectFirst();

        // ---- Layout tab: sample picker (drives the source population tree) ----
        String keepLay = layoutSampleCombo.getValue();
        layoutSampleCombo.setItems(FXCollections.observableArrayList(ctx.workspace().sampleNames()));
        if (keepLay != null && ctx.workspace().sampleNames().contains(keepLay)) layoutSampleCombo.getSelectionModel().select(keepLay);
        else if (!ctx.workspace().sampleNames().isEmpty()) layoutSampleCombo.getSelectionModel().selectFirst();
        rebuildLayoutTree();
    }

    // ==== Layout Editor tab (drag-drop tiles + batch multi-page PDF) ===========

    private void initLayoutTab() {
        layoutColsSpinner.setValueFactory(new javafx.scene.control.SpinnerValueFactory.IntegerSpinnerValueFactory(1, 6, 2));
        layoutRowsSpinner.setValueFactory(new javafx.scene.control.SpinnerValueFactory.IntegerSpinnerValueFactory(1, 6, 2));
        layoutColsSpinner.valueProperty().addListener((o, a, b) -> renderLayoutForCurrentSample());
        layoutSampleCombo.setOnAction(e -> { rebuildLayoutTree(); renderLayoutForCurrentSample(); });
        layoutTree.setShowRoot(true);
        layoutTree.setCellFactory(tv -> new javafx.scene.control.TreeCell<>() {
            @Override protected void updateItem(PopNode n, boolean empty) {
                super.updateItem(n, empty);
                setText(empty || n == null ? null : (n.isRoot() ? "All Events" : n.name()));
            }
        });
        // Drag a population out of the tree…
        layoutTree.setOnDragDetected(e -> {
            javafx.scene.control.TreeItem<PopNode> it = layoutTree.getSelectionModel().getSelectedItem();
            if (it == null || it.getValue() == null) return;
            javafx.scene.input.Dragboard db = layoutTree.startDragAndDrop(javafx.scene.input.TransferMode.COPY);
            javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
            cc.put(POP_PATH, new ArrayList<>(namePath(it.getValue())));
            db.setContent(cc);
            e.consume();
        });
        // …and drop it onto the canvas.
        layoutCanvas.setOnDragOver(e -> {
            if (e.getDragboard().hasContent(POP_PATH)) e.acceptTransferModes(javafx.scene.input.TransferMode.COPY);
            e.consume();
        });
        layoutCanvas.setOnDragDropped(e -> {
            javafx.scene.input.Dragboard db = e.getDragboard();
            boolean ok = false;
            if (db.hasContent(POP_PATH)) {
                @SuppressWarnings("unchecked")
                java.util.List<String> path = (java.util.List<String>) db.getContent(POP_PATH);
                layoutTiles.add(new ArrayList<>(path));
                renderLayoutForCurrentSample();
                ok = true;
            }
            e.setDropCompleted(ok);
            e.consume();
        });
    }

    private void rebuildLayoutTree() {
        if (ctx == null) return;
        String sample = layoutSampleCombo.getValue();
        if (sample == null || !ctx.workspace().hasTree(sample)) { layoutTree.setRoot(null); return; }
        layoutTree.setRoot(buildLayoutItem(ctx.workspace().treeFor(sample)));
        if (layoutTree.getRoot() != null) layoutTree.getRoot().setExpanded(true);
    }
    private javafx.scene.control.TreeItem<PopNode> buildLayoutItem(PopNode n) {
        javafx.scene.control.TreeItem<PopNode> it = new javafx.scene.control.TreeItem<>(n);
        for (PopNode c : n.children) it.getChildren().add(buildLayoutItem(c));
        return it;
    }

    /** Name-path from root (root excluded) — lets the same population be re-resolved in another sample. */
    private static java.util.List<String> namePath(PopNode p) {
        java.util.ArrayList<String> path = new java.util.ArrayList<>();
        for (PopNode n = p; n != null && !n.isRoot(); n = n.parent) path.add(0, n.name());
        return path;
    }
    private static PopNode resolvePath(PopNode root, java.util.List<String> path) {
        PopNode cur = root;
        for (String name : path) {
            PopNode next = null;
            for (PopNode c : cur.children) if (name.equals(c.name())) { next = c; break; }
            if (next == null) return null;
            cur = next;
        }
        return cur;
    }

    private void renderLayoutForCurrentSample() {
        String sample = layoutSampleCombo.getValue();
        if (sample == null) return;
        if (ctx.workspace().data(sample) == null) {
            layoutStatusLabel.setText("Loading " + shortName(sample) + "…");
            EventLoader.ensureLoaded(ctx, List.of(sample), layoutStatusLabel::setText, () -> renderTiles(sample, layoutCanvas, true));
        } else {
            renderTiles(sample, layoutCanvas, true);
        }
    }

    /** Render every blueprint tile for one sample onto a canvas, arranged in the Cols grid. Populations
     *  that don't exist in this sample are skipped. Returns the number of tiles placed. */
    private int renderTiles(String sample, Pane canvas, boolean interactive) {
        canvas.getChildren().clear();
        EventData root = ctx.workspace().data(sample);
        if (root == null) return 0;
        PopNode rootNode = ctx.workspace().treeFor(sample);
        int cols = Math.max(1, layoutColsSpinner.getValue());
        int placed = 0;
        for (java.util.List<String> path : layoutTiles) {
            PopNode node = path.isEmpty() ? rootNode : resolvePath(rootNode, path);
            if (node == null) continue;
            VBox cell;
            try { cell = makeStepCell(root, node); } catch (Exception ex) { continue; }
            int col = placed % cols, row = placed / cols;
            cell.setLayoutX(L_MARGIN + col * (L_CELL_W + L_GAP));
            cell.setLayoutY(L_MARGIN + row * (L_CELL_H + L_GAP));
            canvas.getChildren().add(cell);
            placed++;
        }
        if (interactive) layoutStatusLabel.setText(placed + " tile(s) · " + shortName(sample)
                + ". Drag more populations from the tree; Batch renders every sample.");
        return placed;
    }

    @FXML private void onLayoutClear() {
        layoutTiles.clear();
        layoutCanvas.getChildren().clear();
        layoutStatusLabel.setText("Canvas cleared.");
    }

    @FXML private void onLayoutSavePdf() {
        if (layoutCanvas.getChildren().isEmpty()) { layoutStatusLabel.setText("Drop at least one population first."); return; }
        File f = choosePdf("layout.pdf");
        if (f == null) return;
        javafx.scene.image.WritableImage img = snapPane(layoutCanvas);
        new Thread(() -> {
            try { writePdf(List.of(img), f); javafx.application.Platform.runLater(() -> layoutStatusLabel.setText("Saved " + f.getName() + ".")); }
            catch (Exception ex) { javafx.application.Platform.runLater(() -> layoutStatusLabel.setText("PDF failed: " + ex.getMessage())); }
        }, "layout-pdf").start();
    }

    /** Batch: render the tile template for EVERY sample (locked axes) → one page per sample → multi-page PDF. */
    @FXML private void onLayoutBatch() {
        if (layoutTiles.isEmpty()) { layoutStatusLabel.setText("Drop at least one population first."); return; }
        java.util.List<String> allSamples = new ArrayList<>(ctx.workspace().sampleNames());
        if (allSamples.isEmpty()) { layoutStatusLabel.setText("No samples loaded."); return; }
        File f = choosePdf("layout_batch.pdf");
        if (f == null) return;
        layoutBatchButton.setDisable(true);
        EventLoader.ensureLoaded(ctx, allSamples, layoutStatusLabel::setText,
                () -> batchSequence(allSamples, 0, new ArrayList<>(), f));
    }

    /** Sequentially render each sample onto the live canvas, wait for the async plot render to settle,
     *  snapshot it as a PDF page, then move on. FX-thread state machine with pauses (never blocks it). */
    private void batchSequence(java.util.List<String> samples, int idx,
                               java.util.List<javafx.scene.image.WritableImage> pages, File out) {
        if (idx >= samples.size()) {
            new Thread(() -> {
                try {
                    writePdf(pages, out);
                    javafx.application.Platform.runLater(() -> {
                        layoutStatusLabel.setText("Saved " + pages.size() + " page(s) → " + out.getName());
                        layoutBatchButton.setDisable(false); renderLayoutForCurrentSample();
                    });
                } catch (Exception ex) {
                    javafx.application.Platform.runLater(() -> {
                        layoutStatusLabel.setText("PDF failed: " + ex.getMessage()); layoutBatchButton.setDisable(false);
                    });
                }
            }, "layout-batch-pdf").start();
            return;
        }
        String sample = samples.get(idx);
        layoutStatusLabel.setText("Rendering " + (idx + 1) + "/" + samples.size() + " — " + shortName(sample) + "…");
        int placed = renderTiles(sample, layoutCanvas, false);
        // Let the async CytoPlot renders paint before snapshotting (50ms debounce + off-thread compute).
        javafx.animation.PauseTransition pause = new javafx.animation.PauseTransition(javafx.util.Duration.millis(650));
        pause.setOnFinished(e -> {
            if (placed > 0) pages.add(snapPane(layoutCanvas));
            batchSequence(samples, idx + 1, pages, out);
        });
        pause.play();
    }

    private File choosePdf(String defaultName) {
        FileChooser fc = new FileChooser();
        fc.setTitle("Save layout as PDF");
        fc.setInitialFileName(defaultName);
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PDF document (*.pdf)", "*.pdf"));
        return fc.showSaveDialog(layoutCanvas.getScene().getWindow());
    }

    /** Snapshot a canvas Pane at export DPI, cropped tight (white bg). */
    private javafx.scene.image.WritableImage snapPane(Pane canvas) {
        double scale = Math.max(1.0, (ctx != null ? ctx.settings().exportDpi() : 300) / 96.0);
        javafx.scene.SnapshotParameters sp = new javafx.scene.SnapshotParameters();
        sp.setFill(javafx.scene.paint.Color.WHITE);
        sp.setTransform(javafx.scene.transform.Transform.scale(scale, scale));
        double maxX = L_MARGIN, maxY = L_MARGIN;
        for (javafx.scene.Node n : canvas.getChildren()) {
            maxX = Math.max(maxX, n.getLayoutX() + n.getBoundsInParent().getWidth());
            maxY = Math.max(maxY, n.getLayoutY() + n.getBoundsInParent().getHeight());
        }
        sp.setViewport(new javafx.geometry.Rectangle2D(0, 0, maxX + L_MARGIN, maxY + L_MARGIN));
        return canvas.snapshot(sp, null);
    }

    /** Assemble one PDF page per image via Apache PDFBox. */
    private static void writePdf(java.util.List<javafx.scene.image.WritableImage> pages, File out) throws Exception {
        try (org.apache.pdfbox.pdmodel.PDDocument doc = new org.apache.pdfbox.pdmodel.PDDocument()) {
            for (javafx.scene.image.WritableImage img : pages) {
                java.awt.image.BufferedImage bi = javafx.embed.swing.SwingFXUtils.fromFXImage(img, null);
                org.apache.pdfbox.pdmodel.PDPage page = new org.apache.pdfbox.pdmodel.PDPage(
                        new org.apache.pdfbox.pdmodel.common.PDRectangle(bi.getWidth(), bi.getHeight()));
                doc.addPage(page);
                org.apache.pdfbox.pdmodel.graphics.image.PDImageXObject pdImg =
                        org.apache.pdfbox.pdmodel.graphics.image.LosslessFactory.createFromImage(doc, bi);
                try (org.apache.pdfbox.pdmodel.PDPageContentStream cs =
                             new org.apache.pdfbox.pdmodel.PDPageContentStream(doc, page)) {
                    cs.drawImage(pdImg, 0, 0, bi.getWidth(), bi.getHeight());
                }
            }
            doc.save(out);
        }
    }

    // ---- Gating Strategy canvas ----

    @FXML
    private void onGsBuild() {
        if (ctx == null) return;
        String sample = gsSampleCombo.getValue();
        if (sample == null) { info("No sample", "Pick a sample to build its gating-strategy figure."); return; }
        if (!ctx.workspace().hasTree(sample) || ctx.workspace().treeFor(sample).children.isEmpty()) {
            info("No gates", "This sample has no gates. Draw a gating strategy first.");
            return;
        }
        currentGsSample = sample;
        gsStatusLabel.setText("Loading events…");
        EventLoader.ensureLoaded(ctx, List.of(sample), gsStatusLabel::setText, () -> buildCanvas(sample));
    }

    private void buildCanvas(String sample) {
        WorkspaceModel ws = ctx.workspace();
        EventData root = ws.data(sample);
        if (root == null || root.rows() == 0) { gsStatusLabel.setText("No events for " + sample + "."); return; }
        PopNode rootNode = ws.treeFor(sample);
        List<PopNode> steps = new ArrayList<>();
        collectSteps(rootNode, steps);
        if (steps.isEmpty()) { info("No gates", "This sample has no gates to show."); return; }

        // Clear previous canvas content (panels + arrows), but keep user text boxes
        gsCanvas.getChildren().removeIf(n -> n.getUserData() instanceof String s &&
                (s.startsWith("panel") || s.startsWith("arrow")));
        gsCells.clear();
        cellPopMap.clear();
        cellPlotMap.clear();
        gsLinks.clear();
        gsSelectedArrow = null;
        arrowShapesByLink.clear();
        gsCellBorder.clear();
        clearGsSelection();
        hideGsPopup();

        // Auto-layout: place panels left to right, auto-connect adjacent siblings
        double x = 30, y = 40;
        final double CELL_W = 240, GAP = 60;
        for (int i = 0; i < steps.size(); i++) {
            PopNode p = steps.get(i);
            try {
                VBox cell = makeStepCell(root, p);
                cell.setLayoutX(x);
                cell.setLayoutY(y);
                cell.setUserData("panel:" + i);
                gsCanvas.getChildren().add(cell);
                gsCells.add(cell);
                cellPopMap.put(cell, p);
                makeGsDraggable(cell, () -> { redrawArrows(); if (gsSelected == cell) repositionHandles(); });
                makeGsInteractive(cell, p);
                if (i > 0) gsLinks.add(new GsLink(gsCells.get(i - 1), cell));
                x += CELL_W + GAP;
            } catch (Exception ex) {
                gsStatusLabel.setText("Error at step '" + p.name() + "': " + ex.getMessage());
                return;
            }
        }
        redrawArrows();
        gsStatusLabel.setText(steps.size() + " step(s) for " + shortName(sample) + ". Drag panels · Double-click to open · Right-click for options.");
    }

    private void collectSteps(PopNode n, List<PopNode> out) {
        if (!n.children.isEmpty()) out.add(n);
        for (PopNode c : n.children) collectSteps(c, out);
    }

    /** Build one panel VBox: label + CytoPlot, styled for light/publication mode. */
    private VBox makeStepCell(EventData root, PopNode p) {
        EventData ev = p.isRoot() ? root : subsetFor(root, p);
        // Axis selection — MUST match GraphWindowController.selectNode priority so the panel reflects
        // whatever the graph window shows: user's stored view (viewX) first, then the axes the child
        // gates were drawn on (so children render), then this node's own gate axes as a last resort.
        // See ui-bug-log BUG-06: putting child-gate axes ahead of viewX made axis edits invisible here.
        String ax = p.viewX, ay = p.viewY, axs = p.viewXScale, ays = p.viewYScale;
        if (ax == null && !p.children.isEmpty()) {
            CytoPlot.Gate g0 = p.children.get(0).gate;
            if (g0 != null && g0.xChan != null) { ax = g0.xChan; ay = g0.yChan; }
        }
        if (ax == null && !p.isRoot() && p.gate != null) { ax = p.gate.xChan; ay = p.gate.yChan; }
        // Scale is UNIVERSAL per marker (BUG-11): the workspace-wide scale for the channel wins, so the
        // figure always matches the graph window. Fall back to stored node scale, then child-parent scale.
        String axU = ctx.workspace().channelScale(ax);
        if (axU != null) axs = axU;
        else if (axs == null && !p.children.isEmpty()) {
            PopNode firstChild = p.children.get(0);
            axs = firstChild.parent != null ? firstChild.parent.viewXScale : null;
        }
        if (ay != null) { String ayU = ctx.workspace().channelScale(ay); if (ayU != null) ays = ayU; }
        boolean hist = ay == null;
        CytoPlot plot = new CytoPlot();
        plot.setMinSize(120, 120); plot.setPrefSize(220, 220); plot.setMaxSize(500, 500);
        plot.setChannelLabeler(c -> ctx.aliases().label(c));
        plot.setLightMode(true);
        plot.setData(ev);
        plot.setView(ax, hist ? null : ay, scaleOf(axs), scaleOf(ays), hist ? "histogram" : "pseudocolor");
        // Universal per-marker axis range (BUG-17) so panels match the graph window and gates align.
        double[] xr = ax == null ? null : ctx.workspace().channelRange(ax);
        if (xr != null) { plot.setXMin(xr[0]); plot.setXMax(xr[1]); }
        double[] yr = (hist || ay == null) ? null : ctx.workspace().channelRange(ay);
        if (yr != null) { plot.setYMin(yr[0]); plot.setYMax(yr[1]); }
        for (PopNode c : p.children) if (c.gate != null) {
            double[] off = ctx.workspace().popLabelOffset(c.name());   // BUG-14: universal label position
            if (off != null) { c.gate.lblDx = off[0]; c.gate.lblDy = off[1]; }
            plot.addGate(c.gate);
        }
        Label cap = new Label((p.isRoot() ? "All Events" : p.name()) + "  (" + ev.rows() + ")");
        cap.setStyle("-fx-text-fill:#1A2330; -fx-font-weight:bold; -fx-font-size:11;");
        VBox cell = new VBox(3, cap, plot);
        cellPlotMap.put(cell, plot);
        applyCellStyle(cell);
        return cell;
    }

    /** Panel base style, honouring the per-cell border toggle (default: bordered). */
    private void applyCellStyle(VBox cell) {
        boolean border = gsCellBorder.getOrDefault(cell, Boolean.TRUE);
        cell.setStyle("-fx-background-color:white; -fx-padding:4; -fx-border-width:1; -fx-border-color:"
                + (border ? "#D0D8E4" : "transparent") + ";");
    }

    // ---- drag support ----

    /** Shared drag-to-move behaviour for any canvas-resident Region (panel cell or text box). */
    private void makeGsDraggable(javafx.scene.layout.Region node, Runnable onDragged) {
        double[] base = new double[4];   // [sceneX, sceneY, layoutX, layoutY] at press
        node.setOnMousePressed(e -> {
            if (e.getButton() != MouseButton.PRIMARY) return;
            if (gsConnectMode) { e.consume(); return; }
            base[0] = e.getSceneX(); base[1] = e.getSceneY();
            base[2] = node.getLayoutX(); base[3] = node.getLayoutY();
            node.toFront();
            e.consume();
        });
        node.setOnMouseDragged(e -> {
            if (e.getButton() != MouseButton.PRIMARY || gsConnectMode) return;
            javafx.geometry.Point2D local = gsCanvas.sceneToLocal(e.getSceneX(), e.getSceneY());
            javafx.geometry.Point2D baseLocal = gsCanvas.sceneToLocal(base[0], base[1]);
            node.setLayoutX(Math.max(0, base[2] + local.getX() - baseLocal.getX()));
            node.setLayoutY(Math.max(0, base[3] + local.getY() - baseLocal.getY()));
            if (onDragged != null) onDragged.run();
            e.consume();
        });
    }

    // ---- selection (shared by panel cells and text boxes) ----

    private void selectGsNode(javafx.scene.Node n) {
        clearArrowSelection();
        if (gsSelected == n) { if (n instanceof VBox && gsScrollPane != null) gsScrollPane.requestFocus(); return; }
        clearGsSelection();
        gsSelected = n;
        showResizeHandles(n);
        // Panels aren't focusable; focus the scroll pane so the Delete-key filter receives keys.
        if (n instanceof VBox && gsScrollPane != null) gsScrollPane.requestFocus();
    }

    /** Delete/Backspace removes the selected arrow, text box, or panel (unless a text box is being edited). */
    private void onGsKeyPressed(javafx.scene.input.KeyEvent e) {
        if (e.getCode() != javafx.scene.input.KeyCode.DELETE && e.getCode() != javafx.scene.input.KeyCode.BACK_SPACE) return;
        if (gsSelectedArrow != null) {
            gsLinks.remove(gsSelectedArrow); gsSelectedArrow = null; redrawArrows(); e.consume(); return;
        }
        if (gsSelected instanceof TextArea ta) {
            if (ta.isEditable()) return;   // actively editing text — let Delete edit, don't remove the box
            removeGsTextBox(ta); e.consume();
        } else if (gsSelected instanceof VBox cell) {
            removeGsPanel(cell); e.consume();
        }
    }

    private void clearGsSelection() {
        gsSelected = null;
        gsHandles.clear();
        gsSelectionOutline = null;
        gsCanvas.getChildren().removeIf(n -> "handle".equals(n.getUserData()));
    }

    private static final int[][] GS_HANDLE_ALIGN = {{-1,-1},{0,-1},{1,-1},{1,0},{1,1},{0,1},{-1,1},{-1,0}};

    /** Build the selection outline + 8 resize handles around the selected node, in gsCanvas coordinates. */
    private void showResizeHandles(javafx.scene.Node n) {
        gsCanvas.getChildren().removeIf(c -> "handle".equals(c.getUserData()));
        gsHandles.clear();

        javafx.scene.layout.Region moveTarget;
        javafx.scene.layout.Region sizeTarget;
        if (n instanceof VBox cell) {
            moveTarget = cell;
            CytoPlot plot = cellPlotMap.get(cell);
            sizeTarget = plot != null ? plot : cell;
        } else if (n instanceof javafx.scene.layout.Region r) {
            moveTarget = r;
            sizeTarget = r;
        } else {
            return;
        }

        gsSelectionOutline = new javafx.scene.shape.Rectangle();
        gsSelectionOutline.setUserData("handle");
        gsSelectionOutline.setFill(Color.TRANSPARENT);
        gsSelectionOutline.setStroke(Color.web("#1976D2"));
        gsSelectionOutline.setStrokeWidth(1.5);
        gsSelectionOutline.setMouseTransparent(true);
        gsCanvas.getChildren().add(gsSelectionOutline);

        for (int[] align : GS_HANDLE_ALIGN) {
            javafx.scene.shape.Rectangle h = new javafx.scene.shape.Rectangle(8, 8);
            h.setFill(Color.web("#1976D2"));
            h.setUserData("handle");
            gsHandles.add(h);
            gsCanvas.getChildren().add(h);
            wireResizeHandle(h, n, sizeTarget, moveTarget, align[0], align[1]);
        }
        repositionHandles();
    }

    /** Reposition the current selection outline + handles to match gsSelected's live bounds (no node recreation). */
    private void repositionHandles() {
        if (gsSelected == null || gsSelectionOutline == null) return;
        double x = gsSelected.getLayoutX(), y = gsSelected.getLayoutY();
        double w = gsSelected.getBoundsInLocal().getWidth(), h = gsSelected.getBoundsInLocal().getHeight();
        gsSelectionOutline.setX(x); gsSelectionOutline.setY(y);
        gsSelectionOutline.setWidth(w); gsSelectionOutline.setHeight(h);
        for (int i = 0; i < gsHandles.size() && i < GS_HANDLE_ALIGN.length; i++) {
            int hAlign = GS_HANDLE_ALIGN[i][0], vAlign = GS_HANDLE_ALIGN[i][1];
            double hx = x + (hAlign + 1) / 2.0 * w - 4;
            double hy = y + (vAlign + 1) / 2.0 * h - 4;
            gsHandles.get(i).setX(hx); gsHandles.get(i).setY(hy);
        }
    }

    private static final double GS_TEXT_MIN_W = 40, GS_TEXT_MIN_H = 30, GS_TEXT_MAX = 900;

    private void wireResizeHandle(javafx.scene.shape.Rectangle handle, javafx.scene.Node node,
                                   javafx.scene.layout.Region sizeTarget, javafx.scene.layout.Region moveTarget,
                                   int hAlign, int vAlign) {
        double[] base = new double[6]; // sceneX, sceneY, sizeW, sizeH, moveX, moveY at press
        handle.setOnMousePressed(e -> {
            if (e.getButton() != MouseButton.PRIMARY) return;
            base[0] = e.getSceneX(); base[1] = e.getSceneY();
            base[2] = sizeTarget.getWidth(); base[3] = sizeTarget.getHeight();
            base[4] = moveTarget.getLayoutX(); base[5] = moveTarget.getLayoutY();
            e.consume();
        });
        handle.setOnMouseDragged(e -> {
            if (e.getButton() != MouseButton.PRIMARY) return;
            javafx.geometry.Point2D cur = gsCanvas.sceneToLocal(e.getSceneX(), e.getSceneY());
            javafx.geometry.Point2D start = gsCanvas.sceneToLocal(base[0], base[1]);
            double dx = cur.getX() - start.getX(), dy = cur.getY() - start.getY();
            boolean isPlot = sizeTarget instanceof CytoPlot;
            double minW = isPlot ? 120 : GS_TEXT_MIN_W, maxW = isPlot ? 500 : GS_TEXT_MAX;
            double minH = isPlot ? 120 : GS_TEXT_MIN_H, maxH = isPlot ? 500 : GS_TEXT_MAX;
            if (hAlign != 0) {
                double newW = Math.max(minW, Math.min(maxW, base[2] + hAlign * dx));
                double deltaW = newW - base[2];
                sizeTarget.setPrefWidth(newW);
                if (hAlign < 0) moveTarget.setLayoutX(base[4] - deltaW);
            }
            if (vAlign != 0) {
                double newH = Math.max(minH, Math.min(maxH, base[3] + vAlign * dy));
                double deltaH = newH - base[3];
                sizeTarget.setPrefHeight(newH);
                if (vAlign < 0) moveTarget.setLayoutY(base[5] - deltaH);
            }
            node.autosize();
            repositionHandles();
            redrawArrows();
            e.consume();
        });
    }

    // ---- interactions: double-click + right-click ----

    private void makeGsInteractive(VBox cell, PopNode pop) {
        CytoPlot plot = cellPlotMap.get(cell);
        Label cap = (Label) cell.getChildren().get(0);

        // Single primary click on the panel (outside the title) → connect-mode source/target pick, or select.
        cell.setOnMouseClicked(e -> {
            if (e.getButton() != MouseButton.PRIMARY || e.getClickCount() != 1) return;
            if (gsConnectMode) { handleConnectClick(cell); e.consume(); }
            else { selectGsNode(cell); }
        });

        // Double-click on the plot body → open graphing window for this population.
        if (plot != null) {
            plot.setOnMouseClicked(e -> {
                if (e.getButton() != MouseButton.PRIMARY || e.getClickCount() < 2) return;
                if (currentGsSample == null) return;
                hideGsPopup();
                PopNode focus = pop.isRoot() ? null : pop;
                GraphWindowController.openChild(ctx, currentGsSample, focus, true);
                e.consume();
            });
        }

        // Double-click on the title → inline rename (display-only, does not rename the underlying gate).
        cap.setOnMouseClicked(e -> {
            if (e.getButton() != MouseButton.PRIMARY || e.getClickCount() < 2) return;
            startInlineRename(cell, cap);
            e.consume();
        });

        // Right-click → context menu
        ContextMenu cm = buildPanelContextMenu(cell, pop);
        cell.setOnContextMenuRequested(e -> cm.show(cell, e.getScreenX(), e.getScreenY()));
    }

    private void startInlineRename(VBox cell, Label cap) {
        javafx.scene.control.TextField tf = new javafx.scene.control.TextField(cap.getText());
        tf.setStyle(cap.getStyle());
        cell.getChildren().set(0, tf);
        tf.requestFocus();
        tf.selectAll();
        boolean[] committed = {false};
        Runnable commit = () -> {
            if (committed[0]) return;
            committed[0] = true;
            String text = tf.getText();
            if (text != null && !text.isBlank()) cap.setText(text);
            if (!cell.getChildren().isEmpty() && cell.getChildren().get(0) == tf) cell.getChildren().set(0, cap);
        };
        tf.setOnAction(e -> commit.run());
        tf.focusedProperty().addListener((o, was, isNow) -> { if (!isNow) commit.run(); });
        tf.setOnKeyPressed(e -> {
            if (e.getCode() == javafx.scene.input.KeyCode.ESCAPE) {
                committed[0] = true;
                cell.getChildren().set(0, cap);
            }
        });
    }

    private ContextMenu buildPanelContextMenu(VBox cell, PopNode pop) {
        ContextMenu cm = new ContextMenu();

        MenuItem miOpen = new MenuItem("Open in graphing window");
        miOpen.setOnAction(e -> {
            if (currentGsSample == null) return;
            hideGsPopup();
            GraphWindowController.openChild(ctx, currentGsSample, pop.isRoot() ? null : pop, true);
        });

        // Plot style submenu — re-derive axes from the PopNode axes stored in viewX/viewY
        Menu miStyle = new Menu("Plot style");
        for (String style : List.of("pseudocolor", "histogram", "dot")) {
            MenuItem si = new MenuItem(style.substring(0,1).toUpperCase() + style.substring(1));
            si.setOnAction(ev -> {
                CytoPlot plot = cellPlotMap.get(cell);
                if (plot == null) return;
                String ax = pop.viewX, ay = pop.viewY;
                if (ax == null && !pop.children.isEmpty()) {
                    CytoPlot.Gate g0 = pop.children.get(0).gate;
                    if (g0 != null) { ax = g0.xChan; ay = g0.yChan; }
                }
                plot.setView(ax, "histogram".equals(style) ? null : ay,
                        scaleOf(pop.viewXScale), scaleOf(pop.viewYScale), style);
            });
            miStyle.getItems().add(si);
        }

        // Axis font size slider
        MenuItem miAxisFont = new MenuItem("Axis label size…");
        miAxisFont.setOnAction(e -> showSliderPopup(cell, "Axis label size", 8, 24, 12,
                v -> { CytoPlot p = cellPlotMap.get(cell); if (p != null) p.setAxisFontSize(v); }));

        // Point/dot size, expressed as a percentage of the 0-10px internal radius range
        MenuItem miDotSize = new MenuItem("Point size…");
        miDotSize.setOnAction(e -> {
            CytoPlot p0 = cellPlotMap.get(cell);
            double initPct = p0 != null ? p0.getPointRadius() / 10.0 * 100.0 : 10.0;
            showSliderPopup(cell, "Point size (%)", 0, 100, initPct,
                    v -> { CytoPlot p = cellPlotMap.get(cell); if (p != null) p.setPointRadius((int) Math.round(v / 100.0 * 10)); });
        });

        // Border toggle (PowerPoint-style — the frame is optional)
        javafx.scene.control.CheckMenuItem miBorder = new javafx.scene.control.CheckMenuItem("Border");
        miBorder.setSelected(gsCellBorder.getOrDefault(cell, Boolean.TRUE));
        miBorder.setOnAction(e -> { gsCellBorder.put(cell, miBorder.isSelected()); applyCellStyle(cell); });

        // Remove this panel
        MenuItem miRemovePanel = new MenuItem("Remove panel");
        miRemovePanel.setOnAction(e -> removeGsPanel(cell));

        // Connect from this panel
        MenuItem miConnect = new MenuItem("Connect from here");
        miConnect.setOnAction(e -> { gsConnectMode = true; gsConnectSource = cell; gsConnectBtn.setSelected(true);
            gsStatusLabel.setText("Click a second panel to draw the arrow."); });

        cm.getItems().addAll(miOpen, new SeparatorMenuItem(), miStyle, new SeparatorMenuItem(),
                miAxisFont, miDotSize, miBorder, new SeparatorMenuItem(), miConnect, miRemovePanel);
        return cm;
    }

    private void removeGsPanel(VBox cell) {
        gsCanvas.getChildren().remove(cell);
        gsCells.remove(cell);
        cellPopMap.remove(cell);
        cellPlotMap.remove(cell);
        gsCellBorder.remove(cell);
        gsLinks.removeIf(l -> l.from == cell || l.to == cell);
        if (gsSelected == cell) clearGsSelection();
        redrawArrows();
    }

    // ---- connect mode ----

    @FXML private void onGsConnectToggle() {
        gsConnectMode = gsConnectBtn.isSelected();
        gsConnectSource = null;
        gsStatusLabel.setText(gsConnectMode ? "Click the SOURCE panel, then the TARGET panel." : "");
    }

    private void handleConnectClick(VBox cell) {
        if (gsConnectSource == null) {
            gsConnectSource = cell;
            cell.setStyle(cell.getStyle() + "-fx-border-color:#2C7FB8; -fx-border-width:2;");
            gsStatusLabel.setText("Source selected — click target panel.");
        } else if (gsConnectSource != cell) {
            gsLinks.add(new GsLink(gsConnectSource, cell));
            applyCellStyle(gsConnectSource);
            gsConnectSource = null;
            gsConnectMode = false;
            gsConnectBtn.setSelected(false);
            redrawArrows();
            gsStatusLabel.setText("Connection added. Right-click an arrow to remove it.");
        }
    }

    // ---- arrow drawing ----

    private void redrawArrows() {
        // Remove all existing arrow shapes (lines, heads, endpoint handles) from canvas
        gsCanvas.getChildren().removeIf(n -> "arrow".equals(n.getUserData()));
        arrowShapesByLink.clear();
        String style = gsArrowStyleCombo.getValue() != null ? gsArrowStyleCombo.getValue() : "Solid arrow";
        for (GsLink link : gsLinks) {
            drawArrow(link, style);
        }
    }

    private void drawArrow(GsLink link, String style) {
        VBox from = link.from, to = link.to;
        double fx = from.getLayoutX() + from.getWidth() / 2;
        double fy = from.getLayoutY() + from.getHeight() / 2;
        double tx = to.getLayoutX() + to.getWidth() / 2;
        double ty = to.getLayoutY() + to.getHeight() / 2;

        // Direction unit vector
        double len = Math.hypot(tx - fx, ty - fy);
        if (len < 2) return;
        double dx = (tx - fx) / len, dy = (ty - fy) / len;

        // Trim line to cell edges
        double[] fp = edgePoint(from, dx, dy);
        double[] tp = edgePoint(to, -dx, -dy);

        double headLen = 14, headW = 7;
        double lineEndX = tp[0] - dx * headLen, lineEndY = tp[1] - dy * headLen;

        boolean selected = link == gsSelectedArrow;
        Color color = selected ? Color.web("#1976D2") : Color.web("#444444");
        List<javafx.scene.Node> shapes = new ArrayList<>();

        Line line = new Line(fp[0], fp[1], lineEndX, lineEndY);
        line.setStroke(color);
        boolean dashed = style.contains("Dashed");
        boolean thick = style.contains("Thick");
        boolean lineOnly = style.contains("Line only");
        line.setStrokeWidth(selected ? 3 : (thick ? 2.5 : 1.5));
        if (dashed) line.getStrokeDashArray().addAll(8.0, 5.0);
        line.setUserData("arrow");
        line.setOnMouseClicked(e -> { if (e.getButton() == MouseButton.PRIMARY) { selectGsArrow(link); e.consume(); } });
        line.setOnContextMenuRequested(e -> buildArrowContextMenu(link).show(line, e.getScreenX(), e.getScreenY()));
        gsCanvas.getChildren().add(line);
        shapes.add(line);

        if (!lineOnly) {
            double perpX = -dy, perpY = dx;
            Polygon head = new Polygon(
                    tp[0], tp[1],
                    lineEndX + perpX * headW, lineEndY + perpY * headW,
                    lineEndX - perpX * headW, lineEndY - perpY * headW);
            head.setFill(color);
            head.setUserData("arrow");
            head.setOnMouseClicked(e -> { if (e.getButton() == MouseButton.PRIMARY) { selectGsArrow(link); e.consume(); } });
            head.setOnContextMenuRequested(e -> buildArrowContextMenu(link).show(head, e.getScreenX(), e.getScreenY()));
            gsCanvas.getChildren().add(head);
            shapes.add(head);
        }

        if (selected) {
            javafx.scene.shape.Circle hFrom = new javafx.scene.shape.Circle(fp[0], fp[1], 5, Color.web("#1976D2"));
            hFrom.setUserData("arrow");
            wireArrowEndpointHandle(hFrom, link, true);
            gsCanvas.getChildren().add(hFrom);
            shapes.add(hFrom);

            javafx.scene.shape.Circle hTo = new javafx.scene.shape.Circle(tp[0], tp[1], 5, Color.web("#1976D2"));
            hTo.setUserData("arrow");
            wireArrowEndpointHandle(hTo, link, false);
            gsCanvas.getChildren().add(hTo);
            shapes.add(hTo);
        }

        arrowShapesByLink.put(link, shapes);
    }

    private void selectGsArrow(GsLink link) {
        clearGsSelection();
        gsSelectedArrow = link;
        redrawArrows();
    }

    private void clearArrowSelection() {
        if (gsSelectedArrow != null) { gsSelectedArrow = null; redrawArrows(); }
    }

    private ContextMenu buildArrowContextMenu(GsLink link) {
        ContextMenu cm = new ContextMenu();
        MenuItem miDelete = new MenuItem("Delete arrow");
        miDelete.setOnAction(e -> {
            gsLinks.remove(link);
            if (gsSelectedArrow == link) gsSelectedArrow = null;
            redrawArrows();
        });
        cm.getItems().add(miDelete);
        return cm;
    }

    /** Drag an arrow endpoint onto a different panel to relink it; snaps back if dropped on empty space. */
    private void wireArrowEndpointHandle(javafx.scene.shape.Circle handle, GsLink link, boolean isFromEnd) {
        handle.setOnMousePressed(e -> { if (e.getButton() == MouseButton.PRIMARY) e.consume(); });
        handle.setOnMouseDragged(e -> {
            if (e.getButton() != MouseButton.PRIMARY) return;
            javafx.geometry.Point2D p = gsCanvas.sceneToLocal(e.getSceneX(), e.getSceneY());
            handle.setCenterX(p.getX()); handle.setCenterY(p.getY());
            e.consume();
        });
        handle.setOnMouseReleased(e -> {
            javafx.geometry.Point2D p = gsCanvas.sceneToLocal(e.getSceneX(), e.getSceneY());
            VBox target = findCellAt(p.getX(), p.getY());
            VBox other = isFromEnd ? link.to : link.from;
            if (target != null && target != other) {
                if (isFromEnd) link.from = target; else link.to = target;
            }
            redrawArrows();
            e.consume();
        });
    }

    private VBox findCellAt(double x, double y) {
        for (VBox c : gsCells) {
            if (x >= c.getLayoutX() && x <= c.getLayoutX() + c.getWidth()
                    && y >= c.getLayoutY() && y <= c.getLayoutY() + c.getHeight()) return c;
        }
        return null;
    }

    /** Compute the point on a cell's bounding-box edge in direction (dx,dy) from its center. */
    private double[] edgePoint(VBox cell, double dx, double dy) {
        double cx = cell.getLayoutX() + cell.getWidth() / 2;
        double cy = cell.getLayoutY() + cell.getHeight() / 2;
        double hw = cell.getWidth() / 2 + 4, hh = cell.getHeight() / 2 + 4;
        double tx = dx != 0 ? hw / Math.abs(dx) : Double.MAX_VALUE;
        double ty = dy != 0 ? hh / Math.abs(dy) : Double.MAX_VALUE;
        double t = Math.min(tx, ty);
        return new double[]{cx + dx * t, cy + dy * t};
    }

    // ---- text box ----

    @FXML private void onGsAddText() {
        TextArea ta = new TextArea("Label text");
        ta.setPrefSize(180, 60);
        ta.setWrapText(true);
        TextStyleState state = new TextStyleState();
        textStyles.put(ta, state);
        ta.getStyleClass().add("gs-textbox");
        ta.setStyle(state.toCss());
        ta.setLayoutX(80); ta.setLayoutY(340);
        ta.setUserData("textbox");
        // PowerPoint-style: single click selects (shows handles); double-click enters text editing.
        // Staying out of edit mode until double-click is what lets the Delete key remove the box.
        ta.setEditable(false);
        makeGsDraggable(ta, () -> { if (gsSelected == ta) repositionHandles(); });
        ta.setOnMouseClicked(e -> {
            if (e.getButton() != MouseButton.PRIMARY) return;
            if (e.getClickCount() >= 2) { ta.setEditable(true); ta.requestFocus(); }
            else selectGsNode(ta);
        });
        ta.focusedProperty().addListener((o, was, isNow) -> { if (!isNow) ta.setEditable(false); });
        // Right-click for formatting
        ContextMenu tcm = buildTextContextMenu(ta);
        ta.setOnContextMenuRequested(e -> tcm.show(ta, e.getScreenX(), e.getScreenY()));
        gsCanvas.getChildren().add(ta);
        selectGsNode(ta);
        gsStatusLabel.setText("Text box added — click to select, double-click to edit, Del to remove.");
    }

    private ContextMenu buildTextContextMenu(TextArea ta) {
        TextStyleState state = textStyles.get(ta);
        ContextMenu cm = new ContextMenu();
        javafx.scene.control.CheckMenuItem miBold = new javafx.scene.control.CheckMenuItem("Bold");
        miBold.setSelected(state.bold);
        miBold.setOnAction(e -> { state.bold = miBold.isSelected(); ta.setStyle(state.toCss()); });
        javafx.scene.control.CheckMenuItem miItalic = new javafx.scene.control.CheckMenuItem("Italic");
        miItalic.setSelected(state.italic);
        miItalic.setOnAction(e -> { state.italic = miItalic.isSelected(); ta.setStyle(state.toCss()); });
        MenuItem miFontSize = new MenuItem("Font size…");
        miFontSize.setOnAction(e -> showSliderPopup(ta, "Font size", 8, 36, state.fontSize,
                v -> { state.fontSize = v; ta.setStyle(state.toCss()); }));
        javafx.scene.control.ColorPicker picker = new javafx.scene.control.ColorPicker(state.color);
        picker.valueProperty().addListener((o, ov, nv) -> { state.color = nv; ta.setStyle(state.toCss()); });
        javafx.scene.control.CustomMenuItem miColor = new javafx.scene.control.CustomMenuItem(picker, false);
        javafx.scene.control.CheckMenuItem miBorder = new javafx.scene.control.CheckMenuItem("Border");
        miBorder.setSelected(state.showBorder);
        miBorder.setOnAction(e -> { state.showBorder = miBorder.isSelected(); ta.setStyle(state.toCss()); });
        MenuItem miRemove = new MenuItem("Remove");
        miRemove.setOnAction(e -> removeGsTextBox(ta));
        cm.getItems().addAll(miBold, miItalic, miFontSize, new SeparatorMenuItem(),
                miColor, miBorder, new SeparatorMenuItem(), miRemove);
        return cm;
    }

    private void removeGsTextBox(TextArea ta) {
        gsCanvas.getChildren().remove(ta);
        textStyles.remove(ta);
        if (gsSelected == ta) clearGsSelection();
    }

    // ---- canvas toolbar actions ----

    @FXML private void onGsZoomFit() {
        if (gsCells.isEmpty() || gsScrollPane == null) return;
        double maxX = 0, maxY = 0;
        for (VBox c : gsCells) { maxX = Math.max(maxX, c.getLayoutX() + c.getWidth()); maxY = Math.max(maxY, c.getLayoutY() + c.getHeight()); }
        double vpW = gsScrollPane.getWidth() - 20, vpH = gsScrollPane.getHeight() - 20;
        if (maxX > 0 && maxY > 0 && vpW > 0 && vpH > 0) {
            double scale = Math.min(vpW / (maxX + 30), vpH / (maxY + 30));
            gsCanvas.setScaleX(scale); gsCanvas.setScaleY(scale);
        }
    }

    @FXML private void onGsClearCanvas() {
        gsCanvas.getChildren().clear();
        gsCells.clear(); cellPopMap.clear(); cellPlotMap.clear(); gsLinks.clear();
        textStyles.clear();
        gsCellBorder.clear();
        gsSelected = null;
        gsHandles.clear();
        gsSelectionOutline = null;
        gsSelectedArrow = null;
        arrowShapesByLink.clear();
        hideGsPopup();
        gsStatusLabel.setText("Canvas cleared.");
    }

    // ---- slider popup helper ----

    private javafx.scene.control.PopupControl gsOpenPopup;

    private void hideGsPopup() {
        if (gsOpenPopup != null) { gsOpenPopup.hide(); gsOpenPopup = null; }
    }

    private void showSliderPopup(javafx.scene.Node anchor, String title, double min, double max, double init,
                                  java.util.function.DoubleConsumer onChange) {
        hideGsPopup();
        Slider slider = new Slider(min, max, init);
        slider.setShowTickLabels(true); slider.setShowTickMarks(true);
        slider.setMajorTickUnit((max - min) / 4);
        slider.valueProperty().addListener((o, ov, nv) -> onChange.accept(nv.doubleValue()));
        VBox box = new VBox(6, new Label(title), slider);
        box.setPadding(new Insets(10));
        box.setStyle("-fx-background-color:#1A2330; -fx-background-radius:6;");
        javafx.scene.control.PopupControl popup = new javafx.scene.control.PopupControl();
        popup.getScene().setRoot(box);
        popup.setAutoHide(true);
        popup.setOnAutoHide(e -> { if (gsOpenPopup == popup) gsOpenPopup = null; });
        box.setOnKeyPressed(e -> { if (e.getCode() == javafx.scene.input.KeyCode.ESCAPE) popup.hide(); });
        gsOpenPopup = popup;
        javafx.geometry.Bounds b = anchor.localToScreen(anchor.getBoundsInLocal());
        if (b != null) popup.show(anchor, b.getMinX(), b.getMaxY() + 4);
        javafx.application.Platform.runLater(slider::requestFocus);
    }

    private static CytoPlot.Scale scaleOf(String s) {
        if ("Log".equals(s)) return CytoPlot.Scale.LOG;
        if ("Logicle".equals(s)) return CytoPlot.Scale.LOGICLE;
        if ("ArcSinh".equals(s)) return CytoPlot.Scale.ARCSINH;
        return CytoPlot.Scale.LINEAR;
    }

    @FXML
    private void onGsCopy() {
        javafx.scene.image.WritableImage img = snapCanvas();
        if (img == null) { gsStatusLabel.setText("Build the figure first."); return; }
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        gsStatusLabel.setText("Copied gating-strategy figure at " + dpi + " DPI — paste into PowerPoint.");
    }

    @FXML
    private void onGsSavePng() {
        javafx.scene.image.WritableImage img = snapCanvas();
        if (img == null) { gsStatusLabel.setText("Build the figure first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Save gating-strategy figure (PNG)");
        fc.setInitialFileName("gating_strategy.png");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"));
        File f = fc.showSaveDialog(gsCanvas.getScene().getWindow());
        if (f == null) return;
        try {
            javax.imageio.ImageIO.write(javafx.embed.swing.SwingFXUtils.fromFXImage(img, null), "png", f);
            gsStatusLabel.setText("Saved " + f.getName() + ".");
        } catch (Exception e) {
            gsStatusLabel.setText("Save failed: " + e.getMessage());
        }
    }

    /** Snapshot just the content area (panels + arrows) for Copy / Save PNG. */
    private javafx.scene.image.WritableImage snapCanvas() {
        clearGsSelection();
        if (gsCanvas == null || gsCanvas.getChildren().isEmpty()) return null;
        // Temporarily reset canvas scale for full-res snapshot
        double sx = gsCanvas.getScaleX(), sy = gsCanvas.getScaleY();
        gsCanvas.setScaleX(1); gsCanvas.setScaleY(1);
        double scale = Math.max(1.0, (ctx != null ? ctx.settings().exportDpi() : 300) / 96.0);
        javafx.scene.SnapshotParameters sp = new javafx.scene.SnapshotParameters();
        sp.setFill(javafx.scene.paint.Color.WHITE);
        sp.setTransform(javafx.scene.transform.Transform.scale(scale, scale));
        // Compute tight bounding box around content
        double maxX = 0, maxY = 0;
        for (javafx.scene.Node n : gsCanvas.getChildren()) {
            maxX = Math.max(maxX, n.getLayoutX() + n.getBoundsInParent().getWidth());
            maxY = Math.max(maxY, n.getLayoutY() + n.getBoundsInParent().getHeight());
        }
        sp.setViewport(new javafx.geometry.Rectangle2D(0, 0, maxX + 20, maxY + 20));
        javafx.scene.image.WritableImage img = gsCanvas.snapshot(sp, null);
        gsCanvas.setScaleX(sx); gsCanvas.setScaleY(sy);
        return img;
    }

    @FXML private void onGeSelectAll()  { for (SampleSel s : geSamples) s.use.set(true);  geSampleTable.refresh(); }
    @FXML private void onGeSelectNone() { for (SampleSel s : geSamples) s.use.set(false); geSampleTable.refresh(); }

    @FXML
    private void onGePlot() {
        if (ctx == null) return;
        String ch = geChannelCombo.getValue();
        if (ch == null) { geStatusLabel.setText("Pick a channel."); return; }
        List<String> selected = new ArrayList<>();
        for (SampleSel s : geSamples) if (s.use.get()) selected.add(s.name);
        if (selected.isEmpty()) { info("Select samples", "Tick at least one sample to overlay."); return; }
        geStatusLabel.setText("Loading events…");
        EventLoader.ensureLoaded(ctx, selected, geStatusLabel::setText, () -> plotOverlay(selected, gePopCombo.getValue(), ch));
    }

    private void plotOverlay(List<String> selected, String pop, String ch) {
        WorkspaceModel ws = ctx.workspace();
        String scale = geScaleCombo.getValue();
        boolean modal = geModalCheck.isSelected();
        boolean allEvents = pop == null || "All Events".equals(pop);

        // per-sample channel values (in the chosen scale space), and the pooled display range
        Map<String, double[]> series = new LinkedHashMap<>();
        double lo = Double.POSITIVE_INFINITY, hi = Double.NEGATIVE_INFINITY;
        for (String s : selected) {
            EventData d = ws.data(s);
            if (d == null || d.rows() == 0) continue;
            PopNode node = allEvents ? null : findByName(ws.treeFor(s), pop);
            EventData sub = node == null ? d : subsetFor(d, node);
            int c = sub.indexOf(ch);
            if (c < 0 || sub.rows() == 0) continue;
            double[] v = new double[sub.rows()];
            for (int r = 0; r < v.length; r++) v[r] = scaleVal(sub.get(r, c), scale);
            series.put(s, v);
            double[] pr = percentiles(v, 0.5, 99.5);
            lo = Math.min(lo, pr[0]); hi = Math.max(hi, pr[1]);
        }
        if (series.isEmpty() || !(hi > lo)) { geStatusLabel.setText("No events for that population/channel."); return; }

        int bins = 256;
        double[] x = new double[bins];
        for (int i = 0; i < bins; i++) x[i] = lo + (hi - lo) * (i + 0.5) / bins;
        geChart.clearSeries();
        geChart.setX(x);
        int idx = 0, n = series.size();
        for (Map.Entry<String, double[]> e : series.entrySet()) {
            double[] y = smooth(histogram(e.getValue(), lo, hi, bins), 3);
            if (modal) { double mx = max(y); if (mx > 0) for (int i = 0; i < y.length; i++) y[i] /= mx; }
            geChart.addSeries(shortName(e.getKey()), y, hsb(idx++, n), AnalysisChart.Kind.LINE);
        }
        geChart.setAxisLabels(ctx.aliases().label(ch) + " (" + scale + ")", modal ? "Normalized" : "Count");
        geChart.setTitle("Overlay · " + (allEvents ? "All Events" : pop) + " · " + ctx.aliases().label(ch)
                + " (" + series.size() + " samples)");
        geChart.refresh();
        geStatusLabel.setText("Overlaid " + series.size() + " sample(s). Copy or Save PNG.");
    }

    @FXML
    private void onGeCopy() {
        if (geChart.getWidth() <= 0) { geStatusLabel.setText("Plot an overlay first."); return; }
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = geChart.snapshotAtDpi(dpi);
        if (img == null) { geStatusLabel.setText("Nothing to copy."); return; }
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        geStatusLabel.setText("Copied " + dpi + " DPI overlay — paste into PowerPoint.");
    }

    @FXML
    private void onGeSavePng() {
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = geChart.snapshotAtDpi(dpi);
        if (img == null) { geStatusLabel.setText("Plot an overlay first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Save overlay (PNG)");
        fc.setInitialFileName("overlay.png");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"));
        File f = fc.showSaveDialog(geChart.getScene().getWindow());
        if (f == null) return;
        try {
            javax.imageio.ImageIO.write(javafx.embed.swing.SwingFXUtils.fromFXImage(img, null), "png", f);
            geStatusLabel.setText("Saved " + f.getName() + " at " + dpi + " DPI.");
        } catch (Exception e) {
            geStatusLabel.setText("Save failed: " + e.getMessage());
        }
    }

    // ---- graph-export helpers ----
    private static double scaleVal(double v, String scale) {
        if ("Log".equals(scale)) return Math.log10(Math.max(v, 1.0));
        if ("ArcSinh".equals(scale)) return Math.log((v / 150.0) + Math.sqrt((v / 150.0) * (v / 150.0) + 1));
        return v;
    }
    private static double[] percentiles(double[] v, double loP, double hiP) {
        double[] s = v.clone(); Arrays.sort(s);
        int lo = (int) Math.max(0, Math.min(s.length - 1, Math.round(loP / 100.0 * (s.length - 1))));
        int hi = (int) Math.max(0, Math.min(s.length - 1, Math.round(hiP / 100.0 * (s.length - 1))));
        return new double[]{s[lo], s[hi]};
    }
    private static double[] histogram(double[] v, double lo, double hi, int bins) {
        double[] h = new double[bins]; double range = hi - lo;
        if (range <= 0) return h;
        for (double x : v) {
            int b = (int) ((x - lo) / range * (bins - 1));
            if (b >= 0 && b < bins) h[b]++;
        }
        return h;
    }
    private static double[] smooth(double[] h, int r) {
        double[] out = new double[h.length];
        for (int i = 0; i < h.length; i++) {
            double s = 0; int n = 0;
            for (int j = -r; j <= r; j++) { int k = i + j; if (k >= 0 && k < h.length) { s += h[k]; n++; } }
            out[i] = n == 0 ? 0 : s / n;
        }
        return out;
    }
    private static double max(double[] a) { double m = 0; for (double x : a) m = Math.max(m, x); return m; }
    private static javafx.scene.paint.Color hsb(int i, int n) {
        return javafx.scene.paint.Color.hsb(360.0 * i / Math.max(1, n), 0.7, 0.9);
    }
    private PopNode findByName(PopNode root, String name) {
        for (PopNode n : root.selfAndDescendants()) if (!n.isRoot() && n.name().equals(name)) return n;
        return null;
    }

    @FXML private void onSelectAll()  { for (SampleSel s : samples) s.use.set(true);  sampleTable.refresh(); }
    @FXML private void onSelectNone() { for (SampleSel s : samples) s.use.set(false); sampleTable.refresh(); }

    @FXML
    private void onBuildTable() {
        if (ctx == null) return;
        List<String> selected = new ArrayList<>();
        for (SampleSel s : samples) if (s.use.get()) selected.add(s.name);
        if (selected.isEmpty()) { info("Select samples", "Tick at least one sample to export."); return; }
        if (!anyGatesFor(selected)) {
            info("Gates not detected",
                 "The selected samples have no gated populations.\n\nDraw a gating strategy and apply it "
                 + "to all samples, then build the table.");
            return;
        }
        dataStatusLabel.setText("Loading events…");
        EventLoader.ensureLoaded(ctx, selected, dataStatusLabel::setText, () -> buildNow(selected));
    }

    private void buildNow(List<String> selected) {
        WorkspaceModel ws = ctx.workspace();
        List<String> chans = new ArrayList<>(channelList.getSelectionModel().getSelectedItems());
        boolean perChan = statMedian.isSelected() || statGeoMean.isSelected() || statCV.isSelected();
        if (perChan && chans.isEmpty()) chans = new ArrayList<>(channelList.getItems());   // default: all

        // ---- header row ----
        headers = new ArrayList<>(List.of("Sample", "Population"));
        if (statParent.isSelected()) headers.add("% Parent");
        if (statTotal.isSelected())  headers.add("% Total");
        if (statCount.isSelected())  headers.add("Count");
        if (statMedian.isSelected())  for (String c : chans) headers.add("MFI " + ctx.aliases().label(c));
        if (statGeoMean.isSelected()) for (String c : chans) headers.add("GeoMean " + ctx.aliases().label(c));
        if (statCV.isSelected())      for (String c : chans) headers.add("CV% " + ctx.aliases().label(c));

        // ---- data rows ----
        rows.clear();
        int nSamples = 0;
        for (String sname : selected) {
            EventData d = ws.data(sname);
            if (d == null || d.rows() == 0) continue;
            PopNode root = ws.treeFor(sname);
            int rootCount = d.rows();
            boolean any = false;
            for (PopNode n : root.selfAndDescendants()) {
                if (n.isRoot()) continue;
                EventData sub = subsetFor(d, n);
                int count = sub.rows();
                int parentCount = (n.parent == null || n.parent.isRoot()) ? rootCount : subsetFor(d, n.parent).rows();
                List<String> row = new ArrayList<>();
                row.add(shortName(sname));
                row.add(n.name());
                if (statParent.isSelected()) row.add(fmtPct(parentCount == 0 ? 0 : 100.0 * count / parentCount));
                if (statTotal.isSelected())  row.add(fmtPct(rootCount == 0 ? 0 : 100.0 * count / rootCount));
                if (statCount.isSelected())  row.add(String.valueOf(count));
                if (statMedian.isSelected())  for (String c : chans) row.add(fmtStat(median(sub, c)));
                if (statGeoMean.isSelected()) for (String c : chans) row.add(fmtStat(geomean(sub, c)));
                if (statCV.isSelected())      for (String c : chans) row.add(fmtPct(cv(sub, c)));
                rows.add(row.toArray(new String[0]));
                any = true;
            }
            if (any) nSamples++;
        }
        rebuildColumns();
        dataStatusLabel.setText(rows.size() + " row(s) across " + nSamples + " sample(s). Copy TSV or Save CSV.");
        if (ctx.auditLog() != null) ctx.auditLog().add(AuditLog.Type.EXPORT, "",
                "Data export: " + rows.size() + " rows, " + (headers.size() - 2) + " stat column(s)");
    }

    private void rebuildColumns() {
        dataTable.getColumns().clear();
        for (int j = 0; j < headers.size(); j++) {
            final int idx = j;
            TableColumn<String[], String> col = new TableColumn<>(headers.get(j));
            col.setCellValueFactory(c -> new ReadOnlyStringWrapper(idx < c.getValue().length ? c.getValue()[idx] : ""));
            col.setPrefWidth(j < 2 ? 160 : 92);
            dataTable.getColumns().add(col);
        }
    }

    // ---- export ----
    @FXML
    private void onCopyTsv() {
        if (rows.isEmpty()) { dataStatusLabel.setText("Build the table first."); return; }
        String tsv = toDelimited("\t", false);
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putString(tsv);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        dataStatusLabel.setText("Copied " + rows.size() + " rows as TSV — paste into Excel / Prism.");
    }

    @FXML
    private void onSaveCsv() {
        if (rows.isEmpty()) { dataStatusLabel.setText("Build the table first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Save statistics CSV");
        fc.setInitialFileName("streamflow_stats.csv");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("CSV (*.csv)", "*.csv"));
        File f = fc.showSaveDialog(dataTable.getScene().getWindow());
        if (f == null) return;
        try {
            Files.writeString(f.toPath(), toDelimited(",", true));
            dataStatusLabel.setText("Saved " + f.getName() + " (" + rows.size() + " rows).");
        } catch (Exception e) {
            dataStatusLabel.setText("Save failed: " + e.getMessage());
        }
    }

    private String toDelimited(String sep, boolean csvQuote) {
        StringBuilder sb = new StringBuilder();
        sb.append(joinRow(headers.toArray(new String[0]), sep, csvQuote)).append('\n');
        for (String[] r : rows) sb.append(joinRow(r, sep, csvQuote)).append('\n');
        return sb.toString();
    }
    private static String joinRow(String[] cells, String sep, boolean csvQuote) {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < cells.length; i++) {
            if (i > 0) sb.append(sep);
            String c = cells[i] == null ? "" : cells[i];
            if (csvQuote && (c.contains(",") || c.contains("\"") || c.contains("\n")))
                c = "\"" + c.replace("\"", "\"\"") + "\"";
            sb.append(c);
        }
        return sb.toString();
    }

    // ---- stats helpers (over a population's events) ----
    private EventData subsetFor(EventData d, PopNode node) {
        boolean[] keep = new boolean[d.rows()];
        Arrays.fill(keep, true);
        for (CytoPlot.Gate g : node.chain()) {
            boolean[] m = CytoPlot.mask(d, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        return d.subset(keep);
    }
    private static double[] vals(EventData d, String ch) {
        int c = d.indexOf(ch);
        if (c < 0) return new double[0];
        double[] v = new double[d.rows()];
        for (int r = 0; r < v.length; r++) v[r] = d.get(r, c);
        return v;
    }
    private static double median(EventData d, String ch) {
        double[] v = vals(d, ch); if (v.length == 0) return Double.NaN;
        Arrays.sort(v); int n = v.length;
        return n % 2 == 1 ? v[n / 2] : (v[n / 2 - 1] + v[n / 2]) / 2;
    }
    private static double geomean(EventData d, String ch) {
        double[] v = vals(d, ch); double s = 0; int n = 0;
        for (double x : v) if (x > 0) { s += Math.log(x); n++; }
        return n == 0 ? Double.NaN : Math.exp(s / n);
    }
    private static double cv(EventData d, String ch) {
        double[] v = vals(d, ch); if (v.length == 0) return Double.NaN;
        double m = 0; for (double x : v) m += x; m /= v.length;
        double s = 0; for (double x : v) s += (x - m) * (x - m); s = Math.sqrt(s / v.length);
        return m == 0 ? 0 : 100 * s / m;
    }

    private boolean anyGatesFor(List<String> ss) {
        WorkspaceModel ws = ctx.workspace();
        for (String s : ss) {
            if (!ws.hasTree(s)) continue;
            for (PopNode n : ws.treeFor(s).selfAndDescendants()) if (!n.isRoot()) return true;
        }
        return false;
    }

    private static String fmtPct(double v) { return Double.isNaN(v) ? "" : String.format("%.2f", v); }
    private static String fmtStat(double v) {
        if (Double.isNaN(v)) return "";
        double a = Math.abs(v);
        return (a >= 1e5 || (a > 0 && a < 0.01)) ? String.format("%.2e", v) : String.format("%,.0f", v);
    }
    private static String shortName(String s) { return s.replaceAll("(?i)\\.fcs$", ""); }

    private void info(String title, String msg) {
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle(title); a.setHeaderText(title); a.setContentText(msg);
        AppIcons.theme(a, null);
        a.showAndWait();
    }

    private void setDisabled(boolean d) {
        selAllButton.setDisable(d); selNoneButton.setDisable(d);
        computeButton.setDisable(d); copyTsvButton.setDisable(d); saveCsvButton.setDisable(d);
        geSelAllButton.setDisable(d); geSelNoneButton.setDisable(d);
        gePlotButton.setDisable(d); geCopyButton.setDisable(d); geSavePngButton.setDisable(d);
        gsBuildButton.setDisable(d); gsCopyButton.setDisable(d); gsSavePngButton.setDisable(d);
    }
}
