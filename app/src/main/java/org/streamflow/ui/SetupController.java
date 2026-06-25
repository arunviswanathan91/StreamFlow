package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.geometry.Insets;
import javafx.scene.Scene;
import javafx.stage.Stage;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableCell;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableRow;
import javafx.scene.control.TableView;
import javafx.scene.control.TextField;
import javafx.scene.paint.Color;
import javafx.stage.DirectoryChooser;
import javafx.stage.FileChooser;

import java.io.File;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Phase 1 — Setup module: import FCS files from a folder and show the resulting
 * samples and channels. Talks to the engine's {@code load_fcs} / {@code get_state}
 * commands via the shared {@link AppContext}.
 */
public class SetupController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private TextField experimentField;
    @FXML private Label folderLabel;
    @FXML private CheckBox recursiveCheck;
    @FXML private Button chooseFolderButton;
    @FXML private Button loadButton;
    @FXML private Button keywordsButton;
    @FXML private Button exportGatingMlButton;
    @FXML private Button exportFcsButton;
    @FXML private Button panelCheckButton;
    @FXML private Label summaryLabel;

    @FXML private TableView<SampleRow> sampleTable;
    @FXML private TableColumn<SampleRow, String> sampleNameCol;
    @FXML private TableColumn<SampleRow, Number> sampleEventsCol;

    @FXML private TableView<ChannelRow> channelTable;
    @FXML private TableColumn<ChannelRow, String> channelNameCol;
    @FXML private TableColumn<ChannelRow, String> channelMarkerCol;
    @FXML private TableColumn<ChannelRow, String> channelTypeCol;

    private final ObservableList<SampleRow> samples = FXCollections.observableArrayList();
    private final ObservableList<ChannelRow> channels = FXCollections.observableArrayList();

    private final List<String> channelNames = new ArrayList<>();
    private AppContext ctx;
    private File chosenFolder;
    private int maxEvents = 1;
    private double medianEvents = 0;

    @FXML
    public void initialize() {
        sampleNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name()));
        sampleEventsCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().events()));
        channelNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().channel()));
        channelMarkerCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().marker()));
        channelTypeCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().type()));
        sampleTable.setItems(samples);
        channelTable.setItems(channels);

        // QC sparkline column — event-count bar coloured by deviation from median.
        TableColumn<SampleRow, SampleRow> qcCol = new TableColumn<>("QC");
        qcCol.setPrefWidth(80);
        qcCol.setResizable(false);
        qcCol.setSortable(false);
        qcCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue()));
        qcCol.setCellFactory(tv -> new TableCell<>() {
            private final Canvas canvas = new Canvas(68, 12);
            { setGraphic(canvas); setText(null); }
            @Override protected void updateItem(SampleRow row, boolean empty) {
                super.updateItem(row, empty);
                if (empty || row == null) { canvas.setVisible(false); return; }
                canvas.setVisible(true);
                SetupController.this.drawQcBar(canvas, row);
            }
        });
        sampleTable.getColumns().add(qcCol);

        // Double-click a sample -> open its FlowJo-style Graph Window.
        sampleTable.setRowFactory(tv -> {
            TableRow<SampleRow> row = new TableRow<>();
            row.setOnMouseClicked(ev -> {
                if (ev.getClickCount() == 2 && !row.isEmpty() && ctx != null && !channelNames.isEmpty()) {
                    List<String> names = samples.stream().map(SampleRow::name).toList();
                    GraphWindowController.open(ctx, names, row.getIndex(), channelNames);
                }
            });
            return row;
        });
        setControlsDisabled(true); // until the engine is ready
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setControlsDisabled(false);
    }

    @FXML
    private void onChooseFolder() {
        DirectoryChooser dc = new DirectoryChooser();
        dc.setTitle("Select FCS folder");
        if (chosenFolder != null && chosenFolder.getParentFile() != null) {
            dc.setInitialDirectory(chosenFolder.getParentFile());
        }
        File dir = dc.showDialog(folderLabel.getScene().getWindow());
        if (dir != null) {
            chosenFolder = dir;
            folderLabel.setText(dir.getAbsolutePath());
            loadButton.setDisable(false);
        }
    }

    @FXML
    private void onLoad() {
        if (ctx == null || chosenFolder == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("folder", chosenFolder.getAbsolutePath().replace('\\', '/'));
        args.put("recursive", recursiveCheck.isSelected());
        ctx.jobs().status("Loading FCS from " + chosenFolder.getName() + "…");
        ctx.jobs().run(ctx.bridge().command("load_fcs", args), this::populate);
    }

    /** Repopulate the view from a summary payload (load_fcs / get_state / workspace load). */
    public void populate(JsonNode summary) {
        if (summary == null) return;
        if (summary.hasNonNull("experiment")) {
            experimentField.setText(summary.path("experiment").asText());
        }
        samples.clear();
        for (JsonNode s : summary.path("samples")) {
            samples.add(new SampleRow(s.path("name").asText(), s.path("events").asInt()));
        }
        channels.clear();
        channelNames.clear();
        for (JsonNode c : summary.path("channels")) {
            String ch = c.path("channel").asText();
            channelNames.add(ch);
            channels.add(new ChannelRow(
                    ch,
                    c.path("marker").asText(""),
                    c.path("scatter").asBoolean(false) ? "scatter" : "fluorescence"));
        }
        int skipped = summary.path("skipped").asInt(0);
        summaryLabel.setText(String.format("%d sample(s), %d channel(s)%s",
                samples.size(), channels.size(),
                skipped > 0 ? " — " + skipped + " file(s) skipped" : ""));
        ctx.jobs().status("Loaded " + samples.size() + " sample(s).");

        // Compute max + median event counts so the QC sparkline column can colour bars.
        int[] counts = samples.stream().mapToInt(SampleRow::events).sorted().toArray();
        maxEvents = counts.length > 0 ? counts[counts.length - 1] : 1;
        medianEvents = counts.length > 0 ? counts[counts.length / 2] : 0;
        sampleTable.refresh();
        keywordsButton.setDisable(samples.isEmpty());

        // Publish to the workspace so the Workstation view can list all samples.
        if (ctx != null) {
            ctx.workspace().setSamples(samples.stream().map(SampleRow::name).toList());
            ctx.workspace().setChannelNames(channelNames);
            for (SampleRow s : samples) ctx.workspace().setEventCount(s.name(), s.events());  // QC
        }
    }

    @FXML
    private void onShowKeywords() {
        if (ctx == null || samples.isEmpty()) return;
        SampleRow sel = sampleTable.getSelectionModel().getSelectedItem();
        String name = (sel != null) ? sel.name() : samples.get(0).name();
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", name);
        ctx.jobs().run(ctx.bridge().command("get_metadata", args), result -> {
            TableColumn<String[], String> keyCol = new TableColumn<>("Keyword");
            keyCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[0]));
            keyCol.setPrefWidth(220);
            TableColumn<String[], String> valCol = new TableColumn<>("Value");
            valCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[1]));
            valCol.setPrefWidth(420);
            TableView<String[]> table = new TableView<>();
            table.getColumns().addAll(List.of(keyCol, valCol));

            ObservableList<String[]> krows = FXCollections.observableArrayList();
            result.path("keywords").fields().forEachRemaining(
                    e -> krows.add(new String[]{e.getKey(), e.getValue().asText()}));
            krows.sort((a, b) -> a[0].compareToIgnoreCase(b[0]));
            table.setItems(krows);

            javafx.scene.layout.VBox root = new javafx.scene.layout.VBox(table);
            root.setPadding(new Insets(12));
            javafx.scene.layout.VBox.setVgrow(table, javafx.scene.layout.Priority.ALWAYS);

            Stage stage = new Stage();
            stage.setTitle("FCS Keywords — " + name
                    + " (" + result.path("count").asInt() + " keys)");
            stage.setScene(new Scene(root, 680, 520));
            stage.show();
        });
    }

    @FXML
    private void onExportGatingMl() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();
        ObjectNode gatesNode = serializeGates(ws);
        if (gatesNode.isEmpty()) {
            ctx.jobs().status("No gates to export. Draw gates first.");
            return;
        }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export GatingML 2.0");
        fc.setInitialFileName("gates.xml");
        fc.getExtensionFilters().add(
                new FileChooser.ExtensionFilter("GatingML 2.0 (*.xml)", "*.xml"));
        File f = fc.showSaveDialog(
                sampleTable.getScene() == null ? null : sampleTable.getScene().getWindow());
        if (f == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        args.set("gates", gatesNode);
        ctx.jobs().run(ctx.bridge().command("save_gatingml", args),
                r -> ctx.jobs().status("GatingML exported: " + r.path("n_gates").asInt()
                        + " gate(s) → " + f.getName()));
    }

    @FXML
    private void onExportFcs() {
        if (ctx == null || samples.isEmpty()) return;
        SampleRow sel = sampleTable.getSelectionModel().getSelectedItem();
        if (sel == null) { ctx.jobs().status("Select a sample in the table first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export FCS 3.1");
        fc.setInitialFileName(sel.name().replaceAll("(?i)\\.fcs$", "") + "_export.fcs");
        fc.getExtensionFilters().add(
                new FileChooser.ExtensionFilter("FCS files (*.fcs)", "*.fcs"));
        File f = fc.showSaveDialog(
                sampleTable.getScene() == null ? null : sampleTable.getScene().getWindow());
        if (f == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sel.name());
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        args.put("compensated", false);
        ctx.jobs().run(ctx.bridge().command("export_fcs", args),
                r -> ctx.jobs().status(String.format("FCS exported: %,d events × %d channels → %s",
                        r.path("n").asInt(), r.path("channels").asInt(), f.getName())));
    }

    /** Walk WorkspaceModel trees and serialise all non-root gate nodes as JSON. */
    private ObjectNode serializeGates(WorkspaceModel ws) {
        ObjectNode out = JSON.createObjectNode();
        for (String sample : ws.samples()) {
            ArrayNode sampleArr = JSON.createArrayNode();
            PopNode root = ws.treeFor(sample);
            Map<PopNode, String> idMap = new LinkedHashMap<>();
            int seq = 0;
            for (PopNode n : root.selfAndDescendants()) {
                if (!n.isRoot()) idMap.put(n, sample + "_G" + (++seq));
            }
            for (PopNode n : root.selfAndDescendants()) {
                if (n.isRoot()) continue;
                CytoPlot.Gate g = n.gate;
                ObjectNode gn = JSON.createObjectNode();
                gn.put("id",   idMap.get(n));
                gn.put("name", g.name);
                if (n.parent != null && !n.parent.isRoot()) gn.put("parent_id", idMap.get(n.parent));
                gn.put("type",      g.type);
                gn.put("x_channel", g.xChan);
                if (g.yChan != null && !g.yChan.isBlank()) gn.put("y_channel", g.yChan);
                ArrayNode xsArr = JSON.createArrayNode();
                if (g.xs != null) for (double x : g.xs) xsArr.add(x);
                gn.set("xs", xsArr);
                ArrayNode ysArr = JSON.createArrayNode();
                if (g.ys != null) for (double y : g.ys) ysArr.add(y);
                gn.set("ys", ysArr);
                sampleArr.add(gn);
            }
            if (sampleArr.size() > 0) out.set(sample, sampleArr);
        }
        return out;
    }

    /** #19 — match panel channels (and aliases) to fluorochromes; warn on emission overlap. */
    @FXML
    private void onPanelCheck() {
        if (channelNames.isEmpty()) { if (ctx != null) ctx.jobs().status("Load FCS first."); return; }
        List<String> labels = new ArrayList<>();
        for (String ch : channelNames) {
            labels.add(ch);
            if (ctx != null) {
                String t = ctx.aliases().target(ch);   // also test the alias (often holds the fluor)
                if (t != null && !t.isBlank()) labels.add(t);
            }
        }
        List<SpectralReference.Conflict> conflicts = SpectralReference.conflicts(labels, 30);

        StringBuilder sb = new StringBuilder();
        long matched = labels.stream().filter(l -> SpectralReference.matchFluor(l) != null).count();
        sb.append(matched).append(" channel label(s) matched to known fluorochromes.\n\n");
        if (conflicts.isEmpty()) {
            sb.append("No emission conflicts within 30 nm — panel looks spectrally clean.");
        } else {
            sb.append(conflicts.size()).append(" potential spillover pair(s) (emission ≤30 nm apart):\n\n");
            for (SpectralReference.Conflict c : conflicts) {
                sb.append(String.format("• %s (%s, %dnm)  ↔  %s (%s, %dnm)  — Δ%dnm%n",
                        c.chanA(), c.fluorA(), c.emA(), c.chanB(), c.fluorB(), c.emB(),
                        Math.abs(c.emA() - c.emB())));
            }
            sb.append("\nConsider compensation controls or swapping a reagent.");
        }

        javafx.scene.control.TextArea area = new javafx.scene.control.TextArea(sb.toString());
        area.setEditable(false); area.setWrapText(true); area.setPrefRowCount(16);
        javafx.scene.layout.VBox box = new javafx.scene.layout.VBox(area);
        box.setPadding(new Insets(12));
        javafx.scene.layout.VBox.setVgrow(area, javafx.scene.layout.Priority.ALWAYS);
        Stage stage = new Stage();
        stage.setTitle("Panel spectral check (" + conflicts.size() + " conflict(s))");
        stage.setScene(new Scene(box, 560, 420));
        stage.show();
    }

    private void setControlsDisabled(boolean disabled) {
        chooseFolderButton.setDisable(disabled);
        recursiveCheck.setDisable(disabled);
        experimentField.setDisable(disabled);
        loadButton.setDisable(disabled || chosenFolder == null);
        keywordsButton.setDisable(true);
        exportGatingMlButton.setDisable(disabled);
        exportFcsButton.setDisable(disabled);
        panelCheckButton.setDisable(disabled);
    }

    private void drawQcBar(Canvas canvas, SampleRow row) {
        double w = canvas.getWidth(), h = canvas.getHeight();
        GraphicsContext gc = canvas.getGraphicsContext2D();
        gc.clearRect(0, 0, w, h);
        if (maxEvents <= 0) return;
        double ratio = Math.min(1.0, (double) row.events() / maxEvents);
        double deviation = medianEvents > 0 ? Math.abs(row.events() - medianEvents) / medianEvents : 0;
        Color bar = deviation < 0.20 ? Color.web("#4CAF50")
                  : deviation < 0.50 ? Color.web("#FFC107")
                  :                    Color.web("#F44336");
        gc.setFill(Color.web("#E8E8E8"));
        gc.fillRoundRect(0, 1, w, h - 2, 3, 3);
        gc.setFill(bar);
        gc.fillRoundRect(0, 1, w * ratio, h - 2, 3, 3);
    }

    public record SampleRow(String name, int events) {}
    public record ChannelRow(String channel, String marker, String type) {}
}
