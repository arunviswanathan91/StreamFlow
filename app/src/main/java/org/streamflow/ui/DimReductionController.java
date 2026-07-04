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

public class DimReductionController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private VBox hintPane;
    @FXML private ComboBox<String> methodCombo;
    @FXML private ComboBox<String> sampleCombo;
    @FXML private Spinner<Integer> eventsSpinner;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Button copyButton;
    @FXML private Button exportButton;
    @FXML private Label statusLabel;
    @FXML private StackPane mapPane;
    @FXML private Canvas mapCanvas;

    private AppContext ctx;
    private double[] coordsX, coordsY;
    private String lastMethod = "UMAP";

    @FXML
    public void initialize() {
        methodCombo.setItems(FXCollections.observableArrayList("umap", "tsne"));
        methodCombo.getSelectionModel().select("umap");
        eventsSpinner.setValueFactory(
                new SpinnerValueFactory.IntegerSpinnerValueFactory(500, 200000, 5000, 500));
        copyButton.setDisable(true); exportButton.setDisable(true);
        mapPane.widthProperty().addListener((o, a, b) -> redrawIfReady());
        mapPane.heightProperty().addListener((o, a, b) -> redrawIfReady());
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
    public void refreshFromWorkspace() {
        refreshSamples();
        refreshHints();
    }

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
                "Apply Compensation and Transformation before running UMAP / t-SNE for best results."));
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
        a.put("n_events", eventsSpinner.getValue());
        statusLabel.setText("Running " + methodCombo.getValue().toUpperCase()
                + " (this can take a while — the UI stays responsive)…");
        ctx.jobs().run(ctx.bridge().command("run_dimredux", a), this::showMap);
    }

    private void showMap(JsonNode result) {
        JsonNode coords = result.path("coords");
        int n = coords.size();
        coordsX = new double[n]; coordsY = new double[n];
        for (int i = 0; i < n; i++) {
            coordsX[i] = coords.get(i).get(0).asDouble();
            coordsY[i] = coords.get(i).get(1).asDouble();
        }
        lastMethod = result.path("method").asText().toUpperCase();
        redrawIfReady();
        statusLabel.setText(lastMethod + " — " + result.path("n").asInt()
                + " events, " + result.path("n_features").asInt() + " features.");
        ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                lastMethod + ": " + n + " events");
        copyButton.setDisable(false); exportButton.setDisable(false);
    }

    private void redrawIfReady() {
        if (coordsX == null || coordsX.length == 0) return;
        double W = mapPane.getWidth(), H = mapPane.getHeight();
        if (W < 10 || H < 10) return;
        mapCanvas.setWidth(W); mapCanvas.setHeight(H);
        drawScatter(mapCanvas.getGraphicsContext2D(), W, H);
    }

    private void drawScatter(GraphicsContext g, double W, double H) {
        g.clearRect(0, 0, W, H);
        g.setFill(Color.WHITE); g.fillRect(0, 0, W, H);

        double ML = 52, MR = 14, MT = 34, MB = 46;
        double pw = W - ML - MR, ph = H - MT - MB;
        if (pw < 10 || ph < 10) return;

        double xMin = coordsX[0], xMax = coordsX[0], yMin = coordsY[0], yMax = coordsY[0];
        for (double v : coordsX) { xMin = Math.min(xMin, v); xMax = Math.max(xMax, v); }
        for (double v : coordsY) { yMin = Math.min(yMin, v); yMax = Math.max(yMax, v); }
        double xRange = Math.max(1e-9, xMax - xMin), yRange = Math.max(1e-9, yMax - yMin);
        double pad = 0.04;
        xMin -= xRange * pad; xMax += xRange * pad; xRange *= (1 + 2 * pad);
        yMin -= yRange * pad; yMax += yRange * pad; yRange *= (1 + 2 * pad);

        // grid lines
        g.setStroke(Color.web("#EEEEEE")); g.setLineWidth(0.5);
        for (int k = 0; k <= 4; k++) {
            double xp = ML + k / 4.0 * pw, yp = MT + k / 4.0 * ph;
            g.strokeLine(xp, MT, xp, MT + ph);
            g.strokeLine(ML, yp, ML + pw, yp);
        }

        // dots
        int n = coordsX.length;
        double alpha = n > 10000 ? 0.15 : n > 2000 ? 0.25 : 0.45;
        Color dotColor = new Color(0.031, 0.188, 0.420, alpha);
        double r = n > 5000 ? 1.5 : 2.5;
        g.setFill(dotColor);
        for (int i = 0; i < n; i++) {
            double px = ML + (coordsX[i] - xMin) / xRange * pw;
            double py = MT + ph - (coordsY[i] - yMin) / yRange * ph;
            g.fillOval(px - r, py - r, r * 2, r * 2);
        }

        // frame
        g.setStroke(Color.web("#666666")); g.setLineWidth(1);
        g.strokeRect(ML, MT, pw, ph);

        // ticks + labels
        String xLbl = lastMethod + " 1", yLbl = lastMethod + " 2";
        g.setFont(Font.font("Segoe UI", 9)); g.setFill(Color.web("#555555"));
        for (int k = 0; k <= 4; k++) {
            double xv = xMin + k / 4.0 * xRange, yv = yMin + k / 4.0 * yRange;
            g.fillText(fmt(xv), ML + k / 4.0 * pw - 10, MT + ph + 15);
            g.fillText(fmt(yv), 2, MT + ph - k / 4.0 * ph + 4);
        }
        g.setFont(Font.font("Segoe UI", 11)); g.setFill(Color.web("#222222"));
        g.fillText(xLbl, ML + pw / 2 - xLbl.length() * 3.5, MT + ph + 34);
        g.save(); g.translate(12, MT + ph / 2 + yLbl.length() * 3.5); g.rotate(-90);
        g.fillText(yLbl, 0, 0); g.restore();

        // title
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 12));
        g.setFill(Color.web("#1A1A1A"));
        g.fillText(lastMethod + "  n=" + n, ML, 22);
    }

    private static String fmt(double v) {
        double a = Math.abs(v);
        if (a >= 100) return String.format("%.0f", v);
        if (a >= 1) return String.format("%.1f", v);
        return String.format("%.2f", v);
    }

    @FXML private void onCopy() {
        WritableImage img = mapCanvas.snapshot(null, null);
        ClipboardContent cc = new ClipboardContent(); cc.putImage(img);
        Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied to clipboard.");
    }

    @FXML private void onExport() {
        FileChooser fc = new FileChooser();
        fc.setTitle("Export " + lastMethod);
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG Image", "*.png"));
        File f = fc.showSaveDialog(mapCanvas.getScene().getWindow());
        if (f == null) return;
        try {
            WritableImage img = mapCanvas.snapshot(null, null);
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
