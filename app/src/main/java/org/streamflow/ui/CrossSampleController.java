package org.streamflow.ui;

import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.geometry.Pos;
import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableCell;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.CheckBoxTableCell;
import javafx.scene.layout.HBox;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;
import javafx.scene.text.TextAlignment;
import org.kordamp.ikonli.javafx.FontIcon;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;

/**
 * Cross-Sample analysis (differentiators #7 radar, #11 MFI drift, #14 consistency, #15 matrix).
 * Pure-JavaFX: reads the shared {@link WorkspaceModel} (samples with cached events + their gating
 * trees), computes each gate's frequency per sample, and presents four views. Only samples that
 * have been opened (events cached) contribute.
 */
public class CrossSampleController implements ContextAware, Refreshable {

    @FXML private VBox hintPane;
    @FXML private Button refreshButton;
    @FXML private Button selectAllButton;
    @FXML private Button selectNoneButton;
    @FXML private Label statusLabel;
    @FXML private TableView<SampleSel> sampleSelectTable;
    @FXML private TableColumn<SampleSel, Boolean> selUseCol;
    @FXML private TableColumn<SampleSel, String> selNameCol;
    @FXML private TableView<GateRow> matrixTable;
    @FXML private TableView<ConsRow> consistencyTable;
    @FXML private TableColumn<ConsRow, String> cGateCol, cMeanCol, cSdCol, cCvCol, cFlagCol;
    @FXML private Canvas radarCanvas;
    @FXML private ComboBox<String> driftGateCombo;
    @FXML private TableView<DriftRow> driftTable;

    private final ObservableList<GateRow> matrixRows = FXCollections.observableArrayList();
    private final ObservableList<ConsRow> consRows = FXCollections.observableArrayList();
    private final ObservableList<DriftRow> driftRows = FXCollections.observableArrayList();
    private final ObservableList<SampleSel> sampleSel = FXCollections.observableArrayList();

    private AppContext ctx;
    private List<String> sampleList = new ArrayList<>();   // selected + loaded samples
    private List<String> gateList = new ArrayList<>();      // union of gate names

    /** A selectable sample row in the left-hand picker. */
    public static final class SampleSel {
        final String name;
        final SimpleBooleanProperty use = new SimpleBooleanProperty(true);
        SampleSel(String name) { this.name = name; }
    }

    /** A gate's frequency across all samples (aligned to {@link #sampleList}). */
    public static final class GateRow {
        final String gate; final double[] freqs; final double mean, sd;
        GateRow(String gate, double[] freqs) {
            this.gate = gate; this.freqs = freqs;
            double m = 0; int n = 0;
            for (double f : freqs) if (!Double.isNaN(f)) { m += f; n++; }
            this.mean = n == 0 ? 0 : m / n;
            double s = 0; for (double f : freqs) if (!Double.isNaN(f)) s += (f - mean) * (f - mean);
            this.sd = n > 1 ? Math.sqrt(s / (n - 1)) : 0;
        }
    }

    public record ConsRow(String gate, String mean, String sd, String cv, String flag) {}

    /** A sample's per-channel MFI for the selected gate (aligned to fluor-channel list). */
    public static final class DriftRow {
        final String sample; final double[] mfis;
        DriftRow(String sample, double[] mfis) { this.sample = sample; this.mfis = mfis; }
    }

    @FXML
    public void initialize() {
        selUseCol.setCellValueFactory(c -> c.getValue().use);
        selUseCol.setCellFactory(CheckBoxTableCell.forTableColumn(selUseCol));
        selUseCol.setEditable(true);
        selNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(shortName(c.getValue().name)));
        sampleSelectTable.setItems(sampleSel);
        sampleSelectTable.setEditable(true);
        cGateCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().gate()));
        cMeanCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().mean()));
        cSdCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sd()));
        cCvCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().cv()));
        cFlagCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().flag()));
        cFlagCol.setCellFactory(col -> new TableCell<>() {
            @Override protected void updateItem(String v, boolean empty) {
                super.updateItem(v, empty);
                setText(empty ? null : v);
                setStyle(empty || v == null ? "" :
                        v.startsWith("⚠") ? "-fx-text-fill:#C0392B;-fx-font-weight:bold;"
                                          : "-fx-text-fill:#27AE60;");
            }
        });
        consistencyTable.setItems(consRows);
        driftTable.setItems(driftRows);
        driftGateCombo.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> {
            if (b != null) buildDriftTab(b);
        });
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        syncSampleSelList();
        clearViews("Tick samples on the left, then click Compute.");
        refreshHints();
        ctx.workspace().sampleNames().addListener(
                (javafx.collections.ListChangeListener<String>) c -> refreshHints());
        ctx.workspace().addTreeChangeListener(this::refreshHints);
    }

    @FXML private void onRefresh() { compute(); }

    @Override
    public void refreshFromWorkspace() {
        syncSampleSelList();
        refreshHints();
    }
    @FXML private void onSelectAll()  { for (SampleSel s : sampleSel) s.use.set(true);  sampleSelectTable.refresh(); }
    @FXML private void onSelectNone() { for (SampleSel s : sampleSel) s.use.set(false); sampleSelectTable.refresh(); }

    /** Rebuild the picker from the workspace sample list, preserving existing tick state. */
    private void syncSampleSelList() {
        if (ctx == null) return;
        Map<String, Boolean> prev = new LinkedHashMap<>();
        for (SampleSel s : sampleSel) prev.put(s.name, s.use.get());
        sampleSel.clear();
        for (String s : ctx.workspace().sampleNames()) {
            SampleSel row = new SampleSel(s);
            if (prev.containsKey(s)) row.use.set(prev.get(s));
            sampleSel.add(row);
        }
    }

    private void clearViews(String status) {
        matrixRows.clear(); consRows.clear(); driftRows.clear();
        matrixTable.getColumns().clear();
        driftTable.getColumns().clear();
        sampleList = new ArrayList<>(); gateList = new ArrayList<>();
        drawRadar();
        statusLabel.setText(status);
    }

    /** Gather the ticked samples, load their events on demand, then build the four views. */
    private void compute() {
        if (ctx == null) return;
        syncSampleSelList();
        List<String> selected = new ArrayList<>();
        for (SampleSel s : sampleSel) if (s.use.get()) selected.add(s.name);
        if (selected.size() < 1) { clearViews("Tick at least one sample, then Compute."); return; }
        if (!anyGatesFor(selected)) {
            info("Gates not detected",
                 "The selected samples have no gated populations.\n\nDraw a gating strategy in a graph "
                 + "window and apply it to all samples (Workstation → right-click → Apply to all), then Compute.");
            clearViews("No gates on the selected samples.");
            return;
        }
        statusLabel.setText("Loading events for " + selected.size() + " sample(s)…");
        EventLoader.ensureLoaded(ctx, selected, statusLabel::setText, () -> computeViews(selected));
    }

    private boolean anyGatesFor(List<String> samples) {
        WorkspaceModel ws = ctx.workspace();
        for (String s : samples) {
            if (!ws.hasTree(s)) continue;
            for (PopNode n : ws.treeFor(s).selfAndDescendants()) if (!n.isRoot()) return true;
        }
        return false;
    }

    /** Build the matrix/consistency/radar/drift views over the (now loaded) selected samples. */
    private void computeViews(List<String> selected) {
        WorkspaceModel ws = ctx.workspace();
        sampleList = new ArrayList<>();
        for (String s : selected) {
            EventData d = ws.data(s);
            if (d != null && d.rows() > 0) sampleList.add(s);
        }
        TreeSet<String> gates = new TreeSet<>();
        for (String s : sampleList) {
            PopNode root = ws.treeFor(s);
            for (PopNode n : root.selfAndDescendants()) if (!n.isRoot()) gates.add(n.name());
        }
        gateList = new ArrayList<>(gates);

        if (sampleList.isEmpty() || gateList.isEmpty()) {
            clearViews("No gated populations on the selected samples.");
            return;
        }

        Map<String, double[]> freqByGate = new LinkedHashMap<>();
        for (String g : gateList) freqByGate.put(g, new double[sampleList.size()]);
        for (int si = 0; si < sampleList.size(); si++) {
            String s = sampleList.get(si);
            EventData d = ws.data(s);
            PopNode root = ws.treeFor(s);
            for (String g : gateList) {
                PopNode node = findByName(root, g);
                freqByGate.get(g)[si] = (node == null) ? Double.NaN : freqOf(d, node);
            }
        }

        buildMatrixTab(freqByGate);
        buildConsistencyTab(freqByGate);
        drawRadar();

        driftGateCombo.setItems(FXCollections.observableArrayList(gateList));
        if (!gateList.isEmpty()) driftGateCombo.getSelectionModel().selectFirst();

        statusLabel.setText(String.format("%d sample(s) × %d gate(s).", sampleList.size(), gateList.size()));
    }

    /** Simple modal info dialog with a single OK button. */
    private void info(String title, String msg) {
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle(title); a.setHeaderText(title); a.setContentText(msg);
        a.showAndWait();
    }

    // ---- #15 gate × sample matrix -------------------------------------------
    private void buildMatrixTab(Map<String, double[]> freqByGate) {
        List<TableColumn<GateRow, ?>> cols = new ArrayList<>();
        TableColumn<GateRow, String> gcol = new TableColumn<>("Gate");
        gcol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().gate));
        gcol.setPrefWidth(180);
        cols.add(gcol);
        for (int si = 0; si < sampleList.size(); si++) {
            final int idx = si;
            TableColumn<GateRow, Number> col = new TableColumn<>(shortName(sampleList.get(si)));
            col.setPrefWidth(90);
            col.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().freqs[idx]));
            col.setCellFactory(c -> new TableCell<>() {
                @Override protected void updateItem(Number v, boolean empty) {
                    super.updateItem(v, empty);
                    if (empty || v == null || Double.isNaN(v.doubleValue())) { setText(null); setStyle(""); return; }
                    GateRow row = getTableView().getItems().get(getIndex());
                    setText(String.format("%.1f", v.doubleValue()));
                    double z = row.sd > 1e-9 ? (v.doubleValue() - row.mean) / row.sd : 0;
                    Color bg = divergingColor(z);
                    setStyle("-fx-background-color:" + web(bg) + ";"
                            + (z > 1.4 || z < -1.4 ? "-fx-text-fill:white;" : ""));
                }
            });
            cols.add(col);
        }
        matrixTable.getColumns().setAll(cols);
        matrixRows.clear();
        for (String g : gateList) matrixRows.add(new GateRow(g, freqByGate.get(g)));
        matrixTable.setItems(matrixRows);
    }

    // ---- #14 consistency checker --------------------------------------------
    private void buildConsistencyTab(Map<String, double[]> freqByGate) {
        consRows.clear();
        for (String g : gateList) {
            GateRow gr = new GateRow(g, freqByGate.get(g));
            double cv = gr.mean > 1e-9 ? 100.0 * gr.sd / gr.mean : 0;
            String flag = cv > 50 ? "⚠ inconsistent" : "consistent";
            consRows.add(new ConsRow(g,
                    String.format("%.2f", gr.mean), String.format("%.2f", gr.sd),
                    String.format("%.1f", cv), flag));
        }
    }

    // ---- #7 population radar -------------------------------------------------
    private void drawRadar() {
        GraphicsContext g = radarCanvas.getGraphicsContext2D();
        double W = radarCanvas.getWidth(), H = radarCanvas.getHeight();
        g.clearRect(0, 0, W, H);
        g.setFill(Color.web("#FFFFFF")); g.fillRect(0, 0, W, H);
        int k = Math.min(gateList.size(), 12);
        if (k < 3 || sampleList.isEmpty()) {
            g.setFill(Color.web("#888")); g.setFont(Font.font(13));
            g.fillText("Need ≥3 gates across opened samples for a radar.", 24, H / 2);
            return;
        }
        double cx = W / 2, cy = H / 2 + 6, R = Math.min(W, H) / 2 - 70;

        // per-axis max (across samples) for normalisation
        double[] axisMax = new double[k];
        for (int ai = 0; ai < k; ai++) {
            double mx = 0;
            for (int si = 0; si < sampleList.size(); si++) {
                double v = freqAt(gateList.get(ai), si);
                if (!Double.isNaN(v)) mx = Math.max(mx, v);
            }
            axisMax[ai] = mx <= 0 ? 1 : mx;
        }
        // grid rings + axis spokes + labels
        g.setStroke(Color.web("#DDD")); g.setLineWidth(1);
        for (int ring = 1; ring <= 4; ring++) {
            double rr = R * ring / 4.0;
            g.beginPath();
            for (int ai = 0; ai <= k; ai++) {
                double a = -Math.PI / 2 + 2 * Math.PI * ai / k;
                double px = cx + rr * Math.cos(a), py = cy + rr * Math.sin(a);
                if (ai == 0) g.moveTo(px, py); else g.lineTo(px, py);
            }
            g.stroke();
        }
        g.setFill(Color.web("#444")); g.setFont(Font.font(10)); g.setTextAlign(TextAlignment.CENTER);
        for (int ai = 0; ai < k; ai++) {
            double a = -Math.PI / 2 + 2 * Math.PI * ai / k;
            g.setStroke(Color.web("#DDD"));
            g.strokeLine(cx, cy, cx + R * Math.cos(a), cy + R * Math.sin(a));
            g.fillText(shortName(gateList.get(ai)),
                    cx + (R + 22) * Math.cos(a), cy + (R + 22) * Math.sin(a));
        }
        // one translucent polygon per sample
        Color[] pal = palette(sampleList.size());
        for (int si = 0; si < sampleList.size(); si++) {
            double[] xs = new double[k], ys = new double[k];
            for (int ai = 0; ai < k; ai++) {
                double v = freqAt(gateList.get(ai), si);
                double frac = Double.isNaN(v) ? 0 : v / axisMax[ai];
                double a = -Math.PI / 2 + 2 * Math.PI * ai / k;
                xs[ai] = cx + R * frac * Math.cos(a);
                ys[ai] = cy + R * frac * Math.sin(a);
            }
            Color c = pal[si];
            g.setStroke(c); g.setLineWidth(1.8);
            g.setFill(Color.color(c.getRed(), c.getGreen(), c.getBlue(), 0.12));
            g.fillPolygon(xs, ys, k); g.strokePolygon(xs, ys, k);
        }
        // legend
        g.setFont(Font.font(10)); g.setTextAlign(TextAlignment.LEFT);
        for (int si = 0; si < sampleList.size() && si < 14; si++) {
            g.setFill(pal[si]); g.fillRect(8, 12 + si * 14, 10, 10);
            g.setFill(Color.web("#333")); g.fillText(shortName(sampleList.get(si)), 22, 21 + si * 14);
        }
    }

    private double freqAt(String gate, int sampleIdx) {
        for (GateRow gr : matrixRows) if (gr.gate.equals(gate)) return gr.freqs[sampleIdx];
        return Double.NaN;
    }

    // ---- #11 MFI drift heatmap ----------------------------------------------
    private void buildDriftTab(String gate) {
        if (ctx == null || gate == null) return;
        WorkspaceModel ws = ctx.workspace();
        // fluorescence channels from the panel
        List<String> channels = new ArrayList<>(ws.channelNames());
        channels.removeIf(c -> c.toLowerCase().matches(".*(fsc|ssc|time).*"));
        if (channels.isEmpty()) channels = new ArrayList<>(ws.channelNames());

        driftRows.clear();
        List<TableColumn<DriftRow, ?>> cols = new ArrayList<>();
        TableColumn<DriftRow, String> scol = new TableColumn<>("Sample");
        scol.setCellValueFactory(c -> new ReadOnlyStringWrapper(shortName(c.getValue().sample)));
        scol.setPrefWidth(160);
        cols.add(scol);

        // compute MFI matrix sample × channel
        final List<String> fchan = channels;
        double[][] mfi = new double[sampleList.size()][fchan.size()];
        for (int si = 0; si < sampleList.size(); si++) {
            EventData d = ws.data(sampleList.get(si));
            PopNode node = findByName(ws.treeFor(sampleList.get(si)), gate);
            EventData sub = (node == null) ? d : subsetFor(d, node);
            for (int ci = 0; ci < fchan.size(); ci++) {
                int col = sub.indexOf(fchan.get(ci));
                mfi[si][ci] = (col < 0) ? Double.NaN : median(sub, col);
            }
        }
        // per-column stats for z-score colouring
        double[] cmean = new double[fchan.size()], csd = new double[fchan.size()];
        for (int ci = 0; ci < fchan.size(); ci++) {
            double m = 0; int n = 0;
            for (int si = 0; si < sampleList.size(); si++) if (!Double.isNaN(mfi[si][ci])) { m += mfi[si][ci]; n++; }
            cmean[ci] = n == 0 ? 0 : m / n;
            double s = 0; for (int si = 0; si < sampleList.size(); si++) if (!Double.isNaN(mfi[si][ci])) s += Math.pow(mfi[si][ci] - cmean[ci], 2);
            csd[ci] = n > 1 ? Math.sqrt(s / (n - 1)) : 0;
        }
        for (int ci = 0; ci < fchan.size(); ci++) {
            final int idx = ci;
            TableColumn<DriftRow, Number> col = new TableColumn<>(ctx.aliases().label(fchan.get(ci)));
            col.setPrefWidth(90);
            col.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().mfis[idx]));
            col.setCellFactory(c -> new TableCell<>() {
                @Override protected void updateItem(Number v, boolean empty) {
                    super.updateItem(v, empty);
                    if (empty || v == null || Double.isNaN(v.doubleValue())) { setText(null); setStyle(""); return; }
                    setText(fmtMfi(v.doubleValue()));
                    double z = csd[idx] > 1e-9 ? (v.doubleValue() - cmean[idx]) / csd[idx] : 0;
                    Color bg = divergingColor(z);
                    setStyle("-fx-background-color:" + web(bg) + ";" + (z > 1.4 || z < -1.4 ? "-fx-text-fill:white;" : ""));
                }
            });
            cols.add(col);
        }
        driftTable.getColumns().setAll(cols);
        for (int si = 0; si < sampleList.size(); si++) driftRows.add(new DriftRow(sampleList.get(si), mfi[si]));
    }

    // ---- shared helpers -----------------------------------------------------
    private double freqOf(EventData d, PopNode node) {
        boolean[] keep = new boolean[d.rows()];
        Arrays.fill(keep, true);
        for (CytoPlot.Gate g : node.chain()) {
            boolean[] m = CytoPlot.mask(d, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        int c = 0; for (boolean b : keep) if (b) c++;
        return d.rows() == 0 ? 0 : 100.0 * c / d.rows();
    }

    private EventData subsetFor(EventData d, PopNode node) {
        boolean[] keep = new boolean[d.rows()];
        Arrays.fill(keep, true);
        for (CytoPlot.Gate g : node.chain()) {
            boolean[] m = CytoPlot.mask(d, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        return d.subset(keep);
    }

    private static double median(EventData d, int col) {
        int n = d.rows(); if (n == 0) return Double.NaN;
        double[] v = new double[n];
        for (int r = 0; r < n; r++) v[r] = d.get(r, col);
        Arrays.sort(v);
        return n % 2 == 1 ? v[n / 2] : (v[n / 2 - 1] + v[n / 2]) / 2;
    }

    private PopNode findByName(PopNode root, String name) {
        for (PopNode n : root.selfAndDescendants()) if (!n.isRoot() && n.name().equals(name)) return n;
        return null;
    }

    private static String shortName(String s) {
        String n = s.replaceAll("(?i)\\.fcs$", "");
        return n.length() > 16 ? n.substring(0, 15) + "…" : n;
    }
    private static String fmtMfi(double v) {
        double a = Math.abs(v);
        if (a >= 1e4) return String.format("%.0f", v);
        if (a >= 100) return String.format("%.0f", v);
        return String.format("%.1f", v);
    }

    /** Diverging blue→white→red by z-score (clamped to ±2). */
    private static Color divergingColor(double z) {
        double t = Math.max(-2, Math.min(2, z)) / 2.0;   // -1..1
        if (t < 0) return Color.web("#3B6FB6").interpolate(Color.WHITE, 1 + t);
        return Color.WHITE.interpolate(Color.web("#C0392B"), t);
    }
    private static String web(Color c) {
        return String.format("#%02X%02X%02X",
                (int) (c.getRed() * 255), (int) (c.getGreen() * 255), (int) (c.getBlue() * 255));
    }
    private static Color[] palette(int n) {
        Color[] p = new Color[Math.max(1, n)];
        for (int i = 0; i < p.length; i++) p[i] = Color.hsb(360.0 * i / Math.max(1, p.length), 0.65, 0.85);
        return p;
    }

    private void setDisabled(boolean d) {
        refreshButton.setDisable(d);
        driftGateCombo.setDisable(d);
        selectAllButton.setDisable(d);
        selectNoneButton.setDisable(d);
    }

    // ---- guided UX hints ----------------------------------------------------

    private void refreshHints() {
        if (hintPane == null || ctx == null) return;
        hintPane.getChildren().clear();
        WorkspaceModel ws = ctx.workspace();
        boolean hasFcs   = !ws.sampleNames().isEmpty();
        boolean hasGates = hasFcs && anyGatesInWorkspace(ws);

        if (hasFcs && hasGates) {
            hintPane.setVisible(false);
            hintPane.setManaged(false);
            return;
        }

        if (!hasFcs) {
            hintPane.getChildren().add(hintRow(false,
                    "No FCS files loaded — use File ▸ Load FCS… to load your data.",
                    "Workstation"));
        } else {
            hintPane.getChildren().add(hintRow(true,
                    ws.sampleNames().size() + " sample(s) loaded.", null));
            hintPane.getChildren().add(hintRow(false,
                    "No gates drawn — open a sample in a Graph Window (Workstation → double-click) and draw your gating strategy.",
                    "Workstation"));
            hintPane.getChildren().add(hintInfoRow(
                    "Tip: draw gates on one sample, then right-click in the Workstation → Apply to all samples."));
        }

        hintPane.setVisible(true);
        hintPane.setManaged(true);
    }

    private boolean anyGatesInWorkspace(WorkspaceModel ws) {
        for (String s : ws.sampleNames()) {
            if (!ws.hasTree(s)) continue;
            for (PopNode n : ws.treeFor(s).selfAndDescendants()) if (!n.isRoot()) return true;
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
