package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ComboBox;
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
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;

/**
 * Analysis module A4 — Statistical Comparison. Builds per-sample frequencies for a chosen
 * gate from the shared {@link WorkspaceModel} (using cached events + the gate chain), lets the
 * user assign each sample to a group, then runs the engine's {@code run_stats_comparison}
 * (Mann-Whitney / Wilcoxon / Kruskal-Wallis + Dunn) and shows p-values + an annotated boxplot.
 *
 * <p>Only samples that have been opened at least once (so their events are cached) and contain
 * the selected gate can be included; the status line reports how many were used.
 */
public class StatComparisonController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private ComboBox<String> gateCombo;
    @FXML private CheckBox pairedCheck;
    @FXML private Button refreshButton;
    @FXML private Button runButton;
    @FXML private Label statusLabel;
    @FXML private Label resultLabel;
    @FXML private TableView<Row> sampleTable;
    @FXML private TableColumn<Row, String> sampleCol;
    @FXML private TableColumn<Row, String> freqCol;
    @FXML private TableColumn<Row, String> groupCol;
    @FXML private TableView<PostHoc> posthocTable;
    @FXML private TableColumn<PostHoc, String> pairCol;
    @FXML private TableColumn<PostHoc, String> pCol;
    @FXML private TableColumn<PostHoc, String> starCol;
    @FXML private ImageView plotView;

    private final ObservableList<Row> rows = FXCollections.observableArrayList();
    private final ObservableList<PostHoc> posthocRows = FXCollections.observableArrayList();
    private AppContext ctx;

    /** A sample row: its name, computed gate frequency (or "—"), and an editable group label. */
    public static final class Row {
        final String sample;
        final Double freq;                  // null if not computable (sample not opened / no gate)
        final StringProperty group = new SimpleStringProperty("");
        Row(String sample, Double freq) { this.sample = sample; this.freq = freq; }
    }

    public record PostHoc(String pair, String p, String stars) {}

    @FXML
    public void initialize() {
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sample));
        freqCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(
                c.getValue().freq == null ? "—" : String.format("%.2f", c.getValue().freq)));
        groupCol.setCellValueFactory(c -> c.getValue().group);
        groupCol.setCellFactory(TextFieldTableCell.forTableColumn());
        groupCol.setOnEditCommit(e -> e.getRowValue().group.set(e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(rows);
        sampleTable.setEditable(true);

        pairCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().pair()));
        pCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().p()));
        starCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().stars()));
        posthocTable.setItems(posthocRows);

        plotView.setPreserveRatio(true);
        // recompute frequencies when the user picks a different gate
        gateCombo.getSelectionModel().selectedItemProperty().addListener((o, prev, sel) -> {
            if (sel != null) rebuildRows();
        });
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refresh();
    }

    @FXML
    private void onRefresh() { refresh(); }

    /** Rescan the workspace for gate names and rebuild the sample/group table. */
    private void refresh() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();

        // Union of all non-root gate names across all sample trees.
        TreeSet<String> gateNames = new TreeSet<>();
        for (String s : ws.samples()) {
            PopNode root = ws.treeFor(s);
            for (PopNode n : root.selfAndDescendants()) {
                if (!n.isRoot()) gateNames.add(n.name());
            }
        }
        String prevGate = gateCombo.getValue();
        gateCombo.setItems(FXCollections.observableArrayList(gateNames));
        if (prevGate != null && gateNames.contains(prevGate)) gateCombo.setValue(prevGate);
        else if (!gateNames.isEmpty()) gateCombo.getSelectionModel().selectFirst();

        rebuildRows();
        if (gateNames.isEmpty()) {
            statusLabel.setText("No gates found. Draw at least one gate (and open the samples) first.");
        }
    }

    /** Rebuild the per-sample rows + frequencies for the currently selected gate. */
    private void rebuildRows() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();
        String gate = gateCombo.getValue();

        // preserve existing group assignments by sample name
        Map<String, String> prevGroups = new LinkedHashMap<>();
        for (Row r : rows) prevGroups.put(r.sample, r.group.get());

        rows.clear();
        int computed = 0;
        for (String s : ws.sampleNames()) {
            Double freq = (gate == null) ? null : freqForSample(s, gate);
            Row row = new Row(s, freq);
            if (prevGroups.containsKey(s)) row.group.set(prevGroups.get(s));
            rows.add(row);
            if (freq != null) computed++;
        }
        statusLabel.setText(gate == null ? "Select a gate."
                : String.format("Gate '%s': frequency computed for %d of %d sample(s). "
                        + "Open the others to include them.", gate, computed, rows.size()));
    }

    /** Frequency (% of all events) of the named gate for a sample, from cached events + chain masks. */
    private Double freqForSample(String sample, String gateName) {
        WorkspaceModel ws = ctx.workspace();
        if (!ws.hasTree(sample)) return null;
        EventData root = ws.data(sample);
        if (root == null || root.rows() == 0) return null;   // sample not opened → no cached events
        PopNode target = findByName(ws.treeFor(sample), gateName);
        if (target == null) return null;
        boolean[] keep = new boolean[root.rows()];
        Arrays.fill(keep, true);
        for (CytoPlot.Gate g : target.chain()) {
            boolean[] m = CytoPlot.mask(root, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        int c = 0;
        for (boolean b : keep) if (b) c++;
        return 100.0 * c / root.rows();
    }

    private PopNode findByName(PopNode root, String name) {
        for (PopNode n : root.selfAndDescendants()) {
            if (!n.isRoot() && n.name().equals(name)) return n;
        }
        return null;
    }

    @FXML
    private void onRun() {
        if (ctx == null || gateCombo.getValue() == null) return;

        // group label -> list of frequencies
        Map<String, List<Double>> groups = new LinkedHashMap<>();
        for (Row r : rows) {
            String g = r.group.get();
            if (g == null || g.isBlank() || r.freq == null) continue;
            groups.computeIfAbsent(g.trim(), k -> new ArrayList<>()).add(r.freq);
        }
        long usable = groups.values().stream().filter(l -> l.size() >= 2).count();
        if (usable < 2) {
            statusLabel.setText("Assign at least 2 groups with ≥2 computed samples each.");
            return;
        }

        ObjectNode a = JSON.createObjectNode();
        a.put("gate", gateCombo.getValue());
        a.put("paired", pairedCheck.isSelected());
        ObjectNode gNode = a.putObject("groups");
        groups.forEach((name, vals) -> {
            ArrayNode arr = gNode.putArray(name);
            vals.forEach(arr::add);
        });

        statusLabel.setText("Running statistical test…");
        ctx.jobs().run(ctx.bridge().command("run_stats_comparison", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        resultLabel.setText(String.format("%s — p = %.4g  %s",
                result.path("test").asText(), result.path("p_value").asDouble(),
                result.path("stars").asText()));

        posthocRows.clear();
        for (JsonNode ph : result.path("posthoc")) {
            posthocRows.add(new PostHoc(ph.path("pair").asText(),
                    String.format("%.4g", ph.path("p").asDouble()), ph.path("stars").asText()));
        }

        Path png = Paths.get(result.path("png").asText());
        try (InputStream in = Files.newInputStream(png)) {
            plotView.setImage(new Image(in));
            statusLabel.setText("Test complete: " + result.path("test").asText() + ".");
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, gateCombo.getValue(),
                    String.format("Stats Comparison: %s p=%.4g %s",
                            result.path("test").asText(),
                            result.path("p_value").asDouble(),
                            result.path("stars").asText()));
        } catch (Exception e) {
            statusLabel.setText("Could not load plot: " + e.getMessage());
        } finally {
            try { Files.deleteIfExists(png); } catch (Exception ignored) {}
        }
    }

    private void setDisabled(boolean d) {
        gateCombo.setDisable(d);
        pairedCheck.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
