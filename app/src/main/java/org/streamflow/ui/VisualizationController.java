package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;
import javafx.stage.Screen;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * Phase 3 — Visualization. Renders a density scatter or histogram for a chosen
 * sample/channels in R and shows the returned PNG. Passes the display DPI scale
 * so plots are crisp on high-DPI screens (the same render path the gating canvas
 * will reuse).
 */
public class VisualizationController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private ComboBox<String> sampleCombo;
    @FXML private ComboBox<String> xCombo;
    @FXML private ComboBox<String> yCombo;
    @FXML private ComboBox<String> typeCombo;
    @FXML private Button refreshButton;
    @FXML private Button renderButton;
    @FXML private Label statusLabel;
    @FXML private ImageView plotView;

    private AppContext ctx;

    @FXML
    public void initialize() {
        typeCombo.setItems(FXCollections.observableArrayList("scatter", "histogram"));
        typeCombo.getSelectionModel().select("scatter");
        // Y channel only matters for scatter.
        yCombo.disableProperty().bind(
                typeCombo.getSelectionModel().selectedItemProperty().isEqualTo("histogram"));
        plotView.setPreserveRatio(true);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshChannels();
    }

    @FXML
    private void onRefresh() { refreshChannels(); }
    @Override public void refreshFromWorkspace() { refreshChannels(); }

    private void refreshChannels() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().setAll(toList(r.path("samples")));
            xCombo.getItems().setAll(toList(r.path("channels")));
            yCombo.getItems().setAll(toList(r.path("channels")));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();
            if (xCombo.getItems().size() > 0) xCombo.getSelectionModel().selectFirst();
            if (yCombo.getItems().size() > 1) yCombo.getSelectionModel().select(1);
            statusLabel.setText(xCombo.getItems().size() + " channel(s) available.");
        });
    }

    @FXML
    private void onRender() {
        if (ctx == null || xCombo.getValue() == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sampleCombo.getValue());
        args.put("x", xCombo.getValue());
        args.put("type", typeCombo.getValue());
        if ("scatter".equals(typeCombo.getValue()) && yCombo.getValue() != null) {
            args.put("y", yCombo.getValue());
        }
        args.put("width", 800).put("height", 600);
        args.put("scale", Screen.getPrimary().getOutputScaleX()); // high-DPI crispness
        statusLabel.setText("Rendering…");
        ctx.jobs().run(ctx.bridge().command("render_plot", args), this::showPlot);
    }

    private void showPlot(JsonNode result) {
        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            plotView.setImage(new Image(in)); // synchronous load, then safe to delete
            statusLabel.setText(result.path("type").asText() + " — "
                    + result.path("x").asText()
                    + (result.hasNonNull("y") ? " vs " + result.path("y").asText() : ""));
        } catch (Exception e) {
            statusLabel.setText("Could not load plot image: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private static java.util.List<String> toList(JsonNode arr) {
        java.util.List<String> out = new java.util.ArrayList<>();
        arr.forEach(n -> out.add(n.asText()));
        return out;
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d);
        xCombo.setDisable(d);
        typeCombo.setDisable(d);
        refreshButton.setDisable(d);
        renderButton.setDisable(d);
    }
}
