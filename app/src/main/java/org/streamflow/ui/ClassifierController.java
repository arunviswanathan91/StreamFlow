package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.CheckBoxTableCell;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.image.Image;
import javafx.scene.image.ImageView;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
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
    @FXML private Button  selectAllButton;
    @FXML private Button  selectNoneButton;
    @FXML private Label   featCountLabel;
    @FXML private Label   statusLabel;
    @FXML private ImageView plotView;
    @FXML private TableView<SampleRow>           sampleTable;
    @FXML private TableColumn<SampleRow, Boolean> useCol;
    @FXML private TableColumn<SampleRow, String>  sampleCol;
    @FXML private TableColumn<SampleRow, String>  groupCol;
    @FXML private TableView<LoadRow>             loadingTable;
    @FXML private TableColumn<LoadRow, String>   featCol;
    @FXML private TableColumn<LoadRow, String>   pc1Col;
    @FXML private TableColumn<LoadRow, String>   pc2Col;

    private final ObservableList<SampleRow> sampleRows  = FXCollections.observableArrayList();
    private final ObservableList<LoadRow>   loadingRows = FXCollections.observableArrayList();
    private AppContext ctx;

    public static final class SampleRow {
        final String name;
        final SimpleBooleanProperty use = new SimpleBooleanProperty(true);
        final StringProperty group = new SimpleStringProperty("");
        SampleRow(String name) { this.name = name; }
    }

    public record LoadRow(String feat, String pc1, String pc2) {}

    @FXML
    public void initialize() {
        plotView.setPreserveRatio(true);

        useCol.setCellValueFactory(c -> c.getValue().use);
        useCol.setCellFactory(CheckBoxTableCell.forTableColumn(useCol));
        useCol.setEditable(true);
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
        Map<String, Boolean> prevUse = new LinkedHashMap<>();
        for (SampleRow r : sampleRows) { prevGroups.put(r.name, r.group.get()); prevUse.put(r.name, r.use.get()); }

        sampleRows.clear();
        for (String s : ws.sampleNames()) {
            SampleRow row = new SampleRow(s);
            if (prevGroups.containsKey(s)) row.group.set(prevGroups.get(s));
            if (prevUse.containsKey(s)) row.use.set(prevUse.get(s));
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

    @FXML private void onSelectAll()  { for (SampleRow r : sampleRows) r.use.set(true);  sampleTable.refresh(); }
    @FXML private void onSelectNone() { for (SampleRow r : sampleRows) r.use.set(false); sampleTable.refresh(); }

    @FXML
    private void onRun() {
        if (ctx == null) return;
        List<String> selected = new ArrayList<>();
        for (SampleRow r : sampleRows) if (r.use.get()) selected.add(r.name);
        if (selected.size() < 2) {
            info("Select samples", "Tick at least 2 samples to run PCA.");
            return;
        }
        // gates live in the workspace tree, independent of events — check before loading anything
        if (!anyGatesFor(selected)) {
            info("Gates not detected",
                 "The selected samples have no gated populations.\n\nDraw a gating strategy in a "
                 + "graph window and apply it to all samples (Workstation → right-click → Apply to all), "
                 + "then run the classifier.");
            return;
        }
        runButton.setDisable(true);
        statusLabel.setText("Loading events for " + selected.size() + " sample(s)…");
        // events load on demand — no need to open each sample in a graph window first
        EventLoader.ensureLoaded(ctx, selected, statusLabel::setText, () -> {
            runButton.setDisable(false);
            runClassifier(selected);
        });
    }

    /** True if any of the given samples has at least one gate in its workspace tree. */
    private boolean anyGatesFor(List<String> samples) {
        WorkspaceModel ws = ctx.workspace();
        for (String s : samples) {
            if (!ws.hasTree(s)) continue;
            for (PopNode n : ws.treeFor(s).selfAndDescendants()) if (!n.isRoot()) return true;
        }
        return false;
    }

    private void runClassifier(List<String> selected) {
        WorkspaceModel ws = ctx.workspace();
        // Build feature matrix: {sample: {gate_name: frequency}}
        ObjectNode features = JSON.createObjectNode();
        int computed = 0;
        for (String sname : selected) {
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
            if (featVec.size() > 0) { features.set(sname, featVec); computed++; }
        }

        if (computed < 2) {
            info("Gates not detected",
                 "Fewer than 2 of the selected samples have gated populations.\n\nApply your gating "
                 + "strategy to all selected samples, then run again.");
            statusLabel.setText("Need ≥2 samples with gates.");
            return;
        }

        ObjectNode grpNode = JSON.createObjectNode();
        for (SampleRow r : sampleRows)
            if (r.use.get() && !r.group.get().isBlank()) grpNode.put(r.name, r.group.get().trim());

        ObjectNode a = JSON.createObjectNode();
        a.set("features", features);
        if (grpNode.size() > 0) a.set("group_labels", grpNode);

        statusLabel.setText("Running PCA on " + computed + " samples…");
        ctx.jobs().run(ctx.bridge().command("run_classifier", a), this::showResult);
    }

    /** Simple modal info dialog with a single OK button. */
    private void info(String title, String msg) {
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle(title);
        a.setHeaderText(title);
        a.setContentText(msg);
        a.showAndWait();
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
        selectAllButton.setDisable(d);
        selectNoneButton.setDisable(d);
    }
}
