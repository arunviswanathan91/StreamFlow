package org.streamflow.ui;

import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.TextArea;
import javafx.stage.FileChooser;

import java.io.BufferedWriter;
import java.io.File;
import java.io.FileWriter;
import java.io.IOException;
import java.util.ArrayList;
import java.util.List;

/**
 * Analysis-Log module (S5). Binds to the session-wide {@link AuditLog}
 * (held in {@link AppContext}), shows all logged operations in a table,
 * and can generate a journal-ready Methods paragraph or export as CSV.
 */
public class AnalysisLogController implements ContextAware {

    @FXML private Button   clearButton;
    @FXML private Button   methodsButton;
    @FXML private Button   gatingMethodsButton;
    @FXML private Button   exportCsvButton;
    @FXML private TableView<AuditLog.Entry>         logTable;
    @FXML private TableColumn<AuditLog.Entry, String> timeCol;
    @FXML private TableColumn<AuditLog.Entry, String> typeCol;
    @FXML private TableColumn<AuditLog.Entry, String> sampleCol;
    @FXML private TableColumn<AuditLog.Entry, String> detailCol;
    @FXML private TextArea methodsArea;

    private AppContext ctx;

    @FXML
    public void initialize() {
        timeCol.setCellValueFactory(c   -> new ReadOnlyStringWrapper(c.getValue().time()));
        typeCol.setCellValueFactory(c   -> new ReadOnlyStringWrapper(c.getValue().type().name()));
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().sample()));
        detailCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().detail()));
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        logTable.setItems(ctx.auditLog().entries());
        setDisabled(false);
    }

    @FXML
    private void onClear() {
        if (ctx != null) {
            ctx.auditLog().clear();
            methodsArea.clear();
        }
    }

    @FXML
    private void onMethods() {
        if (ctx == null) return;
        methodsArea.setText(ctx.auditLog().generateMethodsText());
    }

    /** #22 — walk the gating tree(s) to produce a journal-ready gating-strategy paragraph
     *  with mean±SD frequencies across opened samples and channel aliases. */
    @FXML
    private void onGatingMethods() {
        if (ctx == null) { return; }
        WorkspaceModel ws = ctx.workspace();

        // samples with cached events
        List<String> samples = new ArrayList<>();
        for (String s : ws.sampleNames()) {
            EventData d = ws.data(s);
            if (d != null && d.rows() > 0) samples.add(s);
        }
        if (samples.isEmpty()) {
            methodsArea.setText("No opened samples with gates. Open samples in graph windows and draw gates first.");
            return;
        }
        // use the first sample's tree for structure; aggregate frequencies across all samples
        PopNode root = ws.treeFor(samples.get(0));
        List<PopNode> ordered = new ArrayList<>();
        for (PopNode n : root.selfAndDescendants()) if (!n.isRoot()) ordered.add(n);
        if (ordered.isEmpty()) {
            methodsArea.setText("No gates drawn yet on " + samples.get(0) + ".");
            return;
        }

        StringBuilder sb = new StringBuilder();
        sb.append("Gating strategy. Events were analysed in StreamFLOW. ");
        if (!ctx.aliases().all().isEmpty()) {
            sb.append("Detectors were mapped to markers (");
            sb.append(ctx.aliases().all().entrySet().stream()
                    .map(en -> en.getValue() + "=" + en.getKey())
                    .reduce((a, b) -> a + ", " + b).orElse(""));
            sb.append("). ");
        }
        sb.append("Populations were identified hierarchically: ");
        for (PopNode n : ordered) {
            String parent = (n.parent == null || n.parent.isRoot()) ? "all events" : n.parent.name();
            String axes = n.gate.yChan == null
                    ? ctx.aliases().label(n.gate.xChan)
                    : ctx.aliases().label(n.gate.xChan) + " vs " + ctx.aliases().label(n.gate.yChan);
            double[] stat = freqStats(ws, samples, n.name());
            sb.append(String.format("%s (%s gate on %s; %.1f±%.1f%% of %s, n=%d); ",
                    n.name(), n.gate.type, axes, stat[0], stat[1], parent, (int) stat[2]));
        }
        sb.setLength(Math.max(0, sb.length() - 2));
        sb.append(". Frequencies are mean±SD of the parent population across the analysed samples.");
        methodsArea.setText(sb.toString());
        ctx.auditLog().add(AuditLog.Type.EXPORT, "", "Generated gating methods text (" + ordered.size() + " gates)");
    }

    /** {mean%, sd%, nSamples} of a gate's percent-of-parent across the given samples. */
    private double[] freqStats(WorkspaceModel ws, List<String> samples, String gateName) {
        List<Double> vals = new ArrayList<>();
        for (String s : samples) {
            PopNode node = findByName(ws.treeFor(s), gateName);
            EventData d = ws.data(s);
            if (node == null || node.parent == null || d == null) continue;
            boolean[] parentKeep = chainMask(d, node.parent);
            boolean[] selfKeep = chainMask(d, node);
            int pc = 0, sc = 0;
            for (int i = 0; i < d.rows(); i++) { if (parentKeep[i]) pc++; if (selfKeep[i]) sc++; }
            if (pc > 0) vals.add(100.0 * sc / pc);
        }
        if (vals.isEmpty()) return new double[]{0, 0, 0};
        double m = vals.stream().mapToDouble(Double::doubleValue).average().orElse(0);
        double sd = vals.size() < 2 ? 0 : Math.sqrt(vals.stream().mapToDouble(v -> (v - m) * (v - m)).sum() / (vals.size() - 1));
        return new double[]{m, sd, vals.size()};
    }

    private boolean[] chainMask(EventData d, PopNode node) {
        boolean[] keep = new boolean[d.rows()];
        java.util.Arrays.fill(keep, true);
        if (node.isRoot()) return keep;
        for (CytoPlot.Gate g : node.chain()) {
            boolean[] m = CytoPlot.mask(d, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        return keep;
    }

    private PopNode findByName(PopNode root, String name) {
        for (PopNode n : root.selfAndDescendants()) if (!n.isRoot() && n.name().equals(name)) return n;
        return null;
    }

    @FXML
    private void onExportCsv() {
        if (ctx == null || ctx.auditLog().entries().isEmpty()) return;
        FileChooser fc = new FileChooser();
        fc.setTitle("Export Analysis Log");
        fc.setInitialFileName("analysis-log.csv");
        fc.getExtensionFilters().add(
                new FileChooser.ExtensionFilter("CSV files (*.csv)", "*.csv"));
        File f = fc.showSaveDialog(
                logTable.getScene() == null ? null : logTable.getScene().getWindow());
        if (f == null) return;
        try (BufferedWriter w = new BufferedWriter(new FileWriter(f))) {
            w.write("time,type,sample,detail\n");
            for (AuditLog.Entry e : ctx.auditLog().entries()) {
                w.write(String.format("\"%s\",\"%s\",\"%s\",\"%s\"\n",
                        e.time(), e.type(), e.sample(), e.detail().replace("\"", "\"\"")));
            }
        } catch (IOException ex) {
            methodsArea.setText("Export failed: " + ex.getMessage());
        }
    }

    private void setDisabled(boolean d) {
        clearButton.setDisable(d);
        methodsButton.setDisable(d);
        gatingMethodsButton.setDisable(d);
        exportCsvButton.setDisable(d);
    }
}
