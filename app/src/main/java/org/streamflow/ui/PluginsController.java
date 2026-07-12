package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.concurrent.Task;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.TextArea;
import javafx.scene.control.TextField;
import javafx.scene.layout.HBox;
import javafx.scene.layout.VBox;
import org.streamflow.plugins.PluginManifest;
import org.streamflow.plugins.PluginRegistry;
import org.streamflow.plugins.PluginRunner;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Runs the bundled R plugins (see {@code engine/plugins/}). Plugins are only usable when an R runtime
 * is present — the default installer is Python-only — so the module degrades gracefully and explains
 * why rather than silently offering buttons that fail.
 *
 * A plugin that returns {@code kept_indices} (e.g. PeacoQC) can be added as a cleaned sub-population,
 * reusing the same index-defined population machinery as Subsample ({@code CytoPlot.isIndexGate}).
 */
public class PluginsController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Label rStatusLabel, statusLabel, pluginTitleLabel, pluginMetaLabel;
    @FXML private ComboBox<String> sampleCombo;
    @FXML private ListView<PluginManifest> pluginList;
    @FXML private VBox paramBox;
    @FXML private TextArea outputArea;
    @FXML private Button runButton, cancelButton, addPopButton, refreshButton, batchButton;
    @FXML private javafx.scene.control.CheckBox showExperimentalCheck;

    private AppContext ctx;
    private final Map<String, TextField> paramFields = new LinkedHashMap<>();
    private Task<JsonNode> current;
    private int[] lastKeptIndices;
    private String lastPluginName;

    @FXML
    public void initialize() {
        pluginList.setCellFactory(v -> new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(PluginManifest m, boolean empty) {
                super.updateItem(m, empty);
                if (empty || m == null) { setText(null); return; }
                setText(m.name() + "  (" + m.version() + (m.status().isBlank() ? "" : ", " + m.status()) + ")");
            }
        });
        pluginList.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> showPlugin(b));
        showExperimentalCheck.selectedProperty().addListener((o, a, b) -> reloadPlugins());
        reloadPlugins();
    }

    @Override public void init(AppContext context) {
        this.ctx = context;
        ctx.workspace().sampleNames().addListener(
                (javafx.collections.ListChangeListener<String>) c -> refreshFromWorkspace());
        refreshFromWorkspace();
    }

    @Override public void refreshFromWorkspace() {
        if (ctx == null) return;
        String keep = sampleCombo.getValue();
        sampleCombo.setItems(FXCollections.observableArrayList(ctx.workspace().sampleNames()));
        if (keep != null && sampleCombo.getItems().contains(keep)) sampleCombo.getSelectionModel().select(keep);
        else if (!sampleCombo.getItems().isEmpty()) sampleCombo.getSelectionModel().selectFirst();
    }

    @FXML private void onRefresh() { reloadPlugins(); }

    private void reloadPlugins() {
        List<PluginManifest> all = PluginRegistry.discover();
        // Experimental plugins need extra setup the user can't guess at (FlowMagic wants a trained
        // .rds template), so hide them unless explicitly asked for.
        boolean showExp = showExperimentalCheck != null && showExperimentalCheck.isSelected();
        List<PluginManifest> found = showExp ? all
                : all.stream().filter(p -> !"experimental".equalsIgnoreCase(p.status())).toList();
        int hidden = all.size() - found.size();
        pluginList.setItems(FXCollections.observableArrayList(found));
        boolean hasR = PluginRegistry.rAvailable();
        if (!hasR) {
            rStatusLabel.setText("⚠ No R runtime found. Plugins are R-based and ship only in an R-enabled "
                    + "build (release with bundle_r=true), or install R locally. Discovery still works.");
            rStatusLabel.setStyle("-fx-text-fill:#F5A623;");
        } else if (found.isEmpty()) {
            rStatusLabel.setText("R found, but no plugins were discovered under engine/plugins.");
            rStatusLabel.setStyle("-fx-text-fill:#F5A623;");
        } else {
            rStatusLabel.setText("R runtime detected — " + found.size() + " plugin(s) available."
                    + (hidden > 0 ? "  (" + hidden + " experimental hidden)" : ""));
            rStatusLabel.setStyle("-fx-text-fill:#2ECC71;");
        }
        runButton.setDisable(!hasR);
        batchButton.setDisable(!hasR);
        if (!found.isEmpty()) pluginList.getSelectionModel().selectFirst();
    }

    private void showPlugin(PluginManifest m) {
        paramBox.getChildren().clear();
        paramFields.clear();
        addPopButton.setDisable(true);
        lastKeptIndices = null;
        if (m == null) { pluginTitleLabel.setText("Select a plugin"); pluginMetaLabel.setText(""); return; }

        pluginTitleLabel.setText(m.name());
        String meta = m.description()
                + (m.rPackages().isEmpty() ? "" : "\nRequires R packages: " + String.join(", ", m.rPackages()));
        if ("experimental".equalsIgnoreCase(m.status())) {
            meta = "⚠ Experimental — needs extra setup before it will run (see the required parameters below).\n" + meta;
        }
        pluginMetaLabel.setText(meta);

        for (PluginManifest.Param p : m.params()) {
            TextField tf = new TextField(p.defaultValue() == null ? "" : String.valueOf(p.defaultValue()));
            tf.setPrefWidth(160);
            if (p.help() != null && !p.help().isBlank()) tf.setTooltip(new javafx.scene.control.Tooltip(p.help()));
            Label lbl = new Label(p.label() + (p.required() ? " *" : "") + ":");
            lbl.setMinWidth(170);
            paramBox.getChildren().add(new HBox(8, lbl, tf));
            paramFields.put(p.id(), tf);
        }
    }

    @FXML
    private void onRun() {
        PluginManifest m = pluginList.getSelectionModel().getSelectedItem();
        String sample = sampleCombo.getValue();
        if (m == null) { statusLabel.setText("Select a plugin."); return; }
        if (sample == null) { statusLabel.setText("Select a sample."); return; }
        if (ctx == null) return;

        // Only the engine knows the sample's absolute FCS path; R plugins read the FCS themselves.
        outputArea.clear();
        statusLabel.setText("Resolving " + sample + "…");
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        ctx.jobs().run(ctx.bridge().command("sample_path", args), r -> launch(m, r.path("path").asText()));
    }

    private void launch(PluginManifest m, String fcsPath) {
        Map<String, Object> request = buildRequest(m, fcsPath);
        if (request == null) return;

        Task<JsonNode> task = PluginRunner.run(m, request);
        current = task;
        lastPluginName = m.name();
        runButton.setDisable(true); cancelButton.setDisable(false); addPopButton.setDisable(true);
        statusLabel.setText("Running " + m.name() + "…");
        task.messageProperty().addListener((o, a, b) -> {
            if (b != null && !b.isBlank()) outputArea.appendText(b + "\n");
        });
        task.setOnSucceeded(e -> {
            JsonNode resp = task.getValue();
            runButton.setDisable(false); cancelButton.setDisable(true); current = null;
            lastKeptIndices = PluginRunner.indices(resp, "kept_indices");
            // Record the run so it appears in the Analysis Log and the generated Methods paragraph.
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(), auditDetail(m, resp));
            outputArea.appendText("\n--- result ---\n" + resp.toPrettyString().lines()
                    .filter(l -> !l.contains("kept_indices"))
                    .reduce("", (x, y) -> x + y + "\n"));
            if (lastKeptIndices.length > 0) {
                addPopButton.setDisable(false);
                statusLabel.setText(String.format("%s: kept %,d events (%.2f%% removed). "
                                + "Click \"Add as population\".",
                        m.name(), lastKeptIndices.length, resp.path("pct_removed").asDouble()));
            } else {
                statusLabel.setText(m.name() + " finished.");
            }
        });
        task.setOnFailed(e -> {
            runButton.setDisable(false); cancelButton.setDisable(true); current = null;
            Throwable ex = task.getException();
            statusLabel.setText("Plugin failed: " + (ex == null ? "?" : ex.getMessage()));
            if (ex != null) outputArea.appendText("\nERROR: " + ex.getMessage() + "\n");
        });
        Thread t = new Thread(task, "plugin-" + m.id());
        t.setDaemon(true);
        t.start();
    }

    @FXML
    private void onCancel() {
        batchCancelled = true;          // stop the queue after the in-flight sample unwinds
        batchQueue.clear();
        if (current != null) { current.cancel(); statusLabel.setText("Cancelling…"); }
    }

    /** Add the plugin's kept events as an index-defined sub-population (same machinery as Subsample). */
    @FXML
    private void onAddPopulation() {
        String sample = sampleCombo.getValue();
        if (ctx == null || sample == null || lastKeptIndices == null || lastKeptIndices.length == 0) return;
        PopNode node = addPopulationFor(sample, lastKeptIndices, lastPluginName);
        addPopButton.setDisable(true);
        statusLabel.setText("Added '" + node.name() + "' to " + sample + " ("
                + String.format("%,d", lastKeptIndices.length) + " events).");
    }

    /** Create the index-defined population for one sample. Shared by the single run and the batch. */
    private PopNode addPopulationFor(String sample, int[] kept, String pluginName) {
        CytoPlot.Gate g = new CytoPlot.Gate(
                (pluginName == null ? "Plugin" : pluginName.split("\\s+")[0]) + " clean",
                "plugin", null, null, null, null);
        g.subSelected = kept;
        g.subBySample.put(sample, kept);

        PopNode root = ctx.workspace().treeFor(sample);
        PopNode node = new PopNode(g, root);
        node.count = kept.length;
        int total = ctx.workspace().eventCount(sample);
        node.parentPct = total > 0 ? 100.0 * kept.length / total : 0;
        root.children.add(node);
        ctx.workspace().notifyTreeChanged();
        ctx.auditLog().add(AuditLog.Type.GATE, sample, String.format(
                "Population '%s' added from plugin (%,d events, %.1f%% of all events)",
                g.name, kept.length, node.parentPct));
        return node;
    }

    // ---- batch / apply-to-all ------------------------------------------------
    // A plugin's result is computed from the sample's own events, so unlike a geometric gate it
    // CANNOT be cloned across samples — batch means "re-run per sample", one population each.

    private final java.util.ArrayDeque<String> batchQueue = new java.util.ArrayDeque<>();
    private boolean batchCancelled;
    private int batchTotal, batchDone, batchFailed;

    @FXML
    private void onBatch() {
        PluginManifest m = pluginList.getSelectionModel().getSelectedItem();
        if (m == null) { statusLabel.setText("Select a plugin."); return; }
        if (ctx == null || ctx.workspace().sampleNames().isEmpty()) { statusLabel.setText("Load FCS first."); return; }

        ListView<String> picker = new ListView<>(FXCollections.observableArrayList(ctx.workspace().sampleNames()));
        picker.getSelectionModel().setSelectionMode(javafx.scene.control.SelectionMode.MULTIPLE);
        picker.getSelectionModel().selectAll();
        picker.setPrefHeight(280);

        VBox box = new VBox(8,
                new Label("Run \"" + m.name() + "\" on the selected samples. Each sample is analysed "
                        + "separately (results cannot be copied between samples) and gets its own population."),
                new Label("Ctrl/Shift-click to change the selection. All samples are selected by default."),
                picker);
        box.setStyle("-fx-padding:14;");

        javafx.scene.control.Dialog<javafx.scene.control.ButtonType> dlg = new javafx.scene.control.Dialog<>();
        dlg.setTitle("Run on samples — " + m.name());
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(box);
        dlg.getDialogPane().getButtonTypes().addAll(
                javafx.scene.control.ButtonType.OK, javafx.scene.control.ButtonType.CANCEL);
        AppIcons.theme(dlg, statusLabel.getScene() == null ? null : statusLabel.getScene().getWindow());
        if (dlg.showAndWait().orElse(javafx.scene.control.ButtonType.CANCEL) != javafx.scene.control.ButtonType.OK) return;

        List<String> chosen = new ArrayList<>(picker.getSelectionModel().getSelectedItems());
        if (chosen.isEmpty()) { statusLabel.setText("No samples selected."); return; }

        outputArea.clear();
        batchQueue.clear(); batchQueue.addAll(chosen);
        batchTotal = chosen.size(); batchDone = 0; batchFailed = 0; batchCancelled = false;
        runButton.setDisable(true); batchButton.setDisable(true); cancelButton.setDisable(false);
        nextInBatch(m);
    }

    private void nextInBatch(PluginManifest m) {
        if (batchCancelled || batchQueue.isEmpty()) {
            runButton.setDisable(false); batchButton.setDisable(false); cancelButton.setDisable(true);
            statusLabel.setText(String.format("Batch %s: %d/%d done%s%s.",
                    batchCancelled ? "cancelled" : "finished", batchDone, batchTotal,
                    batchFailed > 0 ? ", " + batchFailed + " failed" : "",
                    batchCancelled ? "" : ""));
            return;
        }
        String sample = batchQueue.poll();
        statusLabel.setText(String.format("Batch %d/%d — %s…", batchDone + 1, batchTotal, sample));
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        ctx.jobs().run(ctx.bridge().command("sample_path", args),
                r -> launchBatchOne(m, sample, r.path("path").asText()));
    }

    private void launchBatchOne(PluginManifest m, String sample, String fcsPath) {
        Map<String, Object> request = buildRequest(m, fcsPath);
        if (request == null) { batchCancelled = true; nextInBatch(m); return; }

        Task<JsonNode> task = PluginRunner.run(m, request);
        current = task;
        task.messageProperty().addListener((o, a, b) -> {
            if (b != null && !b.isBlank()) outputArea.appendText("[" + sample + "] " + b + "\n");
        });
        task.setOnSucceeded(e -> {
            JsonNode resp = task.getValue();
            batchDone++;
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, sample, auditDetail(m, resp));
            int[] kept = PluginRunner.indices(resp, "kept_indices");
            if (kept.length > 0) addPopulationFor(sample, kept, m.name());
            current = null;
            nextInBatch(m);
        });
        task.setOnFailed(e -> {
            batchDone++; batchFailed++;
            Throwable ex = task.getException();
            outputArea.appendText("[" + sample + "] ERROR: " + (ex == null ? "?" : ex.getMessage()) + "\n");
            current = null;
            nextInBatch(m);
        });
        Thread t = new Thread(task, "plugin-batch-" + m.id());
        t.setDaemon(true);
        t.start();
    }

    /** Collect the param form into a request; returns null (and reports) if a required field is blank. */
    private Map<String, Object> buildRequest(PluginManifest m, String fcsPath) {
        Map<String, Object> request = new LinkedHashMap<>();
        request.put("fcs_path", fcsPath);
        for (PluginManifest.Param p : m.params()) {
            TextField tf = paramFields.get(p.id());
            String raw = tf == null ? "" : tf.getText().trim();
            if (raw.isEmpty()) {
                if (p.required()) { statusLabel.setText("Parameter '" + p.label() + "' is required."); return null; }
                continue;   // let the plugin apply its own default
            }
            request.put(p.id(), coerce(p.type(), raw));
        }
        return request;
    }

    /** A one-line, methods-paragraph-friendly summary of a plugin run. */
    private static String auditDetail(PluginManifest m, JsonNode resp) {
        StringBuilder sb = new StringBuilder(m.name()).append(" v").append(m.version());
        if (resp.has("n_removed") && resp.has("pct_removed")) {
            sb.append(String.format(": removed %,d events (%.2f%%)",
                    resp.path("n_removed").asInt(), resp.path("pct_removed").asDouble()));
            if (resp.has("n_good")) sb.append(String.format(", kept %,d", resp.path("n_good").asInt()));
        } else if (resp.has("n_gates")) {
            sb.append(": produced ").append(resp.path("n_gates").asInt()).append(" gate(s)");
        }
        return sb.toString();
    }

    /** Manifest types are declarative; convert the typed text back to what the R side expects. */
    private static Object coerce(String type, String raw) {
        try {
            return switch (type == null ? "string" : type) {
                case "int" -> Long.parseLong(raw);
                case "double" -> Double.parseDouble(raw);
                case "bool", "boolean" -> Boolean.parseBoolean(raw);
                case "channels" -> {
                    List<String> out = new ArrayList<>();
                    for (String s : raw.split(",")) if (!s.isBlank()) out.add(s.trim());
                    yield out;
                }
                default -> raw;
            };
        } catch (NumberFormatException e) {
            return raw;   // let the plugin validate and report
        }
    }
}
