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
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Analysis module A6 — Sample Classifier (PCA + optional Random Forest).
 *
 * <p>Builds a feature vector per sample from all gated population frequencies
 * in the {@link WorkspaceModel} (gate name → % of all events), sends it to
 * the engine's {@code run_classifier}, and shows a PCA biplot. If the user
 * assigns group labels, RF feature importances are added alongside the biplot.
 *
 * <p>Only samples with cached events (opened at least once) contribute features;
 * the status line reports coverage.
 */
public class ClassifierController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Button  refreshButton;
    @FXML private Button  runButton;
    @FXML private Label   featCountLabel;
    @FXML private Label   statusLabel;
    @FXML private ImageView plotView;
    @FXML private TableView<SampleRow>          sampleTable;
    @FXML private TableColumn<SampleRow, String> sampleCol;
    @FXML private TableColumn<SampleRow, String> groupCol;
    @FXML private TableView<LoadRow>             loadingTable;
    @FXML private TableColumn<LoadRow, String>   featCol;
    @FXML private TableColumn<LoadRow, String>   pc1Col;
    @FXML private TableColumn<LoadRow, String>   pc2Col;

    private final ObservableList<SampleRow> sampleRows  = FXCollections.observableArrayList();
    private final ObservableList<LoadRow>   loadingRows = FXCollections.observableArrayList();
    private AppContext ctx;

    public static final class SampleRow {
        final String name;
        final StringProperty group = new SimpleStringProperty("");
        SampleRow(String name) { this.name = name; }
    }

    public record LoadRow(String feat, String pc1, String pc2) {}

    @FXML
    public void initialize() {
        plotView.setPreserveRatio(true);

        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name));
        groupCol.setCellValueFactory(c -> c.getValue().group);
        groupCol.setCellFactory(TextFieldTableCell.forTableColumn());
        groupCol.setOnEditCommit(e -> e.getRowValue().group.set(
                e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(sampleRows);
        sampleTable.setEditable(true);

        featCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().feat()));
        pc1Col.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pc1()));
        pc2Col.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pc2()));
        loadingTable.setItems(loadingRows);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        rebuildSamples();
    }

    @FXML private void onRefresh() { rebuildSamples(); }

    private void rebuildSamples() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();

        Map<String, String> prevGroups = new LinkedHashMap<>();
        for (SampleRow r : sampleRows) prevGroups.put(r.name, r.group.get());

        sampleRows.clear();
        for (String s : ws.sampleNames()) {
            SampleRow row = new SampleRow(s);
            if (prevGroups.containsKey(s)) row.group.set(prevGroups.get(s));
            sampleRows.add(row);
        }

        // count available features
        int feats = countFeatures(ws);
        featCountLabel.setText(feats > 0 ? feats + " feature(s) from gated populations." : "");
        statusLabel.setText(sampleRows.isEmpty()
                ? "Load FCS files and draw gates first."
                : "Ready — " + sampleRows.size() + " samples, " + feats + " features.");
    }

    private int countFeatures(WorkspaceModel ws) {
        java.util.Set<String> names = new java.util.TreeSet<>();
        for (String s : ws.samples()) {
            PopNode root = ws.treeFor(s);
            for (PopNode n : root.selfAndDescendants())
                if (!n.isRoot()) names.add(n.name());
        }
        return names.size();
    }

    @FXML
    private void onRun() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();

        // Build feature matrix: {sample: {gate_name: frequency}}
        ObjectNode features = JSON.createObjectNode();
        int computed = 0;
        for (String sname : ws.sampleNames()) {
            EventData evData = ws.data(sname);
            if (evData == null || evData.rows() == 0) continue;
            PopNode root = ws.treeFor(sname);
            ObjectNode featVec = JSON.createObjectNode();
            for (PopNode n : root.selfAndDescendants()) {
                if (n.isRoot()) continue;
                boolean[] keep = new boolean[evData.rows()];
                Arrays.fill(keep, true);
                for (CytoPlot.Gate g : n.chain()) {
                    boolean[] m = CytoPlot.mask(evData, g);
                    for (int k = 0; k < keep.length; k++) keep[k] = keep[k] && m[k];
                }
                int cnt = 0; for (boolean b : keep) if (b) cnt++;
                featVec.put(n.name(), 100.0 * cnt / evData.rows());
            }
            if (featVec.size() > 0) {
                features.set(sname, featVec);
                computed++;
            }
        }

        if (computed < 2) {
            statusLabel.setText("Need at least 2 samples with gated events. "
                    + "Open samples in graph windows to cache their events.");
            return;
        }

        ObjectNode grpNode = JSON.createObjectNode();
        for (SampleRow r : sampleRows)
            if (!r.group.get().isBlank()) grpNode.put(r.name, r.group.get().trim());

        ObjectNode a = JSON.createObjectNode();
        a.set("features", features);
        if (grpNode.size() > 0) a.set("group_labels", grpNode);

        statusLabel.setText("Running PCA on " + computed + " samples…");
        ctx.jobs().run(ctx.bridge().command("run_classifier", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        loadingRows.clear();
        for (JsonNode l : result.path("loadings")) {
            loadingRows.add(new LoadRow(
                    l.path("feat").asText(),
                    String.format("%.3f", l.path("pc1").asDouble()),
                    String.format("%.3f", l.path("pc2").asDouble())));
        }

        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            plotView.setImage(new Image(in));
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, "",
                    String.format("Classifier (%s): %d samples × %d features",
                            result.path("method").asText(),
                            result.path("n_samples").asInt(),
                            result.path("n_features").asInt()));
            statusLabel.setText(String.format(
                    "%s — %d samples × %d features. %s",
                    result.path("method").asText(),
                    result.path("n_samples").asInt(),
                    result.path("n_features").asInt(),
                    result.path("var_explained").size() > 0
                        ? String.format("PC1+PC2 = %.1f%%",
                            (result.path("var_explained").get(0).asDouble()
                            + (result.path("var_explained").size() > 1
                               ? result.path("var_explained").get(1).asDouble() : 0)) * 100)
                        : ""));
        } catch (Exception e) {
            statusLabel.setText("Could not load biplot: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
