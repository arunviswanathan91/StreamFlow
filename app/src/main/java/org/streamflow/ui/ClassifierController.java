package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.geometry.Pos;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.CheckBoxTableCell;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.image.WritableImage;
import javafx.scene.input.Clipboard;
import javafx.scene.input.ClipboardContent;
import javafx.scene.layout.HBox;
import javafx.scene.layout.StackPane;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;
import javafx.scene.text.FontWeight;
import javafx.stage.FileChooser;
import org.kordamp.ikonli.javafx.FontIcon;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.File;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Analysis module A6 — Sample Classifier (PCA + optional Random Forest).
 *
 * <p>Builds a feature vector per sample from all gated population frequencies
 * in the {@link WorkspaceModel} (gate name → % of all events), sends it to
 * the engine's {@code run_classifier}, and shows a PCA biplot. If the user
 * assigns group labels, RF feature importances are added alongside the biplot.
 *
 * <p>Only samples with cached events (opened at least once) contribute features;
 * the status line reports coverage.
 */
public class ClassifierController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    private static final Color[] PALETTE = {
        Color.web("#E41A1C"), Color.web("#377EB8"), Color.web("#4DAF4A"),
        Color.web("#984EA3"), Color.web("#FF7F00"), Color.web("#A65628"),
        Color.web("#F781BF"), Color.web("#999999")
    };

    @FXML private VBox    hintPane;
    @FXML private Button  refreshButton;
    @FXML private Button  runButton;
    @FXML private Button  copyButton;
    @FXML private Button  exportButton;
    @FXML private Button  selectAllButton;
    @FXML private Button  selectNoneButton;
    @FXML private Label   featCountLabel;
    @FXML private Label   statusLabel;
    @FXML private StackPane biplotPane;
    @FXML private Canvas    biplotCanvas;
    @FXML private TableView<SampleRow>           sampleTable;
    @FXML private TableColumn<SampleRow, Boolean> useCol;
    @FXML private TableColumn<SampleRow, String>  sampleCol;
    @FXML private TableColumn<SampleRow, String>  groupCol;
    @FXML private TableView<LoadRow>             loadingTable;
    @FXML private TableColumn<LoadRow, String>   featCol;
    @FXML private TableColumn<LoadRow, String>   pc1Col;
    @FXML private TableColumn<LoadRow, String>   pc2Col;

    private final ObservableList<SampleRow> sampleRows  = FXCollections.observableArrayList();
    private final ObservableList<LoadRow>   loadingRows = FXCollections.observableArrayList();
    private AppContext ctx;

    public static final class SampleRow {
        final String name;
        final SimpleBooleanProperty use = new SimpleBooleanProperty(true);
        final StringProperty group = new SimpleStringProperty("");
        SampleRow(String name) { this.name = name; }
    }

    public record LoadRow(String feat, String pc1, String pc2) {}

    // current biplot data for resize-redraw
    private JsonNode lastResult;

    @FXML
    public void initialize() {
        biplotPane.widthProperty().addListener((o, a, b) -> redrawIfReady());
        biplotPane.heightProperty().addListener((o, a, b) -> redrawIfReady());
        copyButton.setDisable(true); exportButton.setDisable(true);

        useCol.setCellValueFactory(c -> c.getValue().use);
        useCol.setCellFactory(CheckBoxTableCell.forTableColumn(useCol));
        useCol.setEditable(true);
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name));
        groupCol.setCellValueFactory(c -> c.getValue().group);
        groupCol.setCellFactory(TextFieldTableCell.forTableColumn());
        groupCol.setOnEditCommit(e -> e.getRowValue().group.set(
                e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(sampleRows);
        sampleTable.setEditable(true);

        featCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().feat()));
        pc1Col.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pc1()));
        pc2Col.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pc2()));
        loadingTable.setItems(loadingRows);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        rebuildSamples();
        refreshHints();
        ctx.workspace().sampleNames().addListener(
                (javafx.collections.ListChangeListener<String>) c -> refreshHints());
        ctx.workspace().addTreeChangeListener(this::refreshHints);
    }

    @FXML private void onRefresh() { rebuildSamples(); }

    @Override
    public void refreshFromWorkspace() {
        rebuildSamples();
        refreshHints();
    }

    private void rebuildSamples() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();

        Map<String, String> prevGroups = new LinkedHashMap<>();
        Map<String, Boolean> prevUse = new LinkedHashMap<>();
        for (SampleRow r : sampleRows) { prevGroups.put(r.name, r.group.get()); prevUse.put(r.name, r.use.get()); }

        sampleRows.clear();
        for (String s : ws.sampleNames()) {
            SampleRow row = new SampleRow(s);
            if (prevGroups.containsKey(s)) row.group.set(prevGroups.get(s));
            if (prevUse.containsKey(s)) row.use.set(prevUse.get(s));
            sampleRows.add(row);
        }

        // count available features
        int feats = countFeatures(ws);
        featCountLabel.setText(feats > 0 ? feats + " feature(s) from gated populations." : "");
        statusLabel.setText(sampleRows.isEmpty()
                ? "Load FCS files and draw gates first."
                : "Ready — " + sampleRows.size() + " samples, " + feats + " features.");
    }

    private int countFeatures(WorkspaceModel ws) {
        java.util.Set<String> names = new java.util.TreeSet<>();
        for (String s : ws.samples()) {
            PopNode root = ws.treeFor(s);
            for (PopNode n : root.selfAndDescendants())
                if (!n.isRoot()) names.add(n.name());
        }
        return names.size();
    }

    @FXML private void onSelectAll()  { for (SampleRow r : sampleRows) r.use.set(true);  sampleTable.refresh(); }
    @FXML private void onSelectNone() { for (SampleRow r : sampleRows) r.use.set(false); sampleTable.refresh(); }

    @FXML
    private void onRun() {
        if (ctx == null) return;
        List<String> selected = new ArrayList<>();
        for (SampleRow r : sampleRows) if (r.use.get()) selected.add(r.name);
        if (selected.size() < 2) {
            info("Select samples", "Tick at least 2 samples to run PCA.");
            return;
        }
        // gates live in the workspace tree, independent of events — check before loading anything
        if (!anyGatesFor(selected)) {
            info("Gates not detected",
                 "The selected samples have no gated populations.\n\nDraw a gating strategy in a "
                 + "graph window and apply it to all samples (Workstation → right-click → Apply to all), "
                 + "then run the classifier.");
            return;
        }
        runButton.setDisable(true);
        statusLabel.setText("Loading events for " + selected.size() + " sample(s)…");
        // events load on demand — no need to open each sample in a graph window first
        EventLoader.ensureLoaded(ctx, selected, statusLabel::setText, () -> {
            runButton.setDisable(false);
            runClassifier(selected);
        });
    }

    /** True if any of the given samples has at least one gate in its workspace tree. */
    private boolean anyGatesFor(List<String> samples) {
        WorkspaceModel ws = ctx.workspace();
        for (String s : samples) {
            if (!ws.hasTree(s)) continue;
            for (PopNode n : ws.treeFor(s).selfAndDescendants()) if (!n.isRoot()) return true;
        }
        return false;
    }

    private void runClassifier(List<String> selected) {
        WorkspaceModel ws = ctx.workspace();
        // Build feature matrix: {sample: {gate_name: frequency}}
        ObjectNode features = JSON.createObjectNode();
        int computed = 0;
        for (String sname : selected) {
            EventData evData = ws.data(sname);
            if (evData == null || evData.rows() == 0) continue;
            PopNode root = ws.treeFor(sname);
            ObjectNode featVec = JSON.createObjectNode();
            for (PopNode n : root.selfAndDescendants()) {
                if (n.isRoot()) continue;
                boolean[] keep = new boolean[evData.rows()];
                Arrays.fill(keep, true);
                for (CytoPlot.Gate g : n.chain()) {
                    boolean[] m = CytoPlot.mask(evData, g);
                    for (int k = 0; k < keep.length; k++) keep[k] = keep[k] && m[k];
                }
                int cnt = 0; for (boolean b : keep) if (b) cnt++;
                featVec.put(n.name(), 100.0 * cnt / evData.rows());
            }
            if (featVec.size() > 0) { features.set(sname, featVec); computed++; }
        }

        if (computed < 2) {
            info("Gates not detected",
                 "Fewer than 2 of the selected samples have gated populations.\n\nApply your gating "
                 + "strategy to all selected samples, then run again.");
            statusLabel.setText("Need ≥2 samples with gates.");
            return;
        }

        ObjectNode grpNode = JSON.createObjectNode();
        for (SampleRow r : sampleRows)
            if (r.use.get() && !r.group.get().isBlank()) grpNode.put(r.name, r.group.get().trim());

        ObjectNode a = JSON.createObjectNode();
        a.set("features", features);
        if (grpNode.size() > 0) a.set("group_labels", grpNode);

        statusLabel.setText("Running PCA on " + computed + " samples…");
        ctx.jobs().run(ctx.bridge().command("run_classifier", a), this::showResult);
    }

    /** Simple modal info dialog with a single OK button. */
    private void info(String title, String msg) {
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle(title);
        a.setHeaderText(title);
        a.setContentText(msg);
        AppIcons.theme(a, null);
        a.showAndWait();
    }

    private void showResult(JsonNode result) {
        loadingRows.clear();
        for (JsonNode l : result.path("loadings"))
            loadingRows.add(new LoadRow(l.path("feat").asText(),
                    String.format("%.3f", l.path("pc1").asDouble()),
                    String.format("%.3f", l.path("pc2").asDouble())));

        lastResult = result;
        redrawIfReady();

        JsonNode ve = result.path("var_explained");
        double pc1pct = ve.size() > 0 ? ve.get(0).asDouble() * 100 : 0;
        double pc2pct = ve.size() > 1 ? ve.get(1).asDouble() * 100 : 0;
        ctx.auditLog().add(AuditLog.Type.ANALYSIS, "",
                String.format("Classifier (%s): %d samples × %d features",
                        result.path("method").asText(),
                        result.path("n_samples").asInt(),
                        result.path("n_features").asInt()));
        statusLabel.setText(String.format("%s — %d samples × %d features. PC1+PC2 = %.1f%%",
                result.path("method").asText(),
                result.path("n_samples").asInt(),
                result.path("n_features").asInt(),
                pc1pct + pc2pct));
        copyButton.setDisable(false); exportButton.setDisable(false);
    }

    private void redrawIfReady() {
        if (lastResult == null) return;
        double W = biplotPane.getWidth(), H = biplotPane.getHeight();
        if (W < 10 || H < 10) return;
        biplotCanvas.setWidth(W); biplotCanvas.setHeight(H);
        drawBiplot(biplotCanvas.getGraphicsContext2D(), W, H);
    }

    private void drawBiplot(GraphicsContext g, double W, double H) {
        g.clearRect(0, 0, W, H);
        g.setFill(Color.WHITE); g.fillRect(0, 0, W, H);

        JsonNode comps    = lastResult.path("components");
        JsonNode loads    = lastResult.path("loadings");
        JsonNode topIdx   = lastResult.path("top_loading_indices");
        JsonNode ve       = lastResult.path("var_explained");
        double pc1pct = ve.size() > 0 ? ve.get(0).asDouble() * 100 : 0;
        double pc2pct = ve.size() > 1 ? ve.get(1).asDouble() * 100 : 0;

        if (comps.size() == 0) return;

        double ML = 56, MR = 16, MT = 32, MB = 46;
        double pw = W - ML - MR, ph = H - MT - MB;
        if (pw < 10 || ph < 10) return;

        // data extents
        double xMin = Double.MAX_VALUE, xMax = -Double.MAX_VALUE;
        double yMin = Double.MAX_VALUE, yMax = -Double.MAX_VALUE;
        for (JsonNode c : comps) {
            double x = c.path("x").asDouble(), y = c.path("y").asDouble();
            xMin = Math.min(xMin, x); xMax = Math.max(xMax, x);
            yMin = Math.min(yMin, y); yMax = Math.max(yMax, y);
        }
        // include loading arrows in extents
        if (topIdx.size() > 0 && loads.size() > 0) {
            for (JsonNode idx : topIdx) {
                int i = idx.asInt();
                if (i >= loads.size()) continue;
                double lx = loads.get(i).path("pc1").asDouble();
                double ly = loads.get(i).path("pc2").asDouble();
                xMin = Math.min(xMin, lx); xMax = Math.max(xMax, lx);
                yMin = Math.min(yMin, ly); yMax = Math.max(yMax, ly);
            }
        }
        double xRange = Math.max(1e-9, xMax - xMin), yRange = Math.max(1e-9, yMax - yMin);
        double pad = 0.12;
        xMin -= xRange * pad; xMax += xRange * pad; xRange *= (1 + 2 * pad);
        yMin -= yRange * pad; yMax += yRange * pad; yRange *= (1 + 2 * pad);

        // grid
        g.setStroke(Color.web("#EEEEEE")); g.setLineWidth(0.5);
        double zero_x = ML + (0 - xMin) / xRange * pw;
        double zero_y = MT + ph - (0 - yMin) / yRange * ph;
        if (zero_x >= ML && zero_x <= ML + pw) g.strokeLine(zero_x, MT, zero_x, MT + ph);
        if (zero_y >= MT && zero_y <= MT + ph) g.strokeLine(ML, zero_y, ML + pw, zero_y);

        // loading arrows (top 8)
        if (topIdx.size() > 0 && loads.size() > 0) {
            for (JsonNode idx : topIdx) {
                int i = idx.asInt();
                if (i >= loads.size()) continue;
                double lx = loads.get(i).path("pc1").asDouble();
                double ly = loads.get(i).path("pc2").asDouble();
                double px = ML + (lx - xMin) / xRange * pw;
                double py = MT + ph - (ly - yMin) / yRange * ph;
                double ox = ML + (0 - xMin) / xRange * pw;
                double oy = MT + ph - (0 - yMin) / yRange * ph;
                g.setStroke(Color.web("#AAAAAA")); g.setLineWidth(1.2);
                g.strokeLine(ox, oy, px, py);
                // arrowhead
                double ang = Math.atan2(py - oy, px - ox);
                g.setFill(Color.web("#AAAAAA"));
                g.fillPolygon(
                    new double[]{px, px - 8 * Math.cos(ang - 0.4), px - 8 * Math.cos(ang + 0.4)},
                    new double[]{py, py - 8 * Math.sin(ang - 0.4), py - 8 * Math.sin(ang + 0.4)}, 3);
                g.setFont(Font.font("Segoe UI", 8.5));
                g.setFill(Color.web("#888888"));
                g.fillText(loads.get(i).path("feat").asText(), px + 3, py - 3);
            }
        }

        // build group → color map
        Map<String, Color> groupColors = new LinkedHashMap<>();
        int gi = 0;
        for (JsonNode c : comps) {
            String grp = c.path("group").asText();
            if (!grp.isEmpty() && !groupColors.containsKey(grp))
                groupColors.put(grp, PALETTE[gi++ % PALETTE.length]);
        }

        // sample dots + labels
        for (JsonNode c : comps) {
            String grp = c.path("group").asText();
            Color col = grp.isEmpty() ? Color.web("#08306B") : groupColors.getOrDefault(grp, Color.web("#08306B"));
            double px = ML + (c.path("x").asDouble() - xMin) / xRange * pw;
            double py = MT + ph - (c.path("y").asDouble() - yMin) / yRange * ph;
            g.setFill(col); g.fillOval(px - 5, py - 5, 10, 10);
            g.setFill(Color.web("#222222")); g.setFont(Font.font("Segoe UI", 9));
            g.fillText(c.path("label").asText(), px + 7, py + 4);
        }

        // frame
        g.setStroke(Color.web("#666666")); g.setLineWidth(1);
        g.strokeRect(ML, MT, pw, ph);

        // axis ticks
        g.setFont(Font.font("Segoe UI", 9)); g.setFill(Color.web("#555555"));
        for (int k = 0; k <= 4; k++) {
            double xv = xMin + k / 4.0 * xRange;
            g.fillText(fmt(xv), ML + k / 4.0 * pw - 10, MT + ph + 14);
        }
        for (int k = 0; k <= 4; k++) {
            double yv = yMin + k / 4.0 * yRange;
            g.fillText(fmt(yv), 2, MT + ph - k / 4.0 * ph + 4);
        }

        // axis labels
        String xLbl = String.format("PC1 (%.1f%%)", pc1pct);
        String yLbl = String.format("PC2 (%.1f%%)", pc2pct);
        g.setFont(Font.font("Segoe UI", 11)); g.setFill(Color.web("#222222"));
        g.fillText(xLbl, ML + pw / 2 - xLbl.length() * 3.5, MT + ph + 34);
        g.save(); g.translate(12, MT + ph / 2 + yLbl.length() * 3.5); g.rotate(-90);
        g.fillText(yLbl, 0, 0); g.restore();

        // title + legend
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 12));
        g.setFill(Color.web("#1A1A1A"));
        g.fillText("PCA Biplot", ML, 22);
        if (!groupColors.isEmpty()) {
            double lx = ML + pw - 120, ly = MT + 10;
            g.setFont(Font.font("Segoe UI", 10));
            for (Map.Entry<String, Color> e : groupColors.entrySet()) {
                g.setFill(e.getValue()); g.fillOval(lx, ly - 7, 9, 9);
                g.setFill(Color.web("#222222")); g.fillText(e.getKey(), lx + 13, ly + 1);
                ly += 16;
            }
        }
    }

    private static String fmt(double v) {
        double a = Math.abs(v);
        if (a >= 10) return String.format("%.1f", v);
        if (a >= 0.1) return String.format("%.2f", v);
        return String.format("%.3f", v);
    }

    @FXML private void onCopy() {
        WritableImage img = biplotCanvas.snapshot(null, null);
        ClipboardContent cc = new ClipboardContent(); cc.putImage(img);
        Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied to clipboard.");
    }

    @FXML private void onExport() {
        FileChooser fc = new FileChooser();
        fc.setTitle("Export Biplot");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG Image", "*.png"));
        File f = fc.showSaveDialog(biplotCanvas.getScene().getWindow());
        if (f == null) return;
        try {
            WritableImage img = biplotCanvas.snapshot(null, null);
            BufferedImage bi = javafx.embed.swing.SwingFXUtils.fromFXImage(img, null);
            ImageIO.write(bi, "png", f);
            statusLabel.setText("Saved: " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("Export failed: " + e.getMessage());
        }
    }

    private void setDisabled(boolean d) {
        refreshButton.setDisable(d);
        runButton.setDisable(d);
        selectAllButton.setDisable(d);
        selectNoneButton.setDisable(d);
    }

    // ---- guided UX hints ----------------------------------------------------

    private void refreshHints() {
        if (hintPane == null || ctx == null) return;
        hintPane.getChildren().clear();
        WorkspaceModel ws = ctx.workspace();
        boolean hasFcs   = ws.sampleNames().size() >= 2;
        boolean hasGates = !ws.sampleNames().isEmpty() && anyGatesInWorkspace(ws);

        if (hasFcs && hasGates) {
            hintPane.setVisible(false);
            hintPane.setManaged(false);
            return;
        }

        int n = ws.sampleNames().size();
        if (n == 0) {
            hintPane.getChildren().add(hintRow(false,
                    "No FCS files loaded — use File ▸ Load FCS… to load your data.",
                    "Workstation"));
        } else if (n == 1) {
            hintPane.getChildren().add(hintRow(false,
                    "Only 1 sample loaded — PCA needs at least 2 samples.", "Workstation"));
        } else {
            hintPane.getChildren().add(hintRow(true, n + " samples loaded.", null));
        }

        if (n > 0 && !hasGates) {
            hintPane.getChildren().add(hintRow(false,
                    "No gated populations detected — draw a gating strategy and apply it to all samples.",
                    "Workstation"));
            hintPane.getChildren().add(hintInfoRow(
                    "Tip: Workstation → right-click a sample → Apply to all samples."));
        }

        hintPane.setVisible(true);
        hintPane.setManaged(true);
    }

    private boolean anyGatesInWorkspace(WorkspaceModel ws) {
        for (String s : ws.sampleNames()) {
            if (!ws.hasTree(s)) continue;
            for (PopNode nd : ws.treeFor(s).selfAndDescendants()) if (!nd.isRoot()) return true;
        }
        return false;
    }

    private HBox hintRow(boolean ok, String text, String navTarget) {
        HBox row = new HBox(8);
        row.setAlignment(Pos.CENTER_LEFT);
        row.getStyleClass().add(ok ? "hint-ok" : "hint-missing");
        FontIcon icon = new FontIcon(ok ? "fas-check-circle" : "fas-times-circle");
        icon.setIconSize(14);
        row.getChildren().addAll(icon, new Label(text));
        if (!ok && navTarget != null) {
            Label link = new Label("Go to " + navTarget + " →");
            link.getStyleClass().add("hint-link");
            link.setOnMouseClicked(e -> ctx.navigator().accept(navTarget));
            row.getChildren().add(link);
        }
        return row;
    }

    private HBox hintInfoRow(String text) {
        HBox row = new HBox(8);
        row.setAlignment(Pos.CENTER_LEFT);
        row.getStyleClass().add("hint-info");
        FontIcon icon = new FontIcon("fas-info-circle");
        icon.setIconSize(14);
        Label lbl = new Label(text);
        lbl.setWrapText(true);
        row.getChildren().addAll(icon, lbl);
        return row;
    }
}
