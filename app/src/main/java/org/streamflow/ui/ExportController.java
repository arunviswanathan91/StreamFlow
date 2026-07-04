package org.streamflow.ui;

import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.CheckBoxTableCell;
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

    // ---- Gating Strategy figure tab ----
    @FXML private javafx.scene.control.ComboBox<String> gsSampleCombo;
    @FXML private Button gsBuildButton, gsCopyButton, gsSavePngButton;
    @FXML private javafx.scene.layout.HBox gsStrip;
    @FXML private Label gsStatusLabel;

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

        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshFromWorkspace();
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
    }

    // ---- Gating Strategy figure ----
    @FXML
    private void onGsBuild() {
        if (ctx == null) return;
        String sample = gsSampleCombo.getValue();
        if (sample == null) { info("No sample", "Pick a sample to build its gating-strategy figure."); return; }
        if (!ctx.workspace().hasTree(sample) || ctx.workspace().treeFor(sample).children.isEmpty()) {
            info("No gates", "This sample has no gates. Draw a gating strategy first.");
            return;
        }
        gsStatusLabel.setText("Loading events…");
        EventLoader.ensureLoaded(ctx, List.of(sample), gsStatusLabel::setText, () -> buildStrip(sample));
    }

    private void buildStrip(String sample) {
        WorkspaceModel ws = ctx.workspace();
        EventData root = ws.data(sample);
        if (root == null || root.rows() == 0) { gsStatusLabel.setText("No events for " + sample + "."); return; }
        PopNode rootNode = ws.treeFor(sample);
        List<PopNode> steps = new ArrayList<>();
        collectSteps(rootNode, steps);                 // populations that carry gates, in tree order
        gsStrip.getChildren().clear();
        if (steps.isEmpty()) { info("No gates", "This sample has no gates to show."); return; }
        boolean first = true;
        for (PopNode p : steps) {
            if (!first) gsStrip.getChildren().add(arrowNode());
            first = false;
            gsStrip.getChildren().add(stepCell(root, p));
        }
        gsStatusLabel.setText(steps.size() + " gating step(s) for " + shortName(sample) + ". Copy (PPT) or Save PNG.");
    }

    private void collectSteps(PopNode n, List<PopNode> out) {
        if (!n.children.isEmpty()) out.add(n);
        for (PopNode c : n.children) collectSteps(c, out);
    }

    /** One figure panel: a population's events (light/publication style) with its child gates drawn. */
    private javafx.scene.Node stepCell(EventData root, PopNode p) {
        EventData ev = p.isRoot() ? root : subsetFor(root, p);
        String ax = p.viewX, ay = p.viewY, axs = p.viewXScale, ays = p.viewYScale;
        if (ax == null && !p.children.isEmpty()) {     // fall back to the first child's gate axes
            CytoPlot.Gate g0 = p.children.get(0).gate;
            if (g0 != null) { ax = g0.xChan; ay = g0.yChan; }
        }
        boolean hist = ay == null;
        CytoPlot plot = new CytoPlot();
        plot.setMinSize(220, 220); plot.setPrefSize(220, 220); plot.setMaxSize(220, 220);
        plot.setChannelLabeler(c -> ctx.aliases().label(c));
        plot.setLightMode(true);
        plot.setData(ev);
        plot.setView(ax, hist ? null : ay, scaleOf(axs), scaleOf(ays), hist ? "histogram" : "pseudocolor");
        for (PopNode c : p.children) if (c.gate != null) plot.addGate(c.gate);
        Label cap = new Label((p.isRoot() ? "All Events" : p.name()) + "  (" + ev.rows() + ")");
        cap.setStyle("-fx-text-fill:#1A2330; -fx-font-weight:bold; -fx-font-size:11;");
        javafx.scene.layout.VBox cell = new javafx.scene.layout.VBox(3, cap, plot);
        cell.setStyle("-fx-background-color:white;");
        return cell;
    }

    private javafx.scene.Node arrowNode() {
        org.kordamp.ikonli.javafx.FontIcon a = new org.kordamp.ikonli.javafx.FontIcon("fas-arrow-right");
        a.setIconSize(22); a.setIconColor(javafx.scene.paint.Color.web("#444444"));
        javafx.scene.layout.VBox box = new javafx.scene.layout.VBox(a);
        box.setAlignment(javafx.geometry.Pos.CENTER);
        box.setMinWidth(30);
        box.setStyle("-fx-background-color:white;");
        return box;
    }

    private static CytoPlot.Scale scaleOf(String s) {
        if ("Log".equals(s)) return CytoPlot.Scale.LOG;
        if ("Logicle".equals(s)) return CytoPlot.Scale.LOGICLE;
        if ("ArcSinh".equals(s)) return CytoPlot.Scale.ARCSINH;
        return CytoPlot.Scale.LINEAR;
    }

    @FXML
    private void onGsCopy() {
        javafx.scene.image.WritableImage img = snapStrip();
        if (img == null) { gsStatusLabel.setText("Build the figure first."); return; }
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        gsStatusLabel.setText("Copied gating-strategy figure at " + dpi + " DPI — paste into PowerPoint.");
    }

    @FXML
    private void onGsSavePng() {
        javafx.scene.image.WritableImage img = snapStrip();
        if (img == null) { gsStatusLabel.setText("Build the figure first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Save gating-strategy figure (PNG)");
        fc.setInitialFileName("gating_strategy.png");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"));
        File f = fc.showSaveDialog(gsStrip.getScene().getWindow());
        if (f == null) return;
        try {
            javax.imageio.ImageIO.write(javafx.embed.swing.SwingFXUtils.fromFXImage(img, null), "png", f);
            gsStatusLabel.setText("Saved " + f.getName() + ".");
        } catch (Exception e) {
            gsStatusLabel.setText("Save failed: " + e.getMessage());
        }
    }

    /** Snapshot the whole figure strip at the export DPI, white background (publication-ready). */
    private javafx.scene.image.WritableImage snapStrip() {
        if (gsStrip == null || gsStrip.getChildren().isEmpty()) return null;
        double scale = Math.max(1.0, (ctx != null ? ctx.settings().exportDpi() : 300) / 96.0);
        javafx.scene.SnapshotParameters sp = new javafx.scene.SnapshotParameters();
        sp.setFill(javafx.scene.paint.Color.WHITE);
        sp.setTransform(javafx.scene.transform.Transform.scale(scale, scale));
        return gsStrip.snapshot(sp, null);
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
