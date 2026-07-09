package org.streamflow.ui;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyObjectWrapper;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.TextArea;
import javafx.stage.FileChooser;

import java.io.BufferedWriter;
import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Statistics — per-sample × per-population count, % of parent, % of total and median MFI per channel,
 * computed natively in Java from the gating tree (so drawn populations like P1 appear). The left list
 * selects which populations to include; events are fetched from the engine on demand and cached.
 */
public class StatisticsController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Button computeButton;
    @FXML private Button refreshButton;
    @FXML private Button exportButton;
    @FXML private Button helpButton;
    @FXML private Label statusLabel;
    @FXML private TableView<Row> table;
    @FXML private ListView<String> popList;

    private final ObservableList<Row> rows = FXCollections.observableArrayList();
    private final List<String> channels = new ArrayList<>();
    private AppContext ctx;

    /** sample, population, count, %parent, %total, median MFI per channel (aligned to {@code channels}). */
    public record Row(String sample, String population, int count, double pctParent, double pctTotal, List<Double> mfi) {}

    @FXML
    public void initialize() {
        table.setItems(rows);
        popList.getSelectionModel().setSelectionMode(SelectionMode.MULTIPLE);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshPopulations();
    }

    @FXML
    private void onRefresh() { refreshPopulations(); }
    @Override public void refreshFromWorkspace() { refreshPopulations(); }

    private void refreshPopulations() {
        if (ctx == null) return;
        Set<String> names = new LinkedHashSet<>();
        names.add("All Events");
        for (String s : ctx.workspace().samples())
            for (PopNode n : ctx.workspace().treeFor(s).selfAndDescendants())
                if (!n.isRoot()) names.add(n.name());
        popList.getItems().setAll(names);
        statusLabel.setText(names.size() + " population(s) available. Select some (or none = all), then Compute.");
    }

    @FXML
    private void onCompute() {
        if (ctx == null) return;
        List<String> samples = new ArrayList<>(ctx.workspace().sampleNames());
        if (samples.isEmpty()) { statusLabel.setText("Load FCS first."); return; }
        channels.clear();
        rows.clear();
        computeNext(samples, 0);
    }

    /** Sequentially ensure each sample's events are loaded, compute its population rows, then render. */
    private void computeNext(List<String> samples, int i) {
        if (i >= samples.size()) { renderTable(); return; }
        String sample = samples.get(i);
        statusLabel.setText("Computing " + (i + 1) + "/" + samples.size() + " — " + sample + "…");
        ensureData(sample, () -> {
            try { computeSample(sample); } catch (Exception ignored) {}
            computeNext(samples, i + 1);
        });
    }

    private void computeSample(String sample) {
        EventData d = ctx.workspace().data(sample);
        if (d == null) return;
        if (channels.isEmpty())
            for (String ch : d.channels()) if (!isScatter(ch)) channels.add(ch);

        Set<String> selected = new LinkedHashSet<>(popList.getSelectionModel().getSelectedItems());
        PopNode root = ctx.workspace().hasTree(sample) ? ctx.workspace().treeFor(sample) : new PopNode(null, null);
        int total = d.rows();

        // count per node (cache so %parent is cheap)
        Map<PopNode, Integer> counts = new LinkedHashMap<>();
        Map<PopNode, EventData> subsets = new LinkedHashMap<>();
        for (PopNode n : root.selfAndDescendants()) {
            boolean[] keep = new boolean[d.rows()];
            java.util.Arrays.fill(keep, true);
            for (CytoPlot.Gate g : n.chain()) {
                boolean[] mk = CytoPlot.mask(d, g);
                for (int k = 0; k < keep.length && k < mk.length; k++) keep[k] &= mk[k];
            }
            EventData sub = n.isRoot() ? d : d.subset(keep);
            subsets.put(n, sub);
            counts.put(n, sub.rows());
        }

        for (PopNode n : root.selfAndDescendants()) {
            if (!selected.isEmpty() && !selected.contains(n.name())) continue;
            int cnt = counts.get(n);
            int parentCnt = n.parent == null ? total : counts.getOrDefault(n.parent, total);
            double pctParent = parentCnt > 0 ? 100.0 * cnt / parentCnt : 0;
            double pctTotal = total > 0 ? 100.0 * cnt / total : 0;
            EventData sub = subsets.get(n);
            List<Double> mfi = new ArrayList<>();
            for (String ch : channels) mfi.add(round2(median(sub, sub.indexOf(ch))));
            rows.add(new Row(sample, n.name(), cnt, round2(pctParent), round2(pctTotal), mfi));
        }
    }

    private void renderTable() {
        List<TableColumn<Row, ?>> cols = new ArrayList<>();
        TableColumn<Row, String> sampleCol = new TableColumn<>("Sample");
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sample()));
        sampleCol.setPrefWidth(200);
        TableColumn<Row, String> popCol = new TableColumn<>("Population");
        popCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().population()));
        popCol.setPrefWidth(120);
        TableColumn<Row, Number> countCol = new TableColumn<>("Count");
        countCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().count()));
        TableColumn<Row, Number> ppCol = new TableColumn<>("% Parent");
        ppCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().pctParent()));
        TableColumn<Row, Number> ptCol = new TableColumn<>("% Total");
        ptCol.setCellValueFactory(c -> new ReadOnlyObjectWrapper<>(c.getValue().pctTotal()));
        cols.add(sampleCol); cols.add(popCol); cols.add(countCol); cols.add(ppCol); cols.add(ptCol);
        for (int j = 0; j < channels.size(); j++) {
            final int idx = j;
            TableColumn<Row, Number> col = new TableColumn<>(channels.get(j));
            col.setPrefWidth(105);
            col.setCellValueFactory(c -> {
                List<Double> m = c.getValue().mfi();
                return new ReadOnlyObjectWrapper<>(idx < m.size() ? m.get(idx) : null);
            });
            cols.add(col);
        }
        table.getColumns().setAll(cols);
        statusLabel.setText(rows.size() + " row(s) × " + channels.size() + " channel(s).");
    }

    @FXML
    private void onExport() {
        if (rows.isEmpty()) { statusLabel.setText("Nothing to export — compute first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export statistics");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("CSV (*.csv)", "*.csv"));
        fc.setInitialFileName("statistics.csv");
        File f = fc.showSaveDialog(table.getScene().getWindow());
        if (f == null) return;
        try (BufferedWriter w = Files.newBufferedWriter(f.toPath())) {
            w.write("Sample,Population,Count,PercentParent,PercentTotal");
            for (String ch : channels) w.write("," + csv(ch));
            w.newLine();
            for (Row r : rows) {
                StringBuilder sb = new StringBuilder();
                sb.append(csv(r.sample())).append(',').append(csv(r.population())).append(',')
                  .append(r.count()).append(',').append(r.pctParent()).append(',').append(r.pctTotal());
                for (Double v : r.mfi()) sb.append(',').append(v == null ? "" : v);
                w.write(sb.toString());
                w.newLine();
            }
            statusLabel.setText("Exported " + rows.size() + " rows to " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("Export failed: " + e.getMessage());
        }
    }

    @FXML
    private void onHelp() {
        String msg =
            "STATISTICS\n" +
            "Builds a table of every selected population in every sample: event Count, % of its parent " +
            "population, % of all events, and the median fluorescence (MFI) for each channel.\n\n" +
            "SELECTING POPULATIONS\n" +
            "The left list shows every population in your gating trees (All Events, P1, …). Select one or " +
            "more to restrict the table; select none to include them all. Click 'Refresh populations' after " +
            "drawing new gates.\n\n" +
            "HOW IT'S COMPUTED\n" +
            "Counts and MFI are computed natively in Java from the gates you drew (data-space membership), " +
            "so a population you gated on one sample appears for that sample. Events are pulled from the " +
            "engine and cached on first use.\n\n" +
            "NEXT STEP\n" +
            "Export CSV for your stats package, or use Stats Comparison to test a population's frequency " +
            "across sample groups.";
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle("About Statistics");
        a.setHeaderText("Per-population counts and MFI across samples");
        TextArea ta = new TextArea(msg);
        ta.setEditable(false); ta.setWrapText(true); ta.setPrefSize(560, 360);
        a.getDialogPane().setContent(ta);
        AppIcons.theme(a, null);
        a.showAndWait();
    }

    // ---- helpers -------------------------------------------------------------

    private void ensureData(String sample, Runnable onReady) {
        EventData cached = ctx.workspace().data(sample);
        if (cached != null && cached.rows() > 0) { onReady.run(); return; }
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        ctx.jobs().run(ctx.bridge().command("get_events", args), r -> {
            try {
                List<String> chans = new ArrayList<>();
                r.path("channels").forEach(n -> chans.add(n.asText()));
                Path bin = Paths.get(r.path("file").asText());
                EventData d = EventData.read(bin, chans, r.path("rows").asInt(), r.path("cols").asInt());
                try { Files.deleteIfExists(bin); } catch (Exception ignored) {}
                ctx.workspace().putData(sample, d);
            } catch (Exception ignored) {}
            onReady.run();
        });
    }

    private static boolean isScatter(String ch) {
        return ch != null && ch.toUpperCase().matches(".*(FSC|SSC|TIME|WIDTH|EVENT).*");
    }

    private static double median(EventData d, int col) {
        if (col < 0 || d.rows() == 0) return 0;
        double[] v = new double[d.rows()];
        for (int r = 0; r < d.rows(); r++) v[r] = d.get(r, col);
        java.util.Arrays.sort(v);
        int n = v.length;
        return n % 2 == 1 ? v[n / 2] : (v[n / 2 - 1] + v[n / 2]) / 2;
    }

    private static double round2(double v) { return Math.round(v * 100.0) / 100.0; }

    private static String csv(String s) {
        return s.contains(",") || s.contains("\"") ? "\"" + s.replace("\"", "\"\"") + "\"" : s;
    }

    private void setDisabled(boolean d) {
        computeButton.setDisable(d);
        refreshButton.setDisable(d);
        exportButton.setDisable(d);
    }
}
