package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.image.WritableImage;
import javafx.scene.input.Clipboard;
import javafx.scene.input.ClipboardContent;
import javafx.scene.layout.StackPane;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;
import javafx.scene.text.FontWeight;
import javafx.stage.FileChooser;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.File;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.TreeSet;

public class StatComparisonController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Color[] PALETTE = {
        Color.web("#66C2A5"), Color.web("#FC8D62"), Color.web("#8DA0CB"),
        Color.web("#E78AC3"), Color.web("#A6D854"), Color.web("#FFD92F"),
        Color.web("#E5C494"), Color.web("#B3B3B3")
    };

    @FXML private ComboBox<String> gateCombo;
    @FXML private CheckBox pairedCheck;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Button copyButton;
    @FXML private Button exportButton;
    @FXML private Label statusLabel;
    @FXML private Label resultLabel;
    @FXML private TableView<Row> sampleTable;
    @FXML private TableColumn<Row, String> sampleCol;
    @FXML private TableColumn<Row, String> freqCol;
    @FXML private TableColumn<Row, String> groupCol;
    @FXML private TableView<PostHoc> posthocTable;
    @FXML private TableColumn<PostHoc, String> pairCol;
    @FXML private TableColumn<PostHoc, String> pCol;
    @FXML private TableColumn<PostHoc, String> starCol;
    @FXML private StackPane boxPlotPane;
    @FXML private Canvas boxCanvas;

    private final ObservableList<Row> rows = FXCollections.observableArrayList();
    private final ObservableList<PostHoc> posthocRows = FXCollections.observableArrayList();
    private AppContext ctx;

    // current plot data, stored for resize-redraw
    private List<String>   plotNames;
    private List<double[]> plotValues;
    private double         plotP;
    private String         plotStars;
    private List<PostHoc>  plotPosthoc;

    public static final class Row {
        final String sample;
        final Double freq;
        final StringProperty group = new SimpleStringProperty("");
        Row(String sample, Double freq) { this.sample = sample; this.freq = freq; }
    }

    public record PostHoc(String pair, String p, String stars) {}

    @FXML
    public void initialize() {
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sample));
        freqCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(
                c.getValue().freq == null ? "—" : String.format("%.2f", c.getValue().freq)));
        groupCol.setCellValueFactory(c -> c.getValue().group);
        groupCol.setCellFactory(TextFieldTableCell.forTableColumn());
        groupCol.setOnEditCommit(e -> e.getRowValue().group.set(e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(rows);
        sampleTable.setEditable(true);

        pairCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pair()));
        pCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().p()));
        starCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().stars()));
        posthocTable.setItems(posthocRows);

        copyButton.setDisable(true); exportButton.setDisable(true);

        // resize-redraw
        boxPlotPane.widthProperty().addListener((o, a, b) -> redrawIfReady());
        boxPlotPane.heightProperty().addListener((o, a, b) -> redrawIfReady());

        gateCombo.getSelectionModel().selectedItemProperty().addListener((o, prev, sel) -> {
            if (sel != null) rebuildRows();
        });
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refresh();
    }

    @FXML private void onRefresh() { refresh(); }
    @Override public void refreshFromWorkspace() { refresh(); }

    private void refresh() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();
        TreeSet<String> gateNames = new TreeSet<>();
        for (String s : ws.samples()) {
            PopNode root = ws.treeFor(s);
            for (PopNode n : root.selfAndDescendants())
                if (!n.isRoot()) gateNames.add(n.name());
        }
        String prevGate = gateCombo.getValue();
        gateCombo.setItems(FXCollections.observableArrayList(gateNames));
        if (prevGate != null && gateNames.contains(prevGate)) gateCombo.setValue(prevGate);
        else if (!gateNames.isEmpty()) gateCombo.getSelectionModel().selectFirst();
        rebuildRows();
        if (gateNames.isEmpty())
            statusLabel.setText("No gates found. Draw at least one gate first.");
    }

    private void rebuildRows() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();
        String gate = gateCombo.getValue();
        Map<String, String> prevGroups = new LinkedHashMap<>();
        for (Row r : rows) prevGroups.put(r.sample, r.group.get());
        rows.clear();
        int computed = 0;
        for (String s : ws.sampleNames()) {
            Double freq = (gate == null) ? null : freqForSample(s, gate);
            Row row = new Row(s, freq);
            if (prevGroups.containsKey(s)) row.group.set(prevGroups.get(s));
            rows.add(row);
            if (freq != null) computed++;
        }
        statusLabel.setText(gate == null ? "Select a gate."
                : String.format("Gate '%s': frequency computed for %d of %d sample(s).",
                        gate, computed, rows.size()));
    }

    private Double freqForSample(String sample, String gateName) {
        WorkspaceModel ws = ctx.workspace();
        if (!ws.hasTree(sample)) return null;
        EventData root = ws.data(sample);
        if (root == null || root.rows() == 0) return null;
        PopNode target = findByName(ws.treeFor(sample), gateName);
        if (target == null) return null;
        boolean[] keep = new boolean[root.rows()];
        Arrays.fill(keep, true);
        for (CytoPlot.Gate g : target.chain()) {
            boolean[] m = CytoPlot.mask(root, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        int c = 0; for (boolean b : keep) if (b) c++;
        return 100.0 * c / root.rows();
    }

    private PopNode findByName(PopNode root, String name) {
        for (PopNode n : root.selfAndDescendants())
            if (!n.isRoot() && n.name().equals(name)) return n;
        return null;
    }

    @FXML
    private void onRun() {
        if (ctx == null || gateCombo.getValue() == null) return;
        Map<String, List<Double>> groups = new LinkedHashMap<>();
        for (Row r : rows) {
            String g = r.group.get();
            if (g == null || g.isBlank() || r.freq == null) continue;
            groups.computeIfAbsent(g.trim(), k -> new ArrayList<>()).add(r.freq);
        }
        long usable = groups.values().stream().filter(l -> l.size() >= 2).count();
        if (usable < 2) {
            statusLabel.setText("Assign at least 2 groups with ≥2 computed samples each.");
            return;
        }
        ObjectNode a = JSON.createObjectNode();
        a.put("gate", gateCombo.getValue());
        a.put("paired", pairedCheck.isSelected());
        ObjectNode gNode = a.putObject("groups");
        groups.forEach((name, vals) -> { ArrayNode arr = gNode.putArray(name); vals.forEach(arr::add); });
        statusLabel.setText("Running statistical test…");
        ctx.jobs().run(ctx.bridge().command("run_stats_comparison", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        plotP     = result.path("p_value").asDouble();
        plotStars = result.path("stars").asText();

        resultLabel.setText(String.format("%s — p = %.4g  %s",
                result.path("test").asText(), plotP, plotStars));

        posthocRows.clear();
        plotPosthoc = new ArrayList<>();
        for (JsonNode ph : result.path("posthoc")) {
            PostHoc row = new PostHoc(ph.path("pair").asText(),
                    String.format("%.4g", ph.path("p").asDouble()),
                    ph.path("stars").asText());
            posthocRows.add(row);
            plotPosthoc.add(row);
        }

        plotNames = new ArrayList<>(); plotValues = new ArrayList<>();
        for (JsonNode grp : result.path("groups")) {
            plotNames.add(grp.path("name").asText());
            JsonNode vArr = grp.path("values");
            double[] vals = new double[vArr.size()];
            for (int i = 0; i < vArr.size(); i++) vals[i] = vArr.get(i).asDouble();
            plotValues.add(vals);
        }
        redrawIfReady();
        statusLabel.setText("Test complete: " + result.path("test").asText() + ".");
        ctx.auditLog().add(AuditLog.Type.ANALYSIS, gateCombo.getValue(),
                String.format("Stats Comparison: %s p=%.4g %s",
                        result.path("test").asText(), plotP, plotStars));
        copyButton.setDisable(false); exportButton.setDisable(false);
    }

    private void redrawIfReady() {
        if (plotNames == null || plotNames.isEmpty()) return;
        double W = boxPlotPane.getWidth(), H = boxPlotPane.getHeight();
        if (W < 10 || H < 10) return;
        boxCanvas.setWidth(W); boxCanvas.setHeight(H);
        drawBoxPlot(boxCanvas.getGraphicsContext2D(), W, H);
    }

    private void drawBoxPlot(GraphicsContext g, double W, double H) {
        g.clearRect(0, 0, W, H);
        g.setFill(Color.WHITE); g.fillRect(0, 0, W, H);

        double ML = 58, MR = 14, MT = 36, MB = 48;
        double pw = W - ML - MR, ph = H - MT - MB;
        if (pw < 10 || ph < 10) return;
        int n = plotNames.size();

        // Y range
        double yMin = Double.MAX_VALUE, yMax = -Double.MAX_VALUE;
        for (double[] v : plotValues)
            for (double x : v) { yMin = Math.min(yMin, x); yMax = Math.max(yMax, x); }
        double yRange = Math.max(1e-9, yMax - yMin);
        yMin -= yRange * 0.05; yMax += yRange * 0.20; yRange = yMax - yMin;

        // frame + grid
        g.setStroke(Color.web("#CCCCCC")); g.setLineWidth(0.5);
        for (int k = 0; k <= 4; k++) {
            double yp = MT + ph - k / 4.0 * ph;
            g.strokeLine(ML, yp, ML + pw, yp);
        }
        g.setStroke(Color.web("#666666")); g.setLineWidth(1);
        g.strokeRect(ML, MT, pw, ph);

        double bw = Math.min(50, pw / (n + 1) * 0.55);
        for (int i = 0; i < n; i++) {
            Color col = PALETTE[i % PALETTE.length];
            double cx = ML + pw * (i + 1.0) / (n + 1);
            double[] sorted = Arrays.copyOf(plotValues.get(i), plotValues.get(i).length);
            Arrays.sort(sorted);
            double q1 = pct(sorted, 25), med = pct(sorted, 50), q3 = pct(sorted, 75);
            double iqr = q3 - q1;
            double wLo = sorted[0]; // use actual min/max for whiskers (all data shown as dots)
            double wHi = sorted[sorted.length - 1];

            double pyQ1 = py(q1, MT, ph, yMin, yRange);
            double pyQ3 = py(q3, MT, ph, yMin, yRange);
            double pyMed = py(med, MT, ph, yMin, yRange);
            double pyWLo = py(wLo, MT, ph, yMin, yRange);
            double pyWHi = py(wHi, MT, ph, yMin, yRange);

            // IQR box
            g.setFill(translucent(col, 0.35)); g.setStroke(col); g.setLineWidth(1.5);
            g.fillRect(cx - bw / 2, pyQ3, bw, pyQ1 - pyQ3);
            g.strokeRect(cx - bw / 2, pyQ3, bw, pyQ1 - pyQ3);
            // median
            g.setStroke(Color.web("#222222")); g.setLineWidth(2.2);
            g.strokeLine(cx - bw / 2, pyMed, cx + bw / 2, pyMed);
            // whiskers
            g.setStroke(col); g.setLineWidth(1.2);
            g.strokeLine(cx, pyQ1, cx, pyWLo);
            g.strokeLine(cx, pyQ3, cx, pyWHi);
            g.strokeLine(cx - bw * 0.2, pyWLo, cx + bw * 0.2, pyWLo);
            g.strokeLine(cx - bw * 0.2, pyWHi, cx + bw * 0.2, pyWHi);
            // jitter dots
            Random rng = new Random(i);
            for (double val : plotValues.get(i)) {
                boolean out = val < q1 - 1.5 * iqr || val > q3 + 1.5 * iqr;
                double jx = (rng.nextDouble() - 0.5) * bw * 0.45;
                g.setFill(out ? Color.web("#E74C3C", 0.8) : new Color(col.getRed(), col.getGreen(), col.getBlue(), 0.7));
                double dp = py(val, MT, ph, yMin, yRange);
                g.fillOval(cx + jx - 3.5, dp - 3.5, 7, 7);
            }
            // label
            g.setFill(Color.web("#333333")); g.setFont(Font.font("Segoe UI", 10));
            String lbl = plotNames.get(i);
            g.fillText(lbl, cx - lbl.length() * 2.8, MT + ph + 18);
        }

        // Y axis ticks
        g.setFont(Font.font("Segoe UI", 9)); g.setFill(Color.web("#555555"));
        for (int k = 0; k <= 4; k++) {
            double yv = yMin + k / 4.0 * yRange;
            double yp = py(yv, MT, ph, yMin, yRange);
            g.fillText(fmt(yv), 2, yp + 4);
        }

        // Significance brackets
        List<PostHoc> sig = new ArrayList<>();
        if (plotPosthoc != null) for (PostHoc ph2 : plotPosthoc)
            if (!"ns".equals(ph2.stars())) sig.add(ph2);
        if (n == 2 && sig.isEmpty()) {
            // draw single bracket
            double yTop = MT + ph * 0.05;
            drawBracket(g, ML + pw * (1.0) / (n + 1), ML + pw * (2.0) / (n + 1),
                    yTop, plotStars);
        } else {
            double span = ph * 0.12;
            for (int si = 0; si < sig.size() && si < 5; si++) {
                PostHoc ph2 = sig.get(si);
                String[] parts = ph2.pair().split(" vs ");
                if (parts.length != 2) continue;
                int ia = plotNames.indexOf(parts[0]), ib = plotNames.indexOf(parts[1]);
                if (ia < 0 || ib < 0) continue;
                double xa = ML + pw * (ia + 1.0) / (n + 1), xb = ML + pw * (ib + 1.0) / (n + 1);
                double yTop = MT + ph * 0.03 + si * span * 0.85;
                drawBracket(g, xa, xb, yTop, ph2.stars());
            }
        }

        // title
        g.setFill(Color.web("#1A1A1A"));
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 12));
        g.fillText("p = " + String.format("%.4g", plotP) + "  " + plotStars, ML, 22);
    }

    private void drawBracket(GraphicsContext g, double x1, double x2, double yTop, String stars) {
        double h = 8;
        g.setStroke(Color.web("#333333")); g.setLineWidth(1.1);
        g.strokeLine(x1, yTop + h, x1, yTop);
        g.strokeLine(x1, yTop, x2, yTop);
        g.strokeLine(x2, yTop, x2, yTop + h);
        g.setFill(Color.web("#333333"));
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 13));
        g.fillText(stars, (x1 + x2) / 2 - stars.length() * 4, yTop - 3);
    }

    private static double py(double v, double top, double ph, double yMin, double yRange) {
        return top + ph - (v - yMin) / yRange * ph;
    }

    private static double pct(double[] sorted, double p) {
        double idx = (sorted.length - 1) * p / 100.0;
        int lo = (int) idx;
        double frac = idx - lo;
        if (lo + 1 >= sorted.length) return sorted[lo];
        return sorted[lo] * (1 - frac) + sorted[lo + 1] * frac;
    }

    private static Color translucent(Color c, double a) {
        return new Color(c.getRed(), c.getGreen(), c.getBlue(), a);
    }

    private static String fmt(double v) {
        double a = Math.abs(v);
        if (a >= 1000) return String.format("%.0f", v);
        if (a >= 1) return String.format("%.1f", v);
        return String.format("%.2f", v);
    }

    @FXML private void onCopy() {
        double W = boxCanvas.getWidth(), H = boxCanvas.getHeight();
        if (W < 1 || H < 1) return;
        WritableImage img = boxCanvas.snapshot(null, null);
        ClipboardContent cc = new ClipboardContent(); cc.putImage(img);
        Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied to clipboard.");
    }

    @FXML private void onExport() {
        FileChooser fc = new FileChooser();
        fc.setTitle("Export Boxplot");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG Image", "*.png"));
        File f = fc.showSaveDialog(boxCanvas.getScene().getWindow());
        if (f == null) return;
        try {
            WritableImage img = boxCanvas.snapshot(null, null);
            BufferedImage bi = javafx.embed.swing.SwingFXUtils.fromFXImage(img, null);
            ImageIO.write(bi, "png", f);
            statusLabel.setText("Saved: " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("Export failed: " + e.getMessage());
        }
    }

    private void setDisabled(boolean d) {
        gateCombo.setDisable(d);
        pairedCheck.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
