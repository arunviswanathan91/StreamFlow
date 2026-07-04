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
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.TextFieldTableCell;
import javafx.scene.image.WritableImage;
import javafx.scene.input.Clipboard;
import javafx.scene.input.ClipboardContent;
import javafx.scene.paint.Color;
import javafx.stage.FileChooser;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.File;
import java.util.regex.Pattern;

public class KineticController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern TIME_CHAN =
            Pattern.compile("^Time$|^TIME$|^time$", Pattern.CASE_INSENSITIVE);

    private static final Color[] PALETTE = {
        Color.web("#66C2A5"), Color.web("#FC8D62"), Color.web("#8DA0CB"),
        Color.web("#E78AC3"), Color.web("#A6D854"), Color.web("#FFD92F"),
        Color.web("#E5C494"), Color.web("#B3B3B3")
    };

    @FXML private ComboBox<String> channelCombo;
    @FXML private ComboBox<String> timeCombo;
    @FXML private ComboBox<String> binsCombo;
    @FXML private ComboBox<String> gateCombo;
    @FXML private Button           refreshButton;
    @FXML private Button           runButton;
    @FXML private Button           copyButton;
    @FXML private Button           exportButton;
    @FXML private Label            statusLabel;
    @FXML private AnalysisChart    chart;
    @FXML private TableView<SampleRow>       sampleTable;
    @FXML private TableColumn<SampleRow, String> sampleCol;
    @FXML private TableColumn<SampleRow, String> groupCol;

    private final ObservableList<SampleRow> sampleRows = FXCollections.observableArrayList();
    private AppContext ctx;

    public static final class SampleRow {
        final String name;
        final StringProperty group = new SimpleStringProperty("");
        SampleRow(String name) { this.name = name; }
    }

    @FXML
    public void initialize() {
        binsCombo.setItems(FXCollections.observableArrayList("50", "100", "200", "500"));
        binsCombo.getSelectionModel().select("100");

        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().name));
        groupCol.setCellValueFactory(c -> c.getValue().group);
        groupCol.setCellFactory(TextFieldTableCell.forTableColumn());
        groupCol.setOnEditCommit(e -> e.getRowValue().group.set(
                e.getNewValue() == null ? "" : e.getNewValue().trim()));
        sampleTable.setItems(sampleRows);
        sampleTable.setEditable(true);

        chart.setTitle("Kinetic"); chart.setAxisLabels("Time", "Median MFI");
        copyButton.setDisable(true); exportButton.setDisable(true);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        ctx.workspace().addTreeChangeListener(this::refreshGates);
        refreshChannels();
    }

    @FXML private void onRefresh() { refreshChannels(); }
    @Override public void refreshFromWorkspace() { refreshChannels(); }

    private void refreshChannels() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            sampleRows.clear();
            r.path("samples").forEach(n -> sampleRows.add(new SampleRow(n.asText())));
            channelCombo.getItems().clear();
            timeCombo.getItems().clear();
            String timeGuess = null;
            for (JsonNode c : r.path("channels")) {
                String ch = c.asText();
                channelCombo.getItems().add(ch);
                timeCombo.getItems().add(ch);
                if (timeGuess == null && TIME_CHAN.matcher(ch).find()) timeGuess = ch;
            }
            if (!channelCombo.getItems().isEmpty()) channelCombo.getSelectionModel().selectFirst();
            if (timeGuess != null) timeCombo.getSelectionModel().select(timeGuess);
            else if (!timeCombo.getItems().isEmpty()) timeCombo.getSelectionModel().selectFirst();
            refreshGates();
        });
    }

    // ---- Gate combo (collects unique gate names across all loaded samples) ----

    private void refreshGates() {
        if (ctx == null) return;
        String prev = gateCombo.getValue();
        gateCombo.getItems().clear();
        gateCombo.getItems().add("Ungated (All Events)");
        java.util.LinkedHashSet<String> names = new java.util.LinkedHashSet<>();
        for (SampleRow sr : sampleRows) {
            PopNode root = ctx.workspace().treeFor(sr.name);
            for (PopNode n : root.selfAndDescendants())
                if (!n.isRoot()) names.add(n.name());
        }
        gateCombo.getItems().addAll(names);
        if (prev != null && gateCombo.getItems().contains(prev))
            gateCombo.getSelectionModel().select(prev);
        else
            gateCombo.getSelectionModel().selectFirst();
    }

    /** Return the gate chain from the first sample that has a matching gate node. */
    private PopNode selectedGateNode() {
        String sel = gateCombo.getValue();
        if (sel == null || sel.startsWith("Ungated")) return null;
        for (SampleRow sr : sampleRows) {
            PopNode root = ctx.workspace().treeFor(sr.name);
            for (PopNode n : root.selfAndDescendants())
                if (!n.isRoot() && n.name().equals(sel)) return n;
        }
        return null;
    }

    @FXML
    private void onRun() {
        if (ctx == null || channelCombo.getValue() == null || timeCombo.getValue() == null) return;
        if (channelCombo.getValue().equals(timeCombo.getValue())) {
            statusLabel.setText("MFI channel and Time channel must be different.");
            return;
        }
        ObjectNode grpNode = JSON.createObjectNode();
        for (SampleRow r : sampleRows)
            if (!r.group.get().isBlank()) grpNode.put(r.name, r.group.get().trim());
        ObjectNode a = JSON.createObjectNode();
        a.put("channel", channelCombo.getValue());
        a.put("time_channel", timeCombo.getValue());
        a.put("bins", Integer.parseInt(binsCombo.getValue()));
        a.set("groups", grpNode);
        PopNode target = selectedGateNode();
        if (target != null) {
            ArrayNode polygons = JSON.createArrayNode();
            for (CytoPlot.Gate g : target.chain()) {
                ObjectNode gn = JSON.createObjectNode();
                gn.put("type", g.type); gn.put("x_channel", g.xChan);
                gn.put("y_channel", g.yChan != null ? g.yChan : "");
                ArrayNode xs = JSON.createArrayNode(); for (double v : g.xs) xs.add(v);
                ArrayNode ys = JSON.createArrayNode(); for (double v : g.ys) ys.add(v);
                gn.set("xs", xs); gn.set("ys", ys);
                polygons.add(gn);
            }
            a.set("gate_polygons", polygons);
        }
        String pop = target != null ? target.name() : "All Events";
        statusLabel.setText("Binning by " + timeCombo.getValue() + " (" + pop + ")…");
        ctx.jobs().run(ctx.bridge().command("run_kinetic", a), this::showResult);
    }

    private void showResult(JsonNode result) {
        chart.clearSeries();
        int gi = 0;
        for (JsonNode g : result.path("groups")) {
            Color col = PALETTE[gi % PALETTE.length];
            double[] times = toDoubles(g.path("times"));
            double[] mfi   = toDoubles(g.path("mfi"));
            double[] sd    = toDoubles(g.path("sd"));
            double[] lo = new double[mfi.length], hi = new double[mfi.length];
            for (int i = 0; i < mfi.length; i++) {
                lo[i] = Math.max(0, mfi[i] - sd[i]);
                hi[i] = mfi[i] + sd[i];
            }
            String gname = g.path("name").asText();
            chart.setX(times);
            chart.addBandSeries(gname + " ±SD", lo, hi, col);
            chart.addSeries(gname, mfi, col, AnalysisChart.Kind.LINE);
            gi++;
        }
        chart.setTitle("Kinetic — " + result.path("channel").asText());
        chart.setAxisLabels(result.path("time_channel").asText(), "Median MFI");
        chart.refresh();
        int nGroups = result.path("groups").size();
        statusLabel.setText(String.format(
                "%s vs %s — %d group(s), %d samples, %d bins.",
                result.path("channel").asText(), result.path("time_channel").asText(),
                nGroups, result.path("n_samples").asInt(), result.path("bins").asInt()));
        ctx.auditLog().add(AuditLog.Type.ANALYSIS, channelCombo.getValue(),
                String.format("Kinetic: %s vs %s, %d groups", result.path("channel").asText(),
                        result.path("time_channel").asText(), nGroups));
        copyButton.setDisable(false); exportButton.setDisable(false);
    }

    @FXML private void onCopy() {
        if (ctx == null) return;
        WritableImage img = chart.exportImage(ctx.settings());
        if (img == null) return;
        ClipboardContent cc = new ClipboardContent(); cc.putImage(img);
        Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText("Copied to clipboard.");
    }

    @FXML private void onExport() {
        if (ctx == null) return;
        FileChooser fc = new FileChooser();
        fc.setTitle("Export Kinetic Plot");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("PNG Image", "*.png"));
        File f = fc.showSaveDialog(chart.getScene().getWindow());
        if (f == null) return;
        try {
            WritableImage img = chart.exportImage(ctx.settings());
            BufferedImage bi = javafx.embed.swing.SwingFXUtils.fromFXImage(img, null);
            ImageIO.write(bi, "png", f);
            statusLabel.setText("Saved: " + f.getName());
        } catch (Exception e) {
            statusLabel.setText("Export failed: " + e.getMessage());
        }
    }

    private static double[] toDoubles(JsonNode arr) {
        double[] out = new double[arr.size()];
        for (int i = 0; i < arr.size(); i++) out[i] = arr.get(i).asDouble();
        return out;
    }

    private void setDisabled(boolean d) {
        channelCombo.setDisable(d);
        timeCombo.setDisable(d);
        binsCombo.setDisable(d);
        gateCombo.setDisable(d);
        refreshButton.setDisable(d);
        runButton.setDisable(d);
    }
}
