package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.regex.Pattern;

/**
 * Analysis module A2 — Proliferation Index. Fits a dye-dilution histogram
 * (engine command {@code run_proliferation}) with equal-σ, log2-spaced Gaussian
 * generations and reports the precursor-corrected Proliferation Index (PI),
 * Division Index (DI) and % divided, plus a per-generation breakdown.
 */
public class ProliferationController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern DYE = Pattern.compile("CFSE|CTV|CellTrace|Ki.?67|BrdU|VPD|Prolif",
            Pattern.CASE_INSENSITIVE);

    @FXML private ComboBox<String> sampleCombo;
    @FXML private ComboBox<String> channelCombo;
    @FXML private Spinner<Integer> peaksSpinner;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Label statusLabel;
    @FXML private ImageView plotView;
    @FXML private Label piLabel, diLabel, divLabel;
    @FXML private TableView<GenRow> genTable;
    @FXML private TableColumn<GenRow, Number> genCol;
    @FXML private TableColumn<GenRow, Number> countCol;
    @FXML private TableColumn<GenRow, Number> pctCol;

    private final ObservableList<GenRow> genRows = FXCollections.observableArrayList();
    private AppContext ctx;

    /** One generation row: generation index, fitted cell count, % of total. */
    public record GenRow(int gen, int count, double pct) {}

    @FXML
    public void initialize() {
        peaksSpinner.setValueFactory(
                new SpinnerValueFactory.IntegerSpinnerValueFactory(2, 12, 8, 1));
        plotView.setPreserveRatio(true);
        genCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().gen()));
        countCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().count()));
        pctCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().pct()));
        genTable.setItems(genRows);
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

        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            plotView.setImage(new Image(in));
            statusLabel.setText(genRows.size() + " generation(s) fitted on "
                    + result.path("channel").asText() + " — n=" + result.path("n").asLong() + ".");
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                    String.format("Proliferation on %s: PI=%.3f DI=%.3f div=%.1f%%",
                            result.path("channel").asText(),
                            result.path("PI").asDouble(),
                            result.path("DI").asDouble(),
                            result.path("pct_divided").asDouble()));
        } catch (Exception e) {
            statusLabel.setText("Could not load fit plot: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d);
        channelCombo.setDisable(d);
        peaksSpinner.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
