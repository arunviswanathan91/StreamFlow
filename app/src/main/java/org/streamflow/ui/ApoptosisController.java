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
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.List;
import java.util.regex.Pattern;

/**
 * Analysis module A3 — Apoptosis. Runs {@code run_apoptosis} on the engine:
 * auto-detects Annexin V and PI channel thresholds at density valleys, then
 * counts the four quadrant populations (Live / Early Apoptotic / Late Apoptotic
 * / Necrotic) and displays a coloured scatter plot.
 */
public class ApoptosisController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern ANNEXIN =
            Pattern.compile("Annexin|AnnV|Ann.?V|FITC.*Ann|Ann.*FITC",
                    Pattern.CASE_INSENSITIVE);
    private static final Pattern PI_CHAN =
            Pattern.compile("PI|Propidium|7.?AAD|7AAD|PerCP|EtBr",
                    Pattern.CASE_INSENSITIVE);

    @FXML private ComboBox<String> sampleCombo;
    @FXML private ComboBox<String> annexinCombo;
    @FXML private ComboBox<String> piCombo;
    @FXML private Button           refreshButton;
    @FXML private Button           runButton;
    @FXML private Label            statusLabel;
    @FXML private ImageView        plotView;
    @FXML private TableView<QuadRow>        quadTable;
    @FXML private TableColumn<QuadRow, String>  quadNameCol;
    @FXML private TableColumn<QuadRow, Number>  quadPctCol;
    @FXML private TableColumn<QuadRow, Number>  quadNCol;
    @FXML private Label            annexinThreshLabel;
    @FXML private Label            piThreshLabel;

    private final ObservableList<QuadRow> quadRows = FXCollections.observableArrayList();
    private AppContext ctx;

    /** One quadrant row: display name, percentage, and cell count. */
    public record QuadRow(String name, double pct, int count) {}

    @FXML
    public void initialize() {
        quadNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name()));
        quadPctCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(
                Math.round(c.getValue().pct() * 10.0) / 10.0));
        quadNCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().count()));
        quadTable.setItems(quadRows);
        plotView.setPreserveRatio(true);
        // #29 — show the marker/alias the user assigned (e.g. "FL1-A (Annexin)"), keeping the
        // channel name as the actual value the engine receives.
        setAliasDisplay(annexinCombo);
        setAliasDisplay(piCombo);
        setDisabled(true);
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
        setDisabled(false);
        refreshChannels();
    }

    @FXML
    private void onRefresh() { refreshChannels(); }

    private void refreshChannels() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().clear();
            r.path("samples").forEach(n -> sampleCombo.getItems().add(n.asText()));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();

            List<String> channels = FXCollections.observableArrayList();
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
        });
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
        statusLabel.setText("Detecting thresholds and counting quadrants…");
        ctx.jobs().run(ctx.bridge().command("run_apoptosis", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        JsonNode q = result.path("quadrants");
        int n = result.path("n").asInt();

        quadRows.clear();
        quadRows.addAll(
            new QuadRow("Live",              q.path("live").asDouble(),
                        (int)(q.path("live").asDouble() / 100.0 * n)),
            new QuadRow("Early Apoptotic",   q.path("early_apoptotic").asDouble(),
                        (int)(q.path("early_apoptotic").asDouble() / 100.0 * n)),
            new QuadRow("Late Apoptotic",    q.path("late_apoptotic").asDouble(),
                        (int)(q.path("late_apoptotic").asDouble() / 100.0 * n)),
            new QuadRow("Necrotic",          q.path("necrotic").asDouble(),
                        (int)(q.path("necrotic").asDouble() / 100.0 * n))
        );

        JsonNode thr = result.path("thresholds");
        annexinThreshLabel.setText(String.format("Annexin V threshold: %.2f", thr.path("annexin").asDouble()));
        piThreshLabel.setText(String.format("PI threshold: %.2f", thr.path("pi").asDouble()));

        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            plotView.setImage(new Image(in));
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                    String.format("Apoptosis on %s/%s: Live=%.1f%% Early=%.1f%% Late=%.1f%% Necrotic=%.1f%%",
                            result.path("annexin_channel").asText(), result.path("pi_channel").asText(),
                            q.path("live").asDouble(), q.path("early_apoptotic").asDouble(),
                            q.path("late_apoptotic").asDouble(), q.path("necrotic").asDouble()));
            statusLabel.setText(String.format(
                    "%s on %s / %s  —  n=%,d  |  Live %.1f%%  Early %.1f%%  Late %.1f%%  Necrotic %.1f%%",
                    sampleCombo.getValue(),
                    result.path("annexin_channel").asText(),
                    result.path("pi_channel").asText(),
                    n,
                    q.path("live").asDouble(),
                    q.path("early_apoptotic").asDouble(),
                    q.path("late_apoptotic").asDouble(),
                    q.path("necrotic").asDouble()));
        } catch (Exception e) {
            statusLabel.setText("Could not load scatter plot: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d);
        annexinCombo.setDisable(d);
        piCombo.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
