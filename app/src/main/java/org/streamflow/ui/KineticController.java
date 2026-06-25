package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
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
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.regex.Pattern;

/**
 * Analysis module A5 — Kinetic / Time-Course. Bins FCS events by the Time
 * channel, groups samples by a user-assigned label, and plots group-mean
 * median-MFI ± SD as a line chart with shaded error bands.
 */
public class KineticController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern TIME_CHAN =
            Pattern.compile("^Time$|^TIME$|^time$", Pattern.CASE_INSENSITIVE);

    @FXML private ComboBox<String> channelCombo;
    @FXML private ComboBox<String> timeCombo;
    @FXML private Spinner<Integer> binsSpinner;
    @FXML private Button           refreshButton;
    @FXML private Button           runButton;
    @FXML private Label            statusLabel;
    @FXML private ImageView        plotView;
    @FXML private TableView<SampleRow>       sampleTable;
    @FXML private TableColumn<SampleRow, String> sampleCol;
    @FXML private TableColumn<SampleRow, String> groupCol;

    private final ObservableList<SampleRow> sampleRows = FXCollections.observableArrayList();
    private AppContext ctx;

    public static final class SampleRow {
        final String name;
        final StringProperty group = new SimpleStringProperty("");
        SampleRow(String name) { this.name = name; }
    }

    @FXML
    public void initialize() {
        binsSpinner.setValueFactory(
                new SpinnerValueFactory.IntegerSpinnerValueFactory(20, 500, 100, 10));
        plotView.setPreserveRatio(true);

        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name));
        groupCol.setCellValueFactory(c -> c.getValue().group);
        groupCol.setCellFactory(TextFieldTableCell.forTableColumn());
        groupCol.setOnEditCommit(e -> e.getRowValue().group.set(
                e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(sampleRows);
        sampleTable.setEditable(true);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshChannels();
    }

    @FXML private void onRefresh() { refreshChannels(); }

    private void refreshChannels() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleRows.clear();
            r.path("samples").forEach(n -> sampleRows.add(new SampleRow(n.asText())));

            channelCombo.getItems().clear();
            timeCombo.getItems().clear();
            String timeGuess = null;
            for (JsonNode c : r.path("channels")) {
                String ch = c.asText();
                channelCombo.getItems().add(ch);
                timeCombo.getItems().add(ch);
                if (timeGuess == null && TIME_CHAN.matcher(ch).find()) timeGuess = ch;
            }
            if (!channelCombo.getItems().isEmpty()) channelCombo.getSelectionModel().selectFirst();
            if (timeGuess != null) timeCombo.getSelectionModel().select(timeGuess);
            else if (!timeCombo.getItems().isEmpty()) timeCombo.getSelectionModel().selectFirst();
        });
    }

    @FXML
    private void onRun() {
        if (ctx == null || channelCombo.getValue() == null || timeCombo.getValue() == null) return;
        if (channelCombo.getValue().equals(timeCombo.getValue())) {
            statusLabel.setText("MFI channel and Time channel must be different.");
            return;
        }

        ObjectNode grpNode = JSON.createObjectNode();
        for (SampleRow r : sampleRows) {
            if (!r.group.get().isBlank()) grpNode.put(r.name, r.group.get().trim());
        }

        ObjectNode a = JSON.createObjectNode();
        a.put("channel", channelCombo.getValue());
        a.put("time_channel", timeCombo.getValue());
        a.put("bins", binsSpinner.getValue());
        a.set("groups", grpNode);

        statusLabel.setText("Binning by " + timeCombo.getValue() + "…");
        ctx.jobs().run(ctx.bridge().command("run_kinetic", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            plotView.setImage(new Image(in));
            int nGroups  = result.path("groups").size();
            int nSamples = result.path("n_samples").asInt();
            statusLabel.setText(String.format(
                    "%s vs %s — %d group(s) from %d sample(s), %d bins.",
                    result.path("channel").asText(),
                    result.path("time_channel").asText(),
                    nGroups, nSamples, result.path("bins").asInt()));
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, channelCombo.getValue(),
                    String.format("Kinetic: %s vs %s, %d groups, %d bins",
                            result.path("channel").asText(),
                            result.path("time_channel").asText(),
                            nGroups, result.path("bins").asInt()));
        } catch (Exception e) {
            statusLabel.setText("Could not load kinetic plot: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        channelCombo.setDisable(d);
        timeCombo.setDisable(d);
        binsSpinner.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
