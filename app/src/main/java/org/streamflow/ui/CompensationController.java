package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.concurrent.Task;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.util.converter.DoubleStringConverter;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;

/**
 * Phase 2 — Compensation, FlowJo-style. Extract the embedded spillover matrix (or compute one),
 * edit coefficients in either an interactive {@link CompMatrixView} heatmap or the matrix table
 * (edits sync both ways), preview the before/after effect on any channel pair with two native
 * {@link CytoPlot}s, then Apply. The residual diagnostic flags over/under-compensated pairs both
 * in the table and as outlines on the heatmap.
 */
public class CompensationController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Button extractButton, computeButton, applyButton, residualButton, previewButton;
    @FXML private Label statusLabel;
    @FXML private TableView<Row> matrixTable;
    @FXML private CompMatrixView heatmap;
    @FXML private ComboBox<String> sampleCombo, xCombo, yCombo;
    @FXML private CytoPlot rawPlot, compPlot;
    @FXML private TableView<Flagged> flaggedTable;
    @FXML private TableColumn<Flagged, String> pairColR, rColR, signColR;

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

        // Heatmap edit → keep the table model in sync (single source of truth = rows)
        heatmap.setOnCellEdit((r, c, v) -> {
            if (r < rows.size() && c < rows.get(r).values.size()) {
                rows.get(r).values.set(c, v);
                matrixTable.refresh();
            }
        });
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        rawPlot.setChannelLabeler(ch -> ctx.aliases().label(ch));
        compPlot.setChannelLabeler(ch -> ctx.aliases().label(ch));
        setDisabled(false);
        refreshSamples();
    }

    private void refreshSamples() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleCombo.getItems().clear();
            r.path("samples").forEach(n -> sampleCombo.getItems().add(n.asText()));
            if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();
        });
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
        // FlowJo-style wizard: assign controls, gate, review separation, build the matrix.
        CompWizardController.open(ctx, result -> {
            showMatrix(result);
            statusLabel.setText("Spillover computed from controls — review the heatmap, Preview a pair, then Apply.");
        });
    }

    @FXML
    private void onApply() {
        if (ctx == null) return;
        statusLabel.setText("Applying compensation…");
        ObjectNode args = matrixArgs();
        ctx.jobs().run(ctx.bridge().command("apply_compensation", args),
                r -> statusLabel.setText("Compensation applied to "
                        + r.path("channels").size() + " channels. Use Preview to verify, then run the residual diagnostic."));
    }

    /** Build {channels, matrix} from the current (possibly edited) table model, or null if empty. */
    private ObjectNode matrixArgs() {
        if (rows.isEmpty() || currentChannels.isEmpty()) return null;
        ObjectNode args = JSON.createObjectNode();
        ArrayNode chArr = args.putArray("channels");
        currentChannels.forEach(chArr::add);
        ArrayNode matArr = args.putArray("matrix");
        for (Row row : rows) {
            ArrayNode rowArr = matArr.addArray();
            row.values.forEach(rowArr::add);
        }
        return args;
    }

    private void showMatrix(JsonNode result) {
        List<String> channels = new ArrayList<>();
        result.path("channels").forEach(n -> channels.add(n.asText()));
        currentChannels.clear();
        currentChannels.addAll(channels);

        // ---- table columns ----
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
                    int rowIdx = e.getTablePosition().getRow();
                    e.getRowValue().values.set(idx, e.getNewValue());
                    heatmap.setValue(rowIdx, idx, e.getNewValue());   // keep heatmap in sync
                }
            });
            cols.add(col);
        }
        matrixTable.getColumns().setAll(cols);

        // ---- rows + heatmap backing ----
        rows.clear();
        double[][] m = new double[channels.size()][channels.size()];
        JsonNode mat = result.path("matrix");
        for (int i = 0; i < mat.size(); i++) {
            List<Double> vals = new ArrayList<>();
            JsonNode rowNode = mat.get(i);
            for (int j = 0; j < rowNode.size(); j++) {
                double v = Math.round(rowNode.get(j).asDouble() * 1000.0) / 1000.0;
                vals.add(v);
                if (i < m.length && j < m[i].length) m[i][j] = v;
            }
            rows.add(new Row(channels.get(i), vals));
        }
        heatmap.setMatrix(channels, m);

        // ---- preview channel pickers ----
        xCombo.getItems().setAll(channels);
        yCombo.getItems().setAll(channels);
        if (channels.size() >= 1) xCombo.getSelectionModel().select(0);
        if (channels.size() >= 2) yCombo.getSelectionModel().select(1);
        refreshSamples();   // data is loaded by now → populate the before/after sample picker

        statusLabel.setText(channels.size() + "×" + channels.size()
                + " spillover matrix — edit in the heatmap or table, Preview a pair, then Apply.");
    }

    @FXML
    private void onPreview() {
        if (ctx == null) return;
        if (rows.isEmpty()) { statusLabel.setText("Extract or Compute a matrix first."); return; }
        String xch = xCombo.getValue(), ych = yCombo.getValue();
        if (xch == null || ych == null || xch.equals(ych)) {
            statusLabel.setText("Pick two different channels to preview."); return;
        }
        ObjectNode args = matrixArgs();
        if (args == null) { statusLabel.setText("No matrix to preview."); return; }
        if (sampleCombo.getValue() != null) args.put("sample", sampleCombo.getValue());
        args.put("x", xch);
        args.put("y", ych);
        statusLabel.setText("Rendering before/after for " + xch + " × " + ych + "…");
        Task<JsonNode> task = ctx.bridge().command("comp_preview", args);
        ctx.jobs().run(task, this::showPreview);
    }

    private void showPreview(JsonNode r) {
        String xch = r.path("x").asText(), ych = r.path("y").asText();
        int rowsN = r.path("rows").asInt(), cols = r.path("cols").asInt();
        Path rawBin = Paths.get(r.path("raw_file").asText());
        Path compBin = Paths.get(r.path("comp_file").asText());
        try {
            EventData rawData = EventData.read(rawBin, List.of(xch, ych), rowsN, cols);
            EventData compData = EventData.read(compBin, List.of(xch, ych), rowsN, cols);
            rawPlot.setData(rawData);
            rawPlot.setView(xch, ych, CytoPlot.Scale.LOGICLE, CytoPlot.Scale.LOGICLE, "pseudocolor");
            compPlot.setData(compData);
            compPlot.setView(xch, ych, CytoPlot.Scale.LOGICLE, CytoPlot.Scale.LOGICLE, "pseudocolor");
            statusLabel.setText("Before/after: " + xch + " × " + ych + " — "
                    + rowsN + " events. A tight diagonal in the 'After' plot means good compensation.");
        } catch (Exception ex) {
            statusLabel.setText("Could not render preview: " + ex.getMessage());
        } finally {
            try { Files.deleteIfExists(rawBin); } catch (Exception ignored) {}
            try { Files.deleteIfExists(compBin); } catch (Exception ignored) {}
        }
    }

    @FXML
    private void onResidual() {
        if (ctx == null) return;
        statusLabel.setText("Computing residual correlation…");
        ObjectNode args = JSON.createObjectNode();
        if (sampleCombo.getValue() != null) args.put("sample", sampleCombo.getValue());
        ctx.jobs().run(ctx.bridge().command("comp_residual", args), this::showResidual);
    }

    private void showResidual(JsonNode result) {
        flaggedRows.clear();
        List<String[]> flaggedPairs = new ArrayList<>();
        for (JsonNode f : result.path("flagged")) {
            String a = f.path("a").asText(), b = f.path("b").asText();
            flaggedRows.add(new Flagged(
                    a + " ↔ " + b,
                    String.format("%.3f", f.path("r").asDouble()),
                    f.path("sign").asText().equals("over") ? "over-comp" : "under-comp"));
            flaggedPairs.add(new String[]{a, b});
        }
        heatmap.setFlagged(flaggedPairs);   // outline the offending coefficients on the heatmap

        // the engine still renders a heatmap PNG; we visualise via the native heatmap, so discard it
        if (result.hasNonNull("png")) {
            try { Files.deleteIfExists(Paths.get(result.path("png").asText())); } catch (Exception ignored) {}
        }
        int nf = result.path("flagged").size();
        statusLabel.setText(nf == 0
                ? "Residual diagnostic on " + result.path("sample").asText() + ": no |r|>0.2 pairs — compensation looks clean."
                : nf + " channel pair(s) with |r|>0.2 on " + result.path("sample").asText()
                    + " — flagged on the heatmap; adjust those coefficients.");
        if (ctx != null) ctx.auditLog().add(AuditLog.Type.COMPENSATION,
                result.path("sample").asText(), "Residual diagnostic: " + nf + " flagged pair(s)");
    }

    private void setDisabled(boolean d) {
        extractButton.setDisable(d);
        computeButton.setDisable(d);
        applyButton.setDisable(d);
        residualButton.setDisable(d);
        previewButton.setDisable(d);
    }
}
