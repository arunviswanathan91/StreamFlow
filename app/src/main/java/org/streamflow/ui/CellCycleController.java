package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.Slider;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
import javafx.scene.control.TextArea;
import javafx.scene.paint.Color;
import javafx.stage.FileChooser;

import javax.imageio.ImageIO;
import java.io.File;
import java.nio.file.Files;
import java.util.regex.Pattern;

/**
 * Analysis module A1 — Cell Cycle. The heavy Watson / Dean-Jett-Fox fit runs in the engine
 * ({@code run_cell_cycle}), which now returns the histogram + fitted component curves; this
 * controller renders them in a native, interactive {@link AnalysisChart} (white background,
 * toggle components, X-zoom) and exports a publication PNG (at the Settings DPI) or SVG —
 * instead of showing a static image.
 */
public class CellCycleController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern DNA = Pattern.compile("PI|DAPI|7.?AAD|Hoechst|DNA|FxCycle|DRAQ",
            Pattern.CASE_INSENSITIVE);

    @FXML private ComboBox<String> sampleCombo, channelCombo, modelCombo;
    @FXML private Spinner<Integer> binsSpinner;
    @FXML private Button refreshButton, runButton, helpButton, exportPngButton, exportSvgButton;
    @FXML private CheckBox showHist, showG1, showS, showG2, showModel;
    @FXML private Slider xZoom;
    @FXML private Label statusLabel, g1Label, sLabel, g2Label, cvLabel, fitLabel;
    @FXML private AnalysisChart chart;

    private AppContext ctx;

    @FXML
    public void initialize() {
        modelCombo.setItems(FXCollections.observableArrayList("watson", "djf"));
        modelCombo.getSelectionModel().select("watson");
        binsSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(64, 1024, 256, 32));
        chart.setAxisLabels("DNA content", "Count");
        showHist.setOnAction(e -> chart.setVisible("Histogram", showHist.isSelected()));
        showG1.setOnAction(e -> chart.setVisible("G0/G1", showG1.isSelected()));
        showS.setOnAction(e -> chart.setVisible("S", showS.isSelected()));
        showG2.setOnAction(e -> chart.setVisible("G2/M", showG2.isSelected()));
        showModel.setOnAction(e -> chart.setVisible("Model", showModel.isSelected()));
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
            String dnaGuess = null;
            for (JsonNode c : r.path("channels")) {
                String ch = c.asText();
                channelCombo.getItems().add(ch);
                if (dnaGuess == null && DNA.matcher(ch).find()) dnaGuess = ch;
            }
            if (dnaGuess != null) channelCombo.getSelectionModel().select(dnaGuess);
            else if (!channelCombo.getItems().isEmpty()) channelCombo.getSelectionModel().selectFirst();
        });
    }

    @FXML
    private void onRun() {
        if (ctx == null || sampleCombo.getValue() == null || channelCombo.getValue() == null) return;
        ObjectNode a = JSON.createObjectNode();
        a.put("sample", sampleCombo.getValue());
        a.put("channel", channelCombo.getValue());
        a.put("model", modelCombo.getValue());
        a.put("bins", binsSpinner.getValue());
        statusLabel.setText("Fitting " + modelCombo.getValue().toUpperCase() + " model to " + channelCombo.getValue() + "…");
        ctx.jobs().run(ctx.bridge().command("run_cell_cycle", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        JsonNode ph = result.path("phases"), cv = result.path("cvs"), fit = result.path("fit");
        g1Label.setText(String.format("G0/G1:  %.1f%%", ph.path("G0G1").asDouble()));
        sLabel.setText(String.format("S:  %.1f%%", ph.path("S").asDouble()));
        g2Label.setText(String.format("G2/M:  %.1f%%", ph.path("G2M").asDouble()));
        cvLabel.setText(String.format("CV — G0/G1 %.1f%%   G2/M %.1f%%", cv.path("G0G1").asDouble(), cv.path("G2M").asDouble()));
        fitLabel.setText(String.format("μ G1 %.0f → G2 %.0f%nR² %.3f · RMSE %.1f · n=%,d",
                fit.path("mu_g1").asDouble(), fit.path("mu_g2").asDouble(),
                fit.path("r2").asDouble(), fit.path("rmse").asDouble(), result.path("n").asLong()));

        JsonNode c = result.path("curves");
        double[] x = arr(c.path("x"));
        chart.clearSeries();
        chart.setX(x);
        chart.addSeries("Histogram", arr(c.path("hist")), Color.web("#9DB4CC"), AnalysisChart.Kind.BARS);
        chart.addSeries("S", arr(c.path("s")), Color.web("#F6A623"), AnalysisChart.Kind.AREA);
        chart.addSeries("G0/G1", arr(c.path("g1")), Color.web("#2C7FB8"), AnalysisChart.Kind.LINE);
        chart.addSeries("G2/M", arr(c.path("g2")), Color.web("#D7301F"), AnalysisChart.Kind.LINE);
        AnalysisChart.Series model = chart.addSeries("Model", arr(c.path("total")), Color.web("#111111"), AnalysisChart.Kind.LINE);
        model.dashed = true;
        chart.setTitle("Cell cycle · " + modelCombo.getValue().toUpperCase() + " · " + sampleCombo.getValue());
        chart.setVisible("Histogram", showHist.isSelected());
        chart.setVisible("G0/G1", showG1.isSelected());
        chart.setVisible("S", showS.isSelected());
        chart.setVisible("G2/M", showG2.isSelected());
        chart.setVisible("Model", showModel.isSelected());
        chart.setXMaxFraction(xZoom.getValue());
        chart.refresh();

        double r2 = fit.path("r2").asDouble();
        statusLabel.setText("Fit complete (R²=" + String.format("%.3f", r2) + ")"
                + (r2 < 0.9 ? " — low R²; check the DNA channel/model (use PI/DAPI/7-AAD/Hoechst)." : "."));
        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                String.format("Cell Cycle (%s): G1=%.1f%% S=%.1f%% G2/M=%.1f%% R²=%.3f",
                        modelCombo.getValue(), ph.path("G0G1").asDouble(), ph.path("S").asDouble(),
                        ph.path("G2M").asDouble(), r2));
    }

    @FXML
    private void onExportPng() {
        int dpi = ctx != null ? ctx.settings().exportDpi() : 300;
        javafx.scene.image.WritableImage img = chart.snapshotAtDpi(dpi);
        if (img == null) { statusLabel.setText("Nothing to export — run a fit first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export cell-cycle plot (PNG)");
        fc.setInitialFileName("cellcycle.png");
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
        fc.setTitle("Export cell-cycle plot (SVG)");
        fc.setInitialFileName("cellcycle.svg");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("SVG vector (*.svg)", "*.svg"));
        File f = fc.showSaveDialog(chart.getScene().getWindow());
        if (f == null) return;
        try {
            Files.writeString(f.toPath(), chart.toSvg());
            statusLabel.setText("Exported SVG → " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("SVG export failed: " + e.getMessage());
        }
    }

    @FXML
    private void onHelp() {
        String msg =
            "CELL CYCLE\n" +
            "Fits a DNA-content histogram (stain a fixed/permeabilised sample with PI, DAPI, 7-AAD, " +
            "Hoechst or FxCycle) to estimate the fraction of cells in G0/G1, S and G2/M.\n\n" +
            "MODELS\n" +
            "• Watson — G0/G1 and G2/M are Gaussians (G2/M mean = 2× G0/G1) with equal CV; S phase is a " +
            "broadened rectangle. Robust default.\n" +
            "• Dean-Jett-Fox (djf) — same but the G2/M width is free; use when G2/M is broad.\n\n" +
            "THIS VIEW IS INTERACTIVE\n" +
            "Toggle the histogram / each component, and use X-zoom to focus on G0/G1+S. When it looks right, " +
            "Export PNG (at your Settings DPI) or SVG for publication.\n\n" +
            "TIP: a low R² usually means the channel isn't DNA content, or there's debris — gate out debris " +
            "first, then re-fit.";
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle("About Cell Cycle");
        a.setHeaderText("DNA-content modelling (Watson / Dean-Jett-Fox)");
        TextArea ta = new TextArea(msg);
        ta.setEditable(false); ta.setWrapText(true); ta.setPrefSize(560, 340);
        a.getDialogPane().setContent(ta);
        a.showAndWait();
    }

    private static double[] arr(JsonNode n) {
        if (n == null || !n.isArray()) return new double[0];
        double[] a = new double[n.size()];
        for (int i = 0; i < n.size(); i++) a[i] = n.get(i).asDouble();
        return a;
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d); channelCombo.setDisable(d); modelCombo.setDisable(d);
        binsSpinner.setDisable(d); refreshButton.setDisable(d); runButton.setDisable(d);
        exportPngButton.setDisable(d); exportSvgButton.setDisable(d);
    }
}
