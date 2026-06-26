package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.beans.property.SimpleStringProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.cell.ComboBoxTableCell;
import javafx.scene.layout.FlowPane;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.stage.Stage;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;
import java.util.regex.Pattern;

/**
 * FlowJo-style Compensation Wizard. Lists the loaded control files, auto-matches each single-stain
 * control to its brightest detector (and guesses the unstained universal negative from the file
 * name), lets the user override roles/detectors, then drives the engine's
 * {@code compute_spillover_from_controls}: a size-cleanup gate + per-control positive/negative
 * split (shown as separation histograms) → a Bagwell-Adams spillover matrix the user can hand back
 * to the Compensation editor.
 */
public class CompWizardController {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final Pattern UNSTAINED = Pattern.compile("unstain|\\bblank\\b|no.?stain|^\\s*un\\b", Pattern.CASE_INSENSITIVE);
    // a single-stain control's file name usually carries a fluorophore token
    private static final Pattern FLUOR = Pattern.compile(
            "FITC|PerCP|PE.?Cy7|PE.?CF594|\\bPE\\b|APC.?Cy7|APC.?H7|\\bAPC\\b|AF700|AL700|A700|" +
            "Alexa.?Fluor.?700|BV421|BV510|BV605|BV650|BV711|BV786|BUV395|BUV737|Pacific.?Blue|" +
            "eFluor|PerCP.?Cy5|7.?AAD|Zombie|DAPI|\\bPI\\b", Pattern.CASE_INSENSITIVE);
    // these are NOT single-stain comp controls even if a fluorophore appears in the name
    private static final Pattern NOT_SINGLE = Pattern.compile(
            "FMO|multi.?stain|cocktail|full.?stain|\\bG1\\b|\\bG2\\b|\\bS\\b.?phase", Pattern.CASE_INSENSITIVE);

    static final String ROLE_SINGLE = "Single stain";
    static final String ROLE_UNSTAINED = "Unstained";
    static final String ROLE_IGNORE = "Ignore";
    static final String AUTO = "(auto)";

    @FXML private ComboBox<String> fscCombo, sscCombo;
    @FXML private Button computeButton, useButton, closeButton;
    @FXML private TableView<Assignment> assignTable;
    @FXML private TableColumn<Assignment, String> sampleCol, roleCol, detectorCol;
    @FXML private FlowPane histBox;
    @FXML private CompMatrixView wizardHeatmap;
    @FXML private Label statusLabel;

    private AppContext ctx;
    private Stage stage;
    private Consumer<JsonNode> onUse;
    private JsonNode lastResult;

    private final ObservableList<Assignment> assignments = FXCollections.observableArrayList();
    private final List<String> fluorChannels = new ArrayList<>();
    // per-control positive/negative threshold overrides (arcsinh space), set by dragging a histogram split
    private final Map<String, Double> thresholdOverrides = new HashMap<>();

    /** One control file's role + detector assignment (editable in the wizard table). */
    public static final class Assignment {
        private final String sample;
        private final SimpleStringProperty role;
        private final SimpleStringProperty detector;
        Assignment(String sample, String role, String detector) {
            this.sample = sample;
            this.role = new SimpleStringProperty(role);
            this.detector = new SimpleStringProperty(detector);
        }
        public String getSample() { return sample; }
        public SimpleStringProperty roleProperty() { return role; }
        public SimpleStringProperty detectorProperty() { return detector; }
    }

    /** Open the wizard; {@code onUse} receives the compute result when the user accepts the matrix. */
    public static void open(AppContext ctx, Consumer<JsonNode> onUse) {
        try {
            FXMLLoader loader = new FXMLLoader(
                    CompWizardController.class.getResource("/org/streamflow/ui/comp-wizard.fxml"));
            VBox root = loader.load();
            CompWizardController c = loader.getController();
            c.ctx = ctx;
            c.onUse = onUse;
            Scene scene = new Scene(root);
            scene.getStylesheets().add(CompWizardController.class
                    .getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
            Stage stage = new Stage();
            stage.setTitle("StreamFLOW — Compensation Wizard");
            stage.setScene(scene);
            AppIcons.apply(stage);
            c.stage = stage;
            c.load();
            stage.show();
        } catch (Exception e) {
            throw new RuntimeException("Could not open compensation wizard: " + e.getMessage(), e);
        }
    }

    @FXML
    public void initialize() {
        assignTable.setItems(assignments);
        sampleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue().getSample()));
        roleCol.setCellValueFactory(c -> c.getValue().roleProperty());
        roleCol.setCellFactory(ComboBoxTableCell.forTableColumn(ROLE_SINGLE, ROLE_UNSTAINED, ROLE_IGNORE));
        detectorCol.setCellValueFactory(c -> c.getValue().detectorProperty());
        useButton.setDisable(true);
    }

    /** Fetch the loaded samples + fluorescence channels and seed the assignment table. */
    private void load() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_channels", null), r -> {
            List<String> samples = new ArrayList<>();
            r.path("samples").forEach(n -> samples.add(n.asText()));
            fluorChannels.clear();
            List<String> scatter = new ArrayList<>();
            for (JsonNode cn : r.path("channels")) {
                String ch = cn.asText();
                if (ch.matches("(?i).*(FSC|SSC|Time|Width|Event).*")) scatter.add(ch);
                else fluorChannels.add(ch);
            }
            // detector dropdown = (auto) + every fluorescence channel
            List<String> detectorOpts = new ArrayList<>();
            detectorOpts.add(AUTO);
            detectorOpts.addAll(fluorChannels);
            detectorCol.setCellFactory(ComboBoxTableCell.forTableColumn(
                    FXCollections.observableArrayList(detectorOpts)));

            // scatter pickers
            List<String> scatterOpts = new ArrayList<>(scatter);
            if (scatterOpts.isEmpty()) scatterOpts.addAll(fluorChannels);
            fscCombo.getItems().setAll(scatterOpts);
            sscCombo.getItems().setAll(scatterOpts);
            selectMatch(fscCombo, scatterOpts, "FSC");
            selectMatch(sscCombo, scatterOpts, "SSC");

            // Seed roles conservatively (the experiment may contain biological/FMO/cell-cycle files
            // that must NOT be treated as comp controls): default Ignore, promote only the obvious
            // universal negative (first unstained) and single-stain controls (fluorophore in the name,
            // unless it's an FMO / multistain / cell-cycle file).
            assignments.clear();
            boolean unstainedSeen = false;
            int nSingle = 0, nIgnore = 0;
            for (String s : samples) {
                String role;
                if (!unstainedSeen && UNSTAINED.matcher(s).find()) {
                    role = ROLE_UNSTAINED; unstainedSeen = true;
                } else if (!NOT_SINGLE.matcher(s).find() && FLUOR.matcher(s).find()) {
                    role = ROLE_SINGLE; nSingle++;
                } else {
                    role = ROLE_IGNORE; nIgnore++;
                }
                assignments.add(new Assignment(s, role, AUTO));
            }
            statusLabel.setText(String.format(
                    "%d file(s): %s%d single-stain, %d ignored. Adjust roles/detectors, then Compute matrix.",
                    samples.size(), unstainedSeen ? "1 unstained, " : "no unstained yet — set one, ",
                    nSingle, nIgnore));
        });
    }

    private static void selectMatch(ComboBox<String> combo, List<String> opts, String key) {
        String m = opts.stream().filter(o -> o.toUpperCase().contains(key)).findFirst()
                .orElse(opts.isEmpty() ? null : opts.get(0));
        if (m != null) combo.getSelectionModel().select(m);
    }

    @FXML
    private void onCompute() {
        if (ctx == null) return;
        ObjectNode args = JSON.createObjectNode();

        String unstained = null;
        ArrayNode controls = args.putArray("controls");
        for (Assignment a : assignments) {
            String role = a.roleProperty().get();
            if (ROLE_UNSTAINED.equals(role)) {
                unstained = a.getSample();
            } else if (ROLE_SINGLE.equals(role)) {
                ObjectNode ctrl = controls.addObject();
                ctrl.put("sample", a.getSample());
                String det = a.detectorProperty().get();
                if (det != null && !AUTO.equals(det)) ctrl.put("channel", det);
            }
            // ROLE_IGNORE → skip
        }
        if (controls.size() == 0) { statusLabel.setText("Mark at least one sample as a single-stain control."); return; }
        if (unstained != null) args.put("unstained", unstained);

        ObjectNode scatter = args.putObject("scatter");
        if (fscCombo.getValue() != null) scatter.put("x", fscCombo.getValue());
        if (sscCombo.getValue() != null) scatter.put("y", sscCombo.getValue());

        // carry any manual positive/negative split overrides (from dragging a histogram)
        if (!thresholdOverrides.isEmpty()) {
            ObjectNode ov = args.putObject("threshold_overrides");
            thresholdOverrides.forEach(ov::put);
        }

        statusLabel.setText("Gating controls and computing spillover…");
        ctx.jobs().run(ctx.bridge().command("compute_spillover_from_controls", args), this::showResults);
    }

    private void showResults(JsonNode result) {
        lastResult = result;
        List<String> channels = new ArrayList<>();
        result.path("channels").forEach(n -> channels.add(n.asText()));

        // computed matrix → heatmap preview
        JsonNode mat = result.path("matrix");
        double[][] m = new double[channels.size()][channels.size()];
        for (int i = 0; i < mat.size() && i < m.length; i++)
            for (int j = 0; j < mat.get(i).size() && j < m[i].length; j++)
                m[i][j] = mat.get(i).get(j).asDouble();
        wizardHeatmap.setMatrix(channels, m);

        // per-control separation histograms (negative vs positive split at the Otsu threshold)
        histBox.getChildren().clear();
        int warnings = 0;
        for (JsonNode c : result.path("controls")) {
            String ch = c.path("channel").asText();
            String sample = c.path("sample").asText();
            double thrT = c.path("threshold_t").asDouble();
            boolean ok = c.path("ok").asBoolean(true);
            if (!ok) warnings++;
            double[] x = arr(c.path("hist").path("x"));
            double[] counts = arr(c.path("hist").path("counts"));
            double[] neg = new double[counts.length], pos = new double[counts.length];
            for (int k = 0; k < counts.length; k++) {
                if (x[k] < thrT) neg[k] = counts[k]; else pos[k] = counts[k];
            }
            AnalysisChart chart = new AnalysisChart();
            chart.setPrefSize(250, 165);
            chart.setMinSize(250, 165);
            chart.setAxisLabels("arcsinh(" + ch + ")", "Count");
            chart.setLegendVisible(false);   // compact chart: legend would mask the bars
            chart.setX(x);
            chart.addSeries("Negative", neg, Color.web("#7A8AA0"), AnalysisChart.Kind.BARS);
            chart.addSeries("Positive", pos, Color.web("#2C7FB8"), AnalysisChart.Kind.BARS);
            chart.setTitle(ch + (ok ? "  ✓ (grey=neg, blue=pos)" : "  ⚠ (grey=neg, blue=pos)"));
            // draggable positive/negative split → override that control's threshold and recompute
            chart.setThreshold(thrT);
            chart.setOnThresholdChange(newThr -> {
                thresholdOverrides.put(sample, newThr);
                statusLabel.setText("Adjusted " + ch + " split — recomputing…");
                onCompute();
            });
            chart.refresh();

            String cap = String.format("%s — %.1f%% pos, %,d events%s",
                    sample, c.path("pct_pos").asDouble(),
                    c.path("n_pos").asInt(), ok ? "" : "  (weak separation)");
            Label capLbl = new Label(cap);
            capLbl.setWrapText(true);
            capLbl.setMaxWidth(250);
            capLbl.setStyle(ok ? "-fx-text-fill:#C7D6E8; -fx-font-size:10;"
                               : "-fx-text-fill:#F5A623; -fx-font-size:10;");
            VBox cell = new VBox(2, chart, capLbl);
            histBox.getChildren().add(cell);
        }

        useButton.setDisable(channels.isEmpty());
        statusLabel.setText(channels.size() + "×" + channels.size() + " spillover computed from "
                + result.path("controls").size() + " control(s)"
                + (warnings > 0 ? " — " + warnings + " with weak positive separation; review before using." : ".")
                + " Click 'Use this matrix' to load it into the Compensation editor.");
    }

    @FXML
    private void onUse() {
        if (lastResult != null && onUse != null) onUse.accept(lastResult);
        if (stage != null) stage.close();
    }

    @FXML
    private void onClose() { if (stage != null) stage.close(); }

    private static double[] arr(JsonNode n) {
        if (n == null || !n.isArray()) return new double[0];
        double[] a = new double[n.size()];
        for (int i = 0; i < n.size(); i++) a[i] = n.get(i).asDouble();
        return a;
    }
}
