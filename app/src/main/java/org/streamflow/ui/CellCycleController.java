package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
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
 * Analysis module A1 — Cell Cycle. Fits Watson / Dean-Jett-Fox to a DNA-content
 * histogram. Supports gating via a user-selectable gate combo, draggable G1/G2 peak
 * anchors, and drag-to-zoom on the chart canvas.
 */
public class CellCycleController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern DNA = Pattern.compile("PI|DAPI|7.?AAD|Hoechst|DNA|FxCycle|DRAQ",
            Pattern.CASE_INSENSITIVE);

    // ---- FXML fields --------------------------------------------------------

    @FXML private ComboBox<String> sampleCombo, channelCombo, modelCombo;
    @FXML private ComboBox<String> gateCombo;
    @FXML private Label            gateWarningLabel;
    @FXML private Spinner<Integer> binsSpinner;
    @FXML private Button           refreshButton, runButton, helpButton, copyButton, exportPngButton, exportSvgButton;
    @FXML private CheckBox         showHist, showG1, showS, showG2, showModel;
    @FXML private Slider           xZoom;
    @FXML private Label            statusLabel, g1Label, sLabel, g2Label, cvLabel, fitLabel;
    @FXML private AnalysisChart    chart;

    // ---- State --------------------------------------------------------------

    private AppContext ctx;
    /** Last user-dragged G1 / G2 peak position; NaN = let engine auto-detect. */
    private double lastMuG1 = Double.NaN, lastMuG2 = Double.NaN;

    // ---- Lifecycle ----------------------------------------------------------

    @FXML
    public void initialize() {
        modelCombo.setItems(FXCollections.observableArrayList("watson", "djf"));
        modelCombo.getSelectionModel().select("watson");
        binsSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(64, 1024, 256, 32));
        chart.setAxisLabels("DNA content", "Count");
        chart.setZoomBoxEnabled(true);

        showHist.setOnAction(e -> chart.setVisible("Histogram", showHist.isSelected()));
        showG1.setOnAction(e  -> chart.setVisible("G0/G1",     showG1.isSelected()));
        showS.setOnAction(e   -> chart.setVisible("S",          showS.isSelected()));
        showG2.setOnAction(e  -> chart.setVisible("G2/M",       showG2.isSelected()));
        showModel.setOnAction(e -> chart.setVisible("Model",   showModel.isSelected()));
        xZoom.valueProperty().addListener((o, a, b) -> chart.setXMaxFraction(b.doubleValue()));

        // Refresh gate combo whenever the sample selection changes
        sampleCombo.getSelectionModel().selectedItemProperty().addListener(
                (o, a, b) -> refreshGates(b));

        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        // Auto-refresh gate list whenever any gate in the workspace changes
        ctx.workspace().addTreeChangeListener(() ->
                refreshGates(sampleCombo.getValue()));
        refreshSamples();
    }

    @FXML private void onRefresh() { refreshSamples(); }
    @Override public void refreshFromWorkspace() { refreshSamples(); }

    // ---- Sample / channel refresh -------------------------------------------

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

            // Gate combo is driven by sampleCombo listener; kick it explicitly here too
            refreshGates(sampleCombo.getValue());
        });
    }

    // ---- Gate combo ---------------------------------------------------------

    private void refreshGates(String sample) {
        if (ctx == null || sample == null) return;
        gateCombo.getItems().clear();
        gateCombo.getItems().add("Ungated (All Events)");

        PopNode root = ctx.workspace().treeFor(sample);
        for (PopNode n : root.selfAndDescendants()) {
            if (!n.isRoot()) gateCombo.getItems().add(n.name());
        }
        gateCombo.getSelectionModel().selectFirst();
        updateGateWarning();
    }

    private void updateGateWarning() {
        // Show warning icon when no user-defined gates exist (only the "Ungated" placeholder)
        boolean noGates = gateCombo.getItems().size() <= 1;
        gateWarningLabel.setVisible(noGates);
        gateWarningLabel.setManaged(noGates);
    }

    /** Return the PopNode for the current gateCombo selection, or null for ungated. */
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

    // ---- Run ----------------------------------------------------------------

    @FXML
    private void onRun() {
        if (ctx == null || sampleCombo.getValue() == null || channelCombo.getValue() == null) return;

        ObjectNode a = JSON.createObjectNode();
        a.put("sample",  sampleCombo.getValue());
        a.put("channel", channelCombo.getValue());
        a.put("model",   modelCombo.getValue());
        a.put("bins",    binsSpinner.getValue());

        // Serialize the selected gate chain into gate_polygons for the engine
        PopNode target = selectedGateNode();
        if (target != null) {
            ArrayNode polygons = JSON.createArrayNode();
            for (CytoPlot.Gate g : target.chain()) {
                ObjectNode gn = JSON.createObjectNode();
                gn.put("type",      g.type);
                gn.put("x_channel", g.xChan);
                gn.put("y_channel", g.yChan != null ? g.yChan : "");
                ArrayNode xs = JSON.createArrayNode(); for (double v : g.xs) xs.add(v);
                ArrayNode ys = JSON.createArrayNode(); for (double v : g.ys) ys.add(v);
                gn.set("xs", xs);
                gn.set("ys", ys);
                polygons.add(gn);
            }
            a.set("gate_polygons", polygons);
        }

        // Pass user-dragged peak anchors as hints to seed the fit
        if (!Double.isNaN(lastMuG1)) a.put("mu_g1_hint", lastMuG1);
        if (!Double.isNaN(lastMuG2)) a.put("mu_g2_hint", lastMuG2);

        String pop = target != null ? target.name() : "All Events";
        statusLabel.setStyle("");
        statusLabel.setText("Fitting " + modelCombo.getValue().toUpperCase()
                + " on " + channelCombo.getValue() + " (" + pop + ")…");
        ctx.jobs().run(ctx.bridge().command("run_cell_cycle", a), this::showResult);
    }

    // ---- Show result --------------------------------------------------------

    private void showResult(JsonNode result) {
        JsonNode ph  = result.path("phases");
        JsonNode cv  = result.path("cvs");
        JsonNode fit = result.path("fit");

        g1Label.setText(String.format("G0/G1:  %.1f%%", ph.path("G0G1").asDouble()));
        sLabel.setText(String.format("S:  %.1f%%",      ph.path("S").asDouble()));
        g2Label.setText(String.format("G2/M:  %.1f%%",  ph.path("G2M").asDouble()));
        cvLabel.setText(String.format("CV — G0/G1 %.1f%%   G2/M %.1f%%",
                cv.path("G0G1").asDouble(), cv.path("G2M").asDouble()));
        fitLabel.setText(String.format("μ G1 %.0f → G2 %.0f%nR² %.3f · RMSE %.1f · n=%,d",
                fit.path("mu_g1").asDouble(), fit.path("mu_g2").asDouble(),
                fit.path("r2").asDouble(), fit.path("rmse").asDouble(), result.path("n").asLong()));

        JsonNode c = result.path("curves");
        double[] x = arr(c.path("x"));
        chart.clearSeries();
        chart.setX(x);
        chart.addSeries("Histogram", arr(c.path("hist")), Color.web("#9DB4CC"), AnalysisChart.Kind.BARS);
        chart.addSeries("S",         arr(c.path("s")),    Color.web("#F6A623"), AnalysisChart.Kind.AREA);
        chart.addSeries("G0/G1",     arr(c.path("g1")),   Color.web("#2C7FB8"), AnalysisChart.Kind.LINE);
        chart.addSeries("G2/M",      arr(c.path("g2")),   Color.web("#D7301F"), AnalysisChart.Kind.LINE);
        AnalysisChart.Series modelSeries = chart.addSeries("Model", arr(c.path("total")),
                Color.web("#111111"), AnalysisChart.Kind.LINE);
        modelSeries.dashed = true;

        String pop = gateCombo.getValue() != null && !gateCombo.getValue().startsWith("Ungated")
                ? gateCombo.getValue() : "All Events";
        chart.setTitle("Cell cycle · " + modelCombo.getValue().toUpperCase()
                + " · " + sampleCombo.getValue() + " [" + pop + "]");
        chart.setVisible("Histogram", showHist.isSelected());
        chart.setVisible("G0/G1",     showG1.isSelected());
        chart.setVisible("S",         showS.isSelected());
        chart.setVisible("G2/M",      showG2.isSelected());
        chart.setVisible("Model",     showModel.isSelected());
        chart.setXMaxFraction(xZoom.getValue());

        // Wire up draggable G1/G2 peak anchors — dragging stores hints and re-runs the fit
        double muG1 = fit.path("mu_g1").asDouble();
        double muG2 = fit.path("mu_g2").asDouble();
        lastMuG1 = muG1;
        lastMuG2 = muG2;
        chart.setPeaks(muG1, muG2, peaks -> {
            lastMuG1 = peaks[0];
            lastMuG2 = peaks[1];
            onRun();
        });

        chart.refresh();

        // Colour status by R² quality
        double r2 = fit.path("r2").asDouble();
        if (r2 < 0.5) {
            statusLabel.setStyle("-fx-text-fill: #EF4444;");
            statusLabel.setText(String.format("Poor fit (R²=%.3f) — wrong channel or gating needed.", r2));
        } else if (r2 < 0.9) {
            statusLabel.setStyle("-fx-text-fill: #F59E0B;");
            statusLabel.setText(String.format("Moderate fit (R²=%.3f) — try adjusting peaks or model.", r2));
        } else {
            statusLabel.setStyle("-fx-text-fill: #22C55E;");
            statusLabel.setText(String.format("Good fit (R²=%.3f).", r2));
        }

        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                String.format("Cell Cycle (%s, gate=%s): G1=%.1f%% S=%.1f%% G2/M=%.1f%% R²=%.3f",
                        modelCombo.getValue(), pop,
                        ph.path("G0G1").asDouble(), ph.path("S").asDouble(),
                        ph.path("G2M").asDouble(), r2));
    }

    // ---- Export / Copy ------------------------------------------------------

    @FXML
    private void onCopy() {
        if (ctx == null) return;
        javafx.scene.image.WritableImage img = chart.exportImage(ctx.settings());
        if (img == null) { statusLabel.setStyle(""); statusLabel.setText("Nothing to copy — run a fit first."); return; }
        javafx.scene.input.ClipboardContent cc = new javafx.scene.input.ClipboardContent();
        cc.putImage(img);
        javafx.scene.input.Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setStyle("");
        statusLabel.setText("Copied " + ctx.settings().exportDpi() + " DPI cell cycle plot — paste into PowerPoint.");
    }

    @FXML
    private void onExportPng() {
        if (ctx == null) return;
        javafx.scene.image.WritableImage img = chart.exportImage(ctx.settings());
        if (img == null) { statusLabel.setStyle(""); statusLabel.setText("Nothing to export — run a fit first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export cell-cycle plot (PNG)");
        fc.setInitialFileName("cellcycle.png");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"));
        File f = fc.showSaveDialog(chart.getScene().getWindow());
        if (f == null) return;
        try {
            ImageIO.write(javafx.embed.swing.SwingFXUtils.fromFXImage(img, null), "png", f);
            statusLabel.setStyle("");
            statusLabel.setText("Exported PNG at " + ctx.settings().exportDpi() + " DPI → " + f.getName());
        } catch (Exception e) {
            statusLabel.setStyle("");
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
            statusLabel.setStyle("");
            statusLabel.setText("Exported SVG → " + f.getName());
        } catch (Exception e) {
            statusLabel.setStyle("");
            statusLabel.setText("SVG export failed: " + e.getMessage());
        }
    }

    // ---- Help ---------------------------------------------------------------

    @FXML
    private void onHelp() {
        String msg =
            "CELL CYCLE\n" +
            "Fits a DNA-content histogram (stain a fixed/permeabilised sample with PI, DAPI, 7-AAD, " +
            "Hoechst or FxCycle) to estimate the fraction of cells in G0/G1, S and G2/M.\n\n" +
            "GATE SELECTION\n" +
            "Choose a gate from the Gate dropdown. In a standard workflow, gate on Singlets and Live " +
            "cells before running cell cycle to exclude doublets and dead cells.\n\n" +
            "MODELS\n" +
            "• Watson — G0/G1 and G2/M are Gaussians (G2/M mean = 2× G0/G1) with equal CV; S phase is " +
            "a broadened rectangle. Robust default.\n" +
            "• Dean-Jett-Fox (djf) — same but the G2/M width is free; use when G2/M is broad.\n\n" +
            "INTERACTIVE FEATURES\n" +
            "• Drag the blue (G1) or red (G2) line to nudge the fit if peak detection missed.\n" +
            "• Draw a zoom box by clicking and dragging on the chart; double-click to reset.\n" +
            "• Toggle histogram and component curves below the chart.\n\n" +
            "TIP: a low R² (red status) usually means wrong channel or un-gated debris. " +
            "Gate on Singlets/Live first, then re-fit.";
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle("About Cell Cycle");
        a.setHeaderText("DNA-content modelling (Watson / Dean-Jett-Fox)");
        TextArea ta = new TextArea(msg);
        ta.setEditable(false); ta.setWrapText(true); ta.setPrefSize(560, 380);
        a.getDialogPane().setContent(ta);
        a.showAndWait();
    }

    // ---- Helpers ------------------------------------------------------------

    private static double[] arr(JsonNode n) {
        if (n == null || !n.isArray()) return new double[0];
        double[] a = new double[n.size()];
        for (int i = 0; i < n.size(); i++) a[i] = n.get(i).asDouble();
        return a;
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d);   gateCombo.setDisable(d);
        channelCombo.setDisable(d);  modelCombo.setDisable(d);
        binsSpinner.setDisable(d);   refreshButton.setDisable(d);
        runButton.setDisable(d);     copyButton.setDisable(d);
        exportPngButton.setDisable(d); exportSvgButton.setDisable(d);
    }
}
