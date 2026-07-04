package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.paint.Color;

import java.util.ArrayList;
import java.util.List;
import java.util.regex.Pattern;

/**
 * Analysis module A3 — Apoptosis. Runs {@code run_apoptosis} on the engine:
 * auto-detects Annexin V and PI thresholds at density valleys, then displays
 * the scatter in a live {@link CytoPlot} with a draggable quadrant crosshair —
 * the four population percentages update instantly as you drag.
 */
public class ApoptosisController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern ANNEXIN =
            Pattern.compile("Annexin|AnnV|Ann.?V|FITC.*Ann|Ann.*FITC", Pattern.CASE_INSENSITIVE);
    private static final Pattern PI_CHAN =
            Pattern.compile("PI|Propidium|7.?AAD|7AAD|PerCP|EtBr", Pattern.CASE_INSENSITIVE);

    @FXML private ComboBox<String> sampleCombo;
    @FXML private ComboBox<String> gateCombo;
    @FXML private Label            gateWarningLabel;
    @FXML private ComboBox<String> annexinCombo;
    @FXML private ComboBox<String> piCombo;
    @FXML private Button refreshButton, runButton, copyButton, exportPngButton;
    @FXML private Label statusLabel;
    @FXML private CytoPlot plot;
    @FXML private TableView<QuadRow> quadTable;
    @FXML private TableColumn<QuadRow, String>  quadNameCol;
    @FXML private TableColumn<QuadRow, Number>  quadPctCol;
    @FXML private TableColumn<QuadRow, Number>  quadNCol;
    @FXML private Label annexinThreshLabel;
    @FXML private Label piThreshLabel;

    private final ObservableList<QuadRow> quadRows = FXCollections.observableArrayList();
    private AppContext ctx;

    // The four apoptosis quadrant gates (shared center xs[0], ys[0])
    private CytoPlot.Gate q1, q2, q3, q4;
    private EventData currentData;
    private int totalEvents = 0;

    public record QuadRow(String name, double pct, int count) {}

    @FXML
    public void initialize() {
        quadNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name()));
        quadPctCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(
                Math.round(c.getValue().pct() * 10.0) / 10.0));
        quadNCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().count()));
        quadTable.setItems(quadRows);

        setAliasDisplay(annexinCombo);
        setAliasDisplay(piCombo);
        sampleCombo.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> refreshGates(b));
        setDisabled(true);

        // Wire the CytoPlot: when user drags the quadrant crosshair, recompute counts live
        plot.setOnGateChanged(g -> recomputeQuadrants());
    }

    private void setAliasDisplay(ComboBox<String> combo) {
        combo.setConverter(new javafx.util.StringConverter<>() {
            @Override public String toString(String ch) {
                return ch == null ? "" : (ctx != null ? ctx.aliases().label(ch) : ch);
            }
            @Override public String fromString(String s) { return s; }
        });
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        plot.setChannelLabeler(ch -> ctx.aliases().label(ch));
        setDisabled(false);
        ctx.workspace().addTreeChangeListener(() -> refreshGates(sampleCombo.getValue()));
        refreshChannels();
    }

    @FXML private void onRefresh() { refreshChannels(); }
    @Override public void refreshFromWorkspace() { refreshChannels(); }

    private void refreshChannels() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().clear();
            r.path("samples").forEach(n -> sampleCombo.getItems().add(n.asText()));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();

            List<String> channels = new ArrayList<>();
            r.path("channels").forEach(c -> channels.add(c.asText()));
            annexinCombo.getItems().setAll(channels);
            piCombo.getItems().setAll(channels);

            String annGuess = null, piGuess = null;
            for (String ch : channels) {
                if (annGuess == null && ANNEXIN.matcher(ch).find()) annGuess = ch;
                if (piGuess  == null && PI_CHAN.matcher(ch).find())  piGuess  = ch;
            }
            if (annGuess != null) annexinCombo.getSelectionModel().select(annGuess);
            else if (!channels.isEmpty()) annexinCombo.getSelectionModel().selectFirst();
            if (piGuess != null) piCombo.getSelectionModel().select(piGuess);
            else if (channels.size() > 1) piCombo.getSelectionModel().select(1);
            else if (!channels.isEmpty()) piCombo.getSelectionModel().selectFirst();
            refreshGates(sampleCombo.getValue());
        });
    }

    // ---- Gate combo ---------------------------------------------------------

    private void refreshGates(String sample) {
        if (ctx == null || sample == null) return;
        gateCombo.getItems().clear();
        gateCombo.getItems().add("Ungated (All Events)");
        PopNode root = ctx.workspace().treeFor(sample);
        for (PopNode n : root.selfAndDescendants())
            if (!n.isRoot()) gateCombo.getItems().add(n.name());
        gateCombo.getSelectionModel().selectFirst();
        updateGateWarning();
    }

    private void updateGateWarning() {
        boolean noGates = gateCombo.getItems().size() <= 1;
        gateWarningLabel.setVisible(noGates);
        gateWarningLabel.setManaged(noGates);
    }

    private PopNode selectedGateNode() {
        String sel = gateCombo.getValue();
        if (sel == null || sel.startsWith("Ungated")) return null;
        String sample = sampleCombo.getValue();
        if (sample == null) return null;
        PopNode root = ctx.workspace().treeFor(sample);
        for (PopNode n : root.selfAndDescendants())
            if (!n.isRoot() && n.name().equals(sel)) return n;
        return null;
    }

    @FXML
    private void onRun() {
        if (ctx == null || sampleCombo.getValue() == null
                || annexinCombo.getValue() == null || piCombo.getValue() == null) return;
        if (annexinCombo.getValue().equals(piCombo.getValue())) {
            statusLabel.setText("Annexin V and PI channel must be different.");
            return;
        }
        ObjectNode a = JSON.createObjectNode();
        a.put("sample",           sampleCombo.getValue());
        a.put("annexin_channel",  annexinCombo.getValue());
        a.put("pi_channel",       piCombo.getValue());
        PopNode target = selectedGateNode();
        if (target != null) {
            ArrayNode polygons = JSON.createArrayNode();
            for (CytoPlot.Gate g : target.chain()) {
                ObjectNode gn = JSON.createObjectNode();
                gn.put("type", g.type); gn.put("x_channel", g.xChan);
                gn.put("y_channel", g.yChan != null ? g.yChan : "");
                ArrayNode xs = JSON.createArrayNode(); for (double v : g.xs) xs.add(v);
                ArrayNode ys = JSON.createArrayNode(); for (double v : g.ys) ys.add(v);
                gn.set("xs", xs); gn.set("ys", ys);
                polygons.add(gn);
            }
            a.set("gate_polygons", polygons);
        }
        String pop = target != null ? target.name() : "All Events";
        statusLabel.setText("Detecting thresholds on " + pop + "…");
        ctx.jobs().run(ctx.bridge().command("run_apoptosis", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        JsonNode thr = result.path("thresholds");
        double annThr = thr.path("annexin").asDouble();
        double piThr  = thr.path("pi").asDouble();
        String annCh  = result.path("annexin_channel").asText();
        String piCh   = result.path("pi_channel").asText();
        totalEvents   = result.path("n").asInt();

        // Build EventData from the subsampled events returned by the engine
        JsonNode evs = result.path("events");
        double[] annArr = arr(evs.path("ann"));
        double[] piArr  = arr(evs.path("pi"));
        int nPts = Math.min(annArr.length, piArr.length);
        float[] floats = new float[nPts * 2];
        for (int i = 0; i < nPts; i++) {
            floats[i * 2]     = (float) annArr[i];
            floats[i * 2 + 1] = (float) piArr[i];
        }
        currentData = new EventData(floats, nPts, 2, List.of(annCh, piCh));

        // Quadrant gates: q1=top-right (late), q2=top-left (necrotic),
        //                 q3=bottom-left (live), q4=bottom-right (early)
        q1 = new CytoPlot.Gate("Late Apoptotic",  "q1", annCh, piCh, new double[]{annThr}, new double[]{piThr});
        q2 = new CytoPlot.Gate("Necrotic",        "q2", annCh, piCh, new double[]{annThr}, new double[]{piThr});
        q3 = new CytoPlot.Gate("Live",            "q3", annCh, piCh, new double[]{annThr}, new double[]{piThr});
        q4 = new CytoPlot.Gate("Early Apoptotic", "q4", annCh, piCh, new double[]{annThr}, new double[]{piThr});
        q1.border = Color.web("#E05555");
        q2.border = Color.web("#9B59B6");
        q3.border = Color.web("#4CAF50");
        q4.border = Color.web("#F5A623");

        plot.clearGates();
        plot.setData(currentData);
        plot.setView(annCh, piCh, CytoPlot.Scale.LINEAR, CytoPlot.Scale.LINEAR, "pseudocolor");
        plot.addGate(q1); plot.addGate(q2); plot.addGate(q3); plot.addGate(q4);

        annexinThreshLabel.setText(String.format("Annexin V threshold: %.2f", annThr));
        piThreshLabel.setText(String.format("PI threshold: %.2f", piThr));

        recomputeQuadrants();

        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                String.format("Apoptosis on %s/%s: Live=%.1f%% Early=%.1f%% Late=%.1f%% Necrotic=%.1f%%",
                        annCh, piCh,
                        result.path("quadrants").path("live").asDouble(),
                        result.path("quadrants").path("early_apoptotic").asDouble(),
                        result.path("quadrants").path("late_apoptotic").asDouble(),
                        result.path("quadrants").path("necrotic").asDouble()));
    }

    /** Recompute quadrant membership from the current gate center and update the table. */
    private void recomputeQuadrants() {
        if (currentData == null || q1 == null) return;
        // All four share the same center (moved together by MOVE_QUADRANT drag)
        double cx = q1.xs[0], cy = q1.ys[0];
        annexinThreshLabel.setText(String.format("Annexin V threshold: %.2f", cx));
        piThreshLabel.setText(String.format("PI threshold: %.2f", cy));

        // Count via CytoPlot.mask over the subsampled data
        int nLate = count(q1), nNec = count(q2), nLive = count(q3), nEarly = count(q4);
        int total = Math.max(1, nLive + nEarly + nLate + nNec);
        double scale = totalEvents > 0 ? (double) totalEvents / total : 1.0;

        quadRows.setAll(
            new QuadRow("Live",             100.0 * nLive  / total, (int) (nLive  * scale)),
            new QuadRow("Early Apoptotic",  100.0 * nEarly / total, (int) (nEarly * scale)),
            new QuadRow("Late Apoptotic",   100.0 * nLate  / total, (int) (nLate  * scale)),
            new QuadRow("Necrotic",         100.0 * nNec   / total, (int) (nNec   * scale))
        );

        statusLabel.setText(String.format(
                "Live %.1f%%  Early Apoptotic %.1f%%  Late Apoptotic %.1f%%  Necrotic %.1f%%",
                100.0 * nLive / total, 100.0 * nEarly / total,
                100.0 * nLate / total, 100.0 * nNec / total));
    }

    private int count(CytoPlot.Gate g) {
        if (currentData == null) return 0;
        boolean[] m = CytoPlot.mask(currentData, g);
        int n = 0; for (boolean b : m) if (b) n++; return n;
    }

    @FXML
    private void onCopy() {
        if (currentData == null) { statusLabel.setText("Run analysis first."); return; }
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = plot.exportImage(dpi / 96.0);
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied " + dpi + " DPI apoptosis plot — paste into PowerPoint.");
    }

    @FXML
    private void onExportPng() {
        if (currentData == null) { statusLabel.setText("Run analysis first."); return; }
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = plot.exportImage(dpi / 96.0);
        javafx.stage.FileChooser fc = new javafx.stage.FileChooser();
        fc.setTitle("Export apoptosis plot (PNG)");
        fc.setInitialFileName("apoptosis.png");
        fc.getExtensionFilters().add(new javafx.stage.FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"));
        java.io.File f = fc.showSaveDialog(plot.getScene().getWindow());
        if (f == null) return;
        try {
            javax.imageio.ImageIO.write(javafx.embed.swing.SwingFXUtils.fromFXImage(img, null), "png", f);
            statusLabel.setText("Exported PNG at " + dpi + " DPI → " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("PNG export failed: " + e.getMessage());
        }
    }

    private static double[] arr(JsonNode n) {
        if (n == null || !n.isArray()) return new double[0];
        double[] a = new double[n.size()];
        for (int i = 0; i < n.size(); i++) a[i] = n.get(i).asDouble();
        return a;
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d); gateCombo.setDisable(d);
        annexinCombo.setDisable(d); piCombo.setDisable(d);
        refreshButton.setDisable(d); runButton.setDisable(d);
        copyButton.setDisable(d); exportPngButton.setDisable(d);
    }
}
