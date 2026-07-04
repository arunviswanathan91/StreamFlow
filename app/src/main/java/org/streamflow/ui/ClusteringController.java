package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.geometry.Pos;
import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
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
import java.util.List;

public class ClusteringController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private VBox hintPane;
    @FXML private ComboBox<String> methodCombo;
    @FXML private ComboBox<String> sampleCombo;
    @FXML private Spinner<Integer> kSpinner;
    @FXML private Spinner<Integer> eventsSpinner;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Button copyButton;
    @FXML private Button exportButton;
    @FXML private Label statusLabel;
    @FXML private StackPane heatmapPane;
    @FXML private Canvas heatmapCanvas;
    @FXML private ListView<String> clusterList;

    private AppContext ctx;

    // current data for resize-redraw
    private List<String>     plotChannels;
    private double[][]       plotMedians;  // [cluster][channel]
    private int              plotK;

    @FXML
    public void initialize() {
        methodCombo.setItems(FXCollections.observableArrayList("flowsom", "phenograph"));
        methodCombo.getSelectionModel().select("flowsom");
        kSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(2, 50, 10, 1));
        eventsSpinner.setValueFactory(
                new SpinnerValueFactory.IntegerSpinnerValueFactory(1000, 500000, 20000, 1000));
        kSpinner.disableProperty().bind(
                methodCombo.getSelectionModel().selectedItemProperty().isEqualTo("phenograph"));
        copyButton.setDisable(true); exportButton.setDisable(true);
        heatmapPane.widthProperty().addListener((o, a, b) -> redrawIfReady());
        heatmapPane.heightProperty().addListener((o, a, b) -> redrawIfReady());
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshSamples();
        refreshHints();
        ctx.workspace().sampleNames().addListener(
                (javafx.collections.ListChangeListener<String>) c -> refreshHints());
    }

    @FXML private void onRefresh() { refreshSamples(); }

    @Override
    public void refreshFromWorkspace() { refreshSamples(); refreshHints(); }

    private void refreshSamples() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().clear();
            r.path("samples").forEach(n -> sampleCombo.getItems().add(n.asText()));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();
        });
    }

    private void refreshHints() {
        if (hintPane == null || ctx == null) return;
        hintPane.getChildren().clear();
        boolean hasFcs = !ctx.workspace().sampleNames().isEmpty();
        if (hasFcs) { hintPane.setVisible(false); hintPane.setManaged(false); return; }
        hintPane.getChildren().add(hintRow(false,
                "No FCS files loaded — use File ▸ Load FCS… to load your data.", "Workstation"));
        hintPane.getChildren().add(hintInfoRow(
                "Apply Compensation and Transformation before clustering for best results."));
        hintPane.setVisible(true); hintPane.setManaged(true);
    }

    @FXML
    private void onRun() {
        if (ctx == null) return;
        if (sampleCombo.getValue() == null) {
            statusLabel.setText("Load FCS files first, then pick a sample.");
            return;
        }
        ObjectNode a = JSON.createObjectNode();
        a.put("method", methodCombo.getValue());
        a.put("sample", sampleCombo.getValue());
        a.put("k", kSpinner.getValue());
        a.put("n_events", eventsSpinner.getValue());
        statusLabel.setText("Clustering with " + methodCombo.getValue() + "…");
        ctx.jobs().run(ctx.bridge().command("run_clustering", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        // parse channels + medians
        JsonNode chArr = result.path("channels");
        JsonNode mArr  = result.path("medians");
        int nChan = chArr.size(), nClust = mArr.size();
        plotChannels = new ArrayList<>(nChan);
        for (JsonNode c : chArr) plotChannels.add(c.asText());
        plotMedians = new double[nClust][nChan];
        for (int c = 0; c < nClust; c++)
            for (int f = 0; f < nChan; f++)
                plotMedians[c][f] = mArr.get(c).get(f).asDouble();
        plotK = result.path("k").asInt();

        clusterList.getItems().clear();
        for (JsonNode c : result.path("clusters"))
            clusterList.getItems().add(String.format("C%d — %d events (%.1f%%)",
                    c.path("cluster").asInt(), c.path("count").asInt(), c.path("percent").asDouble()));

        redrawIfReady();
        statusLabel.setText(result.path("method").asText() + " — " + plotK
                + " clusters over " + result.path("n").asInt() + " events.");
        ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                "Clustering: " + result.path("method").asText() + " k=" + plotK);
        copyButton.setDisable(false); exportButton.setDisable(false);
    }

    private void redrawIfReady() {
        if (plotChannels == null || plotMedians == null) return;
        double W = heatmapPane.getWidth(), H = heatmapPane.getHeight();
        if (W < 10 || H < 10) return;
        heatmapCanvas.setWidth(W); heatmapCanvas.setHeight(H);
        drawHeatmap(heatmapCanvas.getGraphicsContext2D(), W, H);
    }

    private void drawHeatmap(GraphicsContext g, double W, double H) {
        g.clearRect(0, 0, W, H);
        g.setFill(Color.WHITE); g.fillRect(0, 0, W, H);

        int nClust = plotMedians.length, nChan = plotChannels.size();
        if (nClust == 0 || nChan == 0) return;

        // Compute z-scores per channel (normalise across clusters)
        double[][] z = new double[nClust][nChan];
        for (int f = 0; f < nChan; f++) {
            double mean = 0;
            for (int c = 0; c < nClust; c++) mean += plotMedians[c][f];
            mean /= nClust;
            double std = 0;
            for (int c = 0; c < nClust; c++) std += Math.pow(plotMedians[c][f] - mean, 2);
            std = Math.sqrt(std / nClust);
            if (std < 1e-9) std = 1;
            for (int c = 0; c < nClust; c++) z[c][f] = (plotMedians[c][f] - mean) / std;
        }

        double ML = 48, MR = 100, MT = 28, MB = 60; // MB for rotated channel labels
        double pw = W - ML - MR, ph = H - MT - MB;
        if (pw < 10 || ph < 10) return;

        double cw = pw / nChan, ch = ph / nClust;

        // draw cells
        for (int c = 0; c < nClust; c++) {
            for (int f = 0; f < nChan; f++) {
                g.setFill(diverging(z[c][f]));
                g.fillRect(ML + f * cw, MT + c * ch, cw, ch);
            }
        }

        // cell borders
        g.setStroke(Color.WHITE); g.setLineWidth(0.5);
        for (int c = 1; c < nClust; c++) g.strokeLine(ML, MT + c * ch, ML + pw, MT + c * ch);
        for (int f = 1; f < nChan; f++) g.strokeLine(ML + f * cw, MT, ML + f * cw, MT + ph);

        // frame
        g.setStroke(Color.web("#888")); g.setLineWidth(1);
        g.strokeRect(ML, MT, pw, ph);

        // cluster labels (left)
        g.setFont(Font.font("Segoe UI", Math.min(11, ch - 2))); g.setFill(Color.web("#222"));
        for (int c = 0; c < nClust; c++)
            g.fillText("C" + c, 2, MT + c * ch + ch / 2 + 4);

        // channel labels (bottom, rotated)
        g.setFont(Font.font("Segoe UI", Math.min(10, cw - 2)));
        for (int f = 0; f < nChan; f++) {
            double cx = ML + f * cw + cw / 2;
            g.save();
            g.translate(cx, MT + ph + 4);
            g.rotate(45);
            g.setFill(Color.web("#333"));
            g.fillText(plotChannels.get(f), 0, 0);
            g.restore();
        }

        // colour scale bar (right)
        double sbX = ML + pw + 16, sbY = MT, sbH = ph;
        for (int py = 0; py < (int) sbH; py++) {
            double t = 1.0 - (double) py / sbH; // 1=high z=2, 0=low z=-2
            g.setFill(diverging((t - 0.5) * 4));
            g.fillRect(sbX, sbY + py, 14, 1);
        }
        g.setStroke(Color.web("#888")); g.setLineWidth(0.5);
        g.strokeRect(sbX, sbY, 14, sbH);
        g.setFont(Font.font("Segoe UI", 9)); g.setFill(Color.web("#555"));
        g.fillText("+2", sbX + 16, sbY + 10);
        g.fillText(" 0", sbX + 16, sbY + sbH / 2 + 4);
        g.fillText("-2", sbX + 16, sbY + sbH - 2);

        // title
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 12));
        g.setFill(Color.web("#1A1A1A"));
        g.fillText("Cluster medians (z-score)  k=" + plotK, ML, 20);
    }

    /** Diverging colour: blue (-2) → white (0) → red (+2), clamped. */
    private static Color diverging(double z) {
        double t = Math.max(-2, Math.min(2, z)) / 2.0; // -1..+1
        if (t >= 0) return new Color(1, 1 - t, 1 - t, 1); // white → red
        else        return new Color(1 + t, 1 + t, 1, 1); // blue → white
    }

    @FXML private void onCopy() {
        WritableImage img = heatmapCanvas.snapshot(null, null);
        ClipboardContent cc = new ClipboardContent(); cc.putImage(img);
        Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied to clipboard.");
    }

    @FXML private void onExport() {
        FileChooser fc = new FileChooser();
        fc.setTitle("Export Heatmap");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG Image", "*.png"));
        File f = fc.showSaveDialog(heatmapCanvas.getScene().getWindow());
        if (f == null) return;
        try {
            WritableImage img = heatmapCanvas.snapshot(null, null);
            BufferedImage bi = javafx.embed.swing.SwingFXUtils.fromFXImage(img, null);
            ImageIO.write(bi, "png", f);
            statusLabel.setText("Saved: " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("Export failed: " + e.getMessage());
        }
    }

    private void setDisabled(boolean d) {
        methodCombo.setDisable(d);
        sampleCombo.setDisable(d);
        eventsSpinner.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }

    // ---- hint row builders --------------------------------------------------

    private HBox hintRow(boolean ok, String text, String navTarget) {
        HBox row = new HBox(8); row.setAlignment(Pos.CENTER_LEFT);
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
        HBox row = new HBox(8); row.setAlignment(Pos.CENTER_LEFT);
        row.getStyleClass().add("hint-info");
        FontIcon icon = new FontIcon("fas-info-circle"); icon.setIconSize(14);
        Label lbl = new Label(text); lbl.setWrapText(true);
        row.getChildren().addAll(icon, lbl);
        return row;
    }
}
