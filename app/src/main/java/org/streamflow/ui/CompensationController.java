package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;
import javafx.util.converter.DoubleStringConverter;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

/**
 * Phase 2 — Compensation. Extract the embedded spillover matrix or compute one
 * from single-colour controls, view and edit it, then apply it.
 * The matrix table is fully editable: double-click any coefficient cell to change it
 * before clicking Apply.
 */
public class CompensationController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Button extractButton;
    @FXML private Button computeButton;
    @FXML private Button applyButton;
    @FXML private Button residualButton;
    @FXML private Label statusLabel;
    @FXML private TableView<Row> matrixTable;
    @FXML private ImageView residualView;
    @FXML private TableView<Flagged> flaggedTable;
    @FXML private TableColumn<Flagged, String> pairColR;
    @FXML private TableColumn<Flagged, String> rColR;
    @FXML private TableColumn<Flagged, String> signColR;

    private final ObservableList<Row> rows = FXCollections.observableArrayList();
    private final ObservableList<Flagged> flaggedRows = FXCollections.observableArrayList();
    private final List<String> currentChannels = new ArrayList<>();
    private AppContext ctx;

    /** One flagged residual-correlation pair. */
    public record Flagged(String pair, String r, String sign) {}

    /** One spillover-matrix row: a channel name + its spillover coefficients. */
    public static final class Row {
        final String channel;
        final List<Double> values;
        Row(String channel, List<Double> values) { this.channel = channel; this.values = values; }
    }

    @FXML
    public void initialize() {
        matrixTable.setItems(rows);
        matrixTable.setEditable(true);
        pairColR.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pair()));
        rColR.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().r()));
        signColR.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sign()));
        flaggedTable.setItems(flaggedRows);
        residualView.setPreserveRatio(true);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
    }

    @FXML
    private void onExtract() {
        if (ctx == null) return;
        statusLabel.setText("Extracting embedded spillover…");
        ctx.jobs().run(ctx.bridge().command("extract_spillover", null), this::showMatrix);
    }

    @FXML
    private void onCompute() {
        if (ctx == null) return;
        statusLabel.setText("Computing spillover from controls…");
        ctx.jobs().run(ctx.bridge().command("compute_spillover", null), this::showMatrix);
    }

    @FXML
    private void onApply() {
        if (ctx == null) return;
        statusLabel.setText("Applying compensation…");

        // Send the current (possibly edited) matrix values back to the engine so edits
        // made in the table are respected, rather than re-using whatever the engine stored.
        ObjectNode args = null;
        if (!rows.isEmpty() && !currentChannels.isEmpty()) {
            args = JSON.createObjectNode();
            ArrayNode chArr = args.putArray("channels");
            currentChannels.forEach(chArr::add);
            ArrayNode matArr = args.putArray("matrix");
            for (Row row : rows) {
                ArrayNode rowArr = matArr.addArray();
                row.values.forEach(rowArr::add);
            }
        }

        final ObjectNode finalArgs = args;
        ctx.jobs().run(ctx.bridge().command("apply_compensation", finalArgs),
                r -> statusLabel.setText("Compensation applied to "
                        + r.path("channels").size() + " channels."));
    }

    private void showMatrix(JsonNode result) {
        List<String> channels = new ArrayList<>();
        result.path("channels").forEach(n -> channels.add(n.asText()));

        currentChannels.clear();
        currentChannels.addAll(channels);

        List<TableColumn<Row, ?>> cols = new ArrayList<>();

        TableColumn<Row, String> nameCol = new TableColumn<>("Channel");
        nameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().channel));
        nameCol.setPrefWidth(150);
        nameCol.setEditable(false);
        cols.add(nameCol);

        for (int j = 0; j < channels.size(); j++) {
            final int idx = j;
            TableColumn<Row, Double> col = new TableColumn<>(channels.get(j));
            col.setPrefWidth(90);
            col.setEditable(true);
            col.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().values.get(idx)));
            col.setCellFactory(TextFieldTableCell.forTableColumn(new DoubleStringConverter()));
            col.setOnEditCommit(e -> {
                if (e.getNewValue() != null) {
                    e.getRowValue().values.set(idx, e.getNewValue());
                }
            });
            cols.add(col);
        }
        matrixTable.getColumns().setAll(cols);

        rows.clear();
        JsonNode mat = result.path("matrix");
        for (int i = 0; i < mat.size(); i++) {
            List<Double> vals = new ArrayList<>();
            mat.get(i).forEach(v -> vals.add(Math.round(v.asDouble() * 1000.0) / 1000.0));
            rows.add(new Row(channels.get(i), vals));
        }
        statusLabel.setText(channels.size() + "×" + channels.size()
                + " spillover matrix — double-click a cell to edit, then click Apply.");
    }

    @FXML
    private void onResidual() {
        if (ctx == null) return;
        statusLabel.setText("Computing residual correlation…");
        ctx.jobs().run(ctx.bridge().command("comp_residual", null), this::showResidual);
    }

    private void showResidual(JsonNode result) {
        flaggedRows.clear();
        for (JsonNode f : result.path("flagged")) {
            flaggedRows.add(new Flagged(
                    f.path("a").asText() + " ↔ " + f.path("b").asText(),
                    String.format("%.3f", f.path("r").asDouble()),
                    f.path("sign").asText().equals("over") ? "over-comp" : "under-comp"));
        }
        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            residualView.setImage(new Image(in));
            int nf = result.path("flagged").size();
            statusLabel.setText(nf == 0
                    ? "Residual diagnostic on " + result.path("sample").asText() + ": no |r|>0.2 pairs — compensation looks clean."
                    : nf + " channel pair(s) with |r|>0.2 on " + result.path("sample").asText() + " — review compensation.");
            if (ctx != null) ctx.auditLog().add(AuditLog.Type.COMPENSATION,
                    result.path("sample").asText(),
                    "Residual diagnostic: " + nf + " flagged pair(s)");
        } catch (Exception e) {
            statusLabel.setText("Could not load residual heatmap: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        extractButton.setDisable(d);
        computeButton.setDisable(d);
        applyButton.setDisable(d);
        residualButton.setDisable(d);
    }
}
