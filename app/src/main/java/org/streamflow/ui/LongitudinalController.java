package org.streamflow.ui;

import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleStringProperty;
import javafx.beans.property.StringProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.scene.chart.LineChart;
import javafx.scene.chart.NumberAxis;
import javafx.scene.chart.XYChart;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.TextFieldTableCell;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;

/**
 * Longitudinal tracking (differentiator #16). Plots a chosen gate's frequency across user-assigned
 * numeric timepoints, one line per subject. Pure-JavaFX over the shared {@link WorkspaceModel}
 * (samples with cached events). Frequencies recompute live from the gating tree.
 */
public class LongitudinalController implements ContextAware, Refreshable {

    @FXML private ComboBox<String> gateCombo;
    @FXML private Button refreshButton, plotButton;
    @FXML private Label statusLabel;
    @FXML private LineChart<Number, Number> chart;
    @FXML private NumberAxis xAxis, yAxis;
    @FXML private TableView<Row> sampleTable;
    @FXML private TableColumn<Row, String> sampleCol, subjectCol, timeCol;

    private final ObservableList<Row> rows = FXCollections.observableArrayList();
    private AppContext ctx;

    public static final class Row {
        final String sample;
        final StringProperty subject = new SimpleStringProperty("");
        final StringProperty time = new SimpleStringProperty("");
        Row(String sample) { this.sample = sample; }
    }

    @FXML
    public void initialize() {
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sample));
        subjectCol.setCellValueFactory(c -> c.getValue().subject);
        subjectCol.setCellFactory(TextFieldTableCell.forTableColumn());
        subjectCol.setOnEditCommit(e -> e.getRowValue().subject.set(e.getNewValue() == null ? "" : e.getNewValue().trim()));
        timeCol.setCellValueFactory(c -> c.getValue().time);
        timeCol.setCellFactory(TextFieldTableCell.forTableColumn());
        timeCol.setOnEditCommit(e -> e.getRowValue().time.set(e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(rows);
        sampleTable.setEditable(true);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refresh();
    }

    @FXML private void onRefresh() { refresh(); }
    @Override public void refreshFromWorkspace() { refresh(); }

    private void refresh() {
        if (ctx == null) return;
        WorkspaceModel ws = ctx.workspace();
        Map<String, String> prevSub = new LinkedHashMap<>(), prevT = new LinkedHashMap<>();
        for (Row r : rows) { prevSub.put(r.sample, r.subject.get()); prevT.put(r.sample, r.time.get()); }

        rows.clear();
        TreeSet<String> gates = new TreeSet<>();
        for (String s : ws.sampleNames()) {
            EventData d = ws.data(s);
            if (d == null || d.rows() == 0) continue;
            Row r = new Row(s);
            if (prevSub.containsKey(s)) r.subject.set(prevSub.get(s));
            if (prevT.containsKey(s)) r.time.set(prevT.get(s));
            rows.add(r);
            for (PopNode n : ws.treeFor(s).selfAndDescendants()) if (!n.isRoot()) gates.add(n.name());
        }
        String prevGate = gateCombo.getValue();
        gateCombo.setItems(FXCollections.observableArrayList(gates));
        if (prevGate != null && gates.contains(prevGate)) gateCombo.setValue(prevGate);
        else if (!gates.isEmpty()) gateCombo.getSelectionModel().selectFirst();
        statusLabel.setText(rows.isEmpty()
                ? "Open samples in graph windows to track them."
                : rows.size() + " sample(s). Assign subject + timepoint, then Plot.");
    }

    @FXML
    private void onPlot() {
        if (ctx == null || gateCombo.getValue() == null) return;
        WorkspaceModel ws = ctx.workspace();
        String gate = gateCombo.getValue();

        // subject -> list of (time, freq)
        Map<String, List<double[]>> series = new LinkedHashMap<>();
        int used = 0;
        for (Row r : rows) {
            String subj = r.subject.get();
            String tStr = r.time.get();
            if (subj == null || subj.isBlank() || tStr == null || tStr.isBlank()) continue;
            double t;
            try { t = Double.parseDouble(tStr.trim()); } catch (NumberFormatException ex) { continue; }
            EventData d = ws.data(r.sample);
            PopNode node = findByName(ws.treeFor(r.sample), gate);
            if (d == null || node == null) continue;
            series.computeIfAbsent(subj.trim(), k -> new ArrayList<>()).add(new double[]{t, freqOf(d, node)});
            used++;
        }
        if (series.isEmpty()) {
            statusLabel.setText("Assign at least one sample a subject and a numeric timepoint.");
            return;
        }
        chart.getData().clear();
        for (Map.Entry<String, List<double[]>> e : series.entrySet()) {
            List<double[]> pts = e.getValue();
            pts.sort(Comparator.comparingDouble(p -> p[0]));
            XYChart.Series<Number, Number> s = new XYChart.Series<>();
            s.setName(e.getKey());
            for (double[] p : pts) s.getData().add(new XYChart.Data<>(p[0], p[1]));
            chart.getData().add(s);
        }
        chart.setTitle(gate + " frequency over time");
        yAxis.setLabel(gate + " (% of all events)");
        statusLabel.setText(String.format("Plotted %d subject(s) from %d sample(s).", series.size(), used));
        ctx.auditLog().add(AuditLog.Type.ANALYSIS, gate,
                String.format("Longitudinal: %d subjects across %d samples", series.size(), used));
    }

    private double freqOf(EventData d, PopNode node) {
        boolean[] keep = new boolean[d.rows()];
        Arrays.fill(keep, true);
        for (CytoPlot.Gate g : node.chain()) {
            boolean[] m = CytoPlot.mask(d, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        int c = 0; for (boolean b : keep) if (b) c++;
        return d.rows() == 0 ? 0 : 100.0 * c / d.rows();
    }

    private PopNode findByName(PopNode root, String name) {
        for (PopNode n : root.selfAndDescendants()) if (!n.isRoot() && n.name().equals(name)) return n;
        return null;
    }

    private void setDisabled(boolean d) {
        gateCombo.setDisable(d);
        refreshButton.setDisable(d);
        plotButton.setDisable(d);
    }
}
