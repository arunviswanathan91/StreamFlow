package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * Phase 4 — Clustering (FlowSOM / PhenoGraph). Long async job; shows the
 * cluster × channel median-expression heatmap and per-cluster sizes.
 */
public class ClusteringController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private ComboBox<String> methodCombo;
    @FXML private ComboBox<String> sampleCombo;
    @FXML private Spinner<Integer> kSpinner;
    @FXML private Spinner<Integer> eventsSpinner;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Label statusLabel;
    @FXML private ImageView heatmapView;
    @FXML private ListView<String> clusterList;

    private AppContext ctx;

    @FXML
    public void initialize() {
        methodCombo.setItems(FXCollections.observableArrayList("flowsom", "phenograph"));
        methodCombo.getSelectionModel().select("flowsom");
        kSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(2, 50, 10, 1));
        eventsSpinner.setValueFactory(
                new SpinnerValueFactory.IntegerSpinnerValueFactory(1000, 500000, 20000, 1000));
        // k only applies to FlowSOM (PhenoGraph finds its own cluster count)
        kSpinner.disableProperty().bind(
                methodCombo.getSelectionModel().selectedItemProperty().isEqualTo("phenograph"));
        heatmapView.setPreserveRatio(true);
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
        a.put("k", kSpinner.getValue());
        a.put("n_events", eventsSpinner.getValue());
        statusLabel.setText("Clustering with " + methodCombo.getValue() + "…");
        ctx.jobs().run(ctx.bridge().command("run_clustering", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            heatmapView.setImage(new Image(in));
        } catch (Exception e) {
            statusLabel.setText("Could not load heatmap: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
        clusterList.getItems().clear();
        for (JsonNode c : result.path("clusters")) {
            clusterList.getItems().add(String.format("C%d — %d events (%.1f%%)",
                    c.path("cluster").asInt(), c.path("count").asInt(), c.path("percent").asDouble()));
        }
        statusLabel.setText(result.path("method").asText() + " — " + result.path("k").asInt()
                + " clusters over " + result.path("n").asInt() + " events.");
    }

    private void setDisabled(boolean d) {
        methodCombo.setDisable(d);
        sampleCombo.setDisable(d);
        eventsSpinner.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
