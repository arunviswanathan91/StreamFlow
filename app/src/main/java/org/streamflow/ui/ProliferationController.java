package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.Slider;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.layout.HBox;
import javafx.scene.paint.Color;
import javafx.stage.FileChooser;

import javax.imageio.ImageIO;
import java.io.File;
import java.util.ArrayList;
import java.util.List;
import java.util.regex.Pattern;

/**
 * Analysis module A2 — Proliferation Index. Fits a dye-dilution histogram
 * (CFSE / CTV / BrdU / Ki-67) with equal-σ, log2-spaced Gaussian generations.
 * The engine now returns curve data; this controller renders them in an interactive
 * {@link AnalysisChart} with per-generation toggles, X-zoom, and publication export.
 */
public class ProliferationController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern DYE = Pattern.compile("CFSE|CTV|CellTrace|Ki.?67|BrdU|VPD|Prolif",
            Pattern.CASE_INSENSITIVE);

    // Viridis-inspired palette for generations (bright=early, dim=late)
    private static final String[] GEN_COLORS = {
        "#440154", "#31688e", "#35b779", "#fde725",
        "#21918c", "#5ec962", "#90d743", "#b5de2b"
    };

    @FXML private ComboBox<String> sampleCombo;
    @FXML private ComboBox<String> channelCombo;
    @FXML private Spinner<Integer> peaksSpinner;
    @FXML private Button refreshButton, runButton, copyButton, exportPngButton, exportSvgButton;
    @FXML private CheckBox showHist, showTotal;
    @FXML private Slider xZoom;
    @FXML private Label statusLabel;
    @FXML private AnalysisChart chart;
    @FXML private HBox genToggleBox;
    @FXML private Label piLabel, diLabel, divLabel;
    @FXML private TableView<GenRow> genTable;
    @FXML private TableColumn<GenRow, Number> genCol;
    @FXML private TableColumn<GenRow, Number> countCol;
    @FXML private TableColumn<GenRow, Number> pctCol;

    private final ObservableList<GenRow> genRows = FXCollections.observableArrayList();
    private AppContext ctx;

    public record GenRow(int gen, int count, double pct) {}

    @FXML
    public void initialize() {
        peaksSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(2, 12, 8, 1));
        genCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().gen()));
        countCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().count()));
        pctCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().pct()));
        genTable.setItems(genRows);
        chart.setAxisLabels("log₁₀(dye intensity) — brighter → fewer divisions", "Count");
        showHist.setOnAction(e -> chart.setVisible("Histogram", showHist.isSelected()));
        showTotal.setOnAction(e -> chart.setVisible("Model fit", showTotal.isSelected()));
        xZoom.valueProperty().addListener((o, a, b) -> chart.setXMaxFraction(b.doubleValue()));
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshSamples();
    }

    @FXML private void onRefresh() { refreshSamples(); }

    private void refreshSamples() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().clear();
            r.path("samples").forEach(n -> sampleCombo.getItems().add(n.asText()));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();
            channelCombo.getItems().clear();
            String dyeGuess = null;
            for (JsonNode c : r.path("channels")) {
                String ch = c.asText();
                channelCombo.getItems().add(ch);
                if (dyeGuess == null && DYE.matcher(ch).find()) dyeGuess = ch;
            }
            if (dyeGuess != null) channelCombo.getSelectionModel().select(dyeGuess);
            else if (!channelCombo.getItems().isEmpty()) channelCombo.getSelectionModel().selectFirst();
        });
    }

    @FXML
    private void onRun() {
        if (ctx == null || sampleCombo.getValue() == null || channelCombo.getValue() == null) return;
        ObjectNode a = JSON.createObjectNode();
        a.put("sample", sampleCombo.getValue());
        a.put("channel", channelCombo.getValue());
        a.put("n_peaks", peaksSpinner.getValue());
        statusLabel.setText("Fitting generations on " + channelCombo.getValue() + "…");
        ctx.jobs().run(ctx.bridge().command("run_proliferation", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        piLabel.setText(String.format("Proliferation Index: %.3f", result.path("PI").asDouble()));
        diLabel.setText(String.format("Division Index: %.3f", result.path("DI").asDouble()));
        divLabel.setText(String.format("%% Divided: %.1f%%", result.path("pct_divided").asDouble()));

        genRows.clear();
        for (JsonNode g : result.path("generations")) {
            genRows.add(new GenRow(g.path("gen").asInt(), g.path("count").asInt(), g.path("pct").asDouble()));
        }

        JsonNode c = result.path("curves");
        double[] x = arr(c.path("x"));
        chart.clearSeries();
        chart.setX(x);
        chart.addSeries("Histogram", arr(c.path("hist")), Color.web("#D8DEE9"), AnalysisChart.Kind.BARS);

        // Per-generation Gaussian curves with viridis-ish palette
        List<CheckBox> genChecks = new ArrayList<>();
        genToggleBox.getChildren().clear();
        JsonNode gens = c.path("generations");
        for (int gi = 0; gi < gens.size(); gi++) {
            JsonNode gn = gens.get(gi);
            int gen = gn.path("gen").asInt();
            String col = GEN_COLORS[gen % GEN_COLORS.length];
            String seriesName = "Gen " + gen;
            chart.addSeries(seriesName, arr(gn.path("y")), Color.web(col), AnalysisChart.Kind.AREA);
            CheckBox cb = new CheckBox("Gen " + gen);
            cb.setSelected(true);
            cb.setOnAction(e -> chart.setVisible(seriesName, cb.isSelected()));
            genChecks.add(cb);
            genToggleBox.getChildren().add(cb);
        }

        AnalysisChart.Series modelSeries = chart.addSeries("Model fit", arr(c.path("total")),
                Color.web("#111111"), AnalysisChart.Kind.LINE);
        modelSeries.dashed = true;

        chart.setTitle("Proliferation · PI=" + String.format("%.2f", result.path("PI").asDouble())
                + "  DI=" + String.format("%.2f", result.path("DI").asDouble())
                + "  Divided=" + String.format("%.1f%%", result.path("pct_divided").asDouble())
                + " · " + sampleCombo.getValue());
        chart.setVisible("Histogram", showHist.isSelected());
        chart.setVisible("Model fit", showTotal.isSelected());
        chart.setXMaxFraction(xZoom.getValue());
        chart.refresh();

        statusLabel.setText(genRows.size() + " generation(s) fitted on "
                + result.path("channel").asText() + " — n=" + result.path("n").asLong() + ".");
        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                String.format("Proliferation on %s: PI=%.3f DI=%.3f div=%.1f%%",
                        result.path("channel").asText(),
                        result.path("PI").asDouble(),
                        result.path("DI").asDouble(),
                        result.path("pct_divided").asDouble()));
    }

    @FXML
    private void onCopy() {
        if (chart.getWidth() <= 0) { statusLabel.setText("Run a fit first."); return; }
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = chart.snapshotAtDpi(dpi);
        if (img == null) { statusLabel.setText("Nothing to copy — run a fit first."); return; }
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied " + dpi + " DPI proliferation plot — paste into PowerPoint.");
    }

    @FXML
    private void onExportPng() {
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = chart.snapshotAtDpi(dpi);
        if (img == null) { statusLabel.setText("Nothing to export — run a fit first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export proliferation plot (PNG)");
        fc.setInitialFileName("proliferation.png");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"));
        File f = fc.showSaveDialog(chart.getScene().getWindow());
        if (f == null) return;
        try {
            ImageIO.write(javafx.embed.swing.SwingFXUtils.fromFXImage(img, null), "png", f);
            statusLabel.setText("Exported PNG at " + dpi + " DPI → " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("PNG export failed: " + e.getMessage());
        }
    }

    @FXML
    private void onExportSvg() {
        FileChooser fc = new FileChooser();
        fc.setTitle("Export proliferation plot (SVG)");
        fc.setInitialFileName("proliferation.svg");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("SVG vector (*.svg)", "*.svg"));
        File f = fc.showSaveDialog(chart.getScene().getWindow());
        if (f == null) return;
        try {
            java.nio.file.Files.writeString(f.toPath(), chart.toSvg());
            statusLabel.setText("Exported SVG → " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("SVG export failed: " + e.getMessage());
        }
    }

    private static double[] arr(JsonNode n) {
        if (n == null || !n.isArray()) return new double[0];
        double[] a = new double[n.size()];
        for (int i = 0; i < n.size(); i++) a[i] = n.get(i).asDouble();
        return a;
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d); channelCombo.setDisable(d);
        peaksSpinner.setDisable(d); refreshButton.setDisable(d); runButton.setDisable(d);
        copyButton.setDisable(d); exportPngButton.setDisable(d); exportSvgButton.setDisable(d);
    }
}
