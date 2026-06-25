package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * Phase 4 — Dimensionality reduction (UMAP / t-SNE). A long async job that keeps
 * the UI responsive (the operation that froze the old Shiny app).
 */
public class DimReductionController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private ComboBox<String> methodCombo;
    @FXML private ComboBox<String> sampleCombo;
    @FXML private Spinner<Integer> eventsSpinner;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Label statusLabel;
    @FXML private ImageView mapView;

    private AppContext ctx;

    @FXML
    public void initialize() {
        methodCombo.setItems(FXCollections.observableArrayList("umap", "tsne"));
        methodCombo.getSelectionModel().select("umap");
        eventsSpinner.setValueFactory(
                new SpinnerValueFactory.IntegerSpinnerValueFactory(500, 200000, 5000, 500));
        mapView.setPreserveRatio(true);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshSamples();
    }

    @FXML
    private void onRefresh() { refreshSamples(); }

    private void refreshSamples() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().clear();
            r.path("samples").forEach(n -> sampleCombo.getItems().add(n.asText()));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();
        });
    }

    @FXML
    private void onRun() {
        if (ctx == null || sampleCombo.getValue() == null) return;
        ObjectNode a = JSON.createObjectNode();
        a.put("method", methodCombo.getValue());
        a.put("sample", sampleCombo.getValue());
        a.put("n_events", eventsSpinner.getValue());
        statusLabel.setText("Running " + methodCombo.getValue().toUpperCase()
                + " (this can take a while — the UI stays responsive)…");
        ctx.jobs().run(ctx.bridge().command("run_dimredux", a), this::showMap);
    }

    private void showMap(JsonNode result) {
        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            mapView.setImage(new Image(in));
            statusLabel.setText(result.path("method").asText().toUpperCase()
                    + " — " + result.path("n").asInt() + " events.");
        } catch (Exception e) {
            statusLabel.setText("Could not load embedding: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        methodCombo.setDisable(d);
        sampleCombo.setDisable(d);
        eventsSpinner.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
