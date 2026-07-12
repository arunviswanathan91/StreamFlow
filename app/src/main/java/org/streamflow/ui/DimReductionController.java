package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonBar;
import javafx.scene.control.ButtonType;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Dialog;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.ScrollPane;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.Spinner;
import javafx.scene.control.SpinnerValueFactory;
import javafx.scene.control.TextArea;
import javafx.scene.control.TextField;
import javafx.scene.control.ToggleButton;
import javafx.scene.image.WritableImage;
import javafx.scene.input.Clipboard;
import javafx.scene.input.ClipboardContent;
import javafx.scene.layout.FlowPane;
import javafx.scene.layout.GridPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.StackPane;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.stage.FileChooser;
import javafx.stage.Window;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * UMAP / t-SNE on a single POOLED embedding across samples.
 *
 * Why pooled: t-SNE and UMAP are stochastic and each run produces its own coordinate system, so an
 * embedding computed per file is not comparable with any other file — the same cell type lands in a
 * different place every time. Every sample is therefore downsampled to the same N (seeded) and the
 * events are embedded together; you colour or facet by sample afterwards. Equal N also stops a large
 * file from dominating the map.
 *
 * The plot is a {@link CytoPlot}, which supplies a SQUARE (undistorted) plot area, gating, light mode
 * and DPI-scaled export. The previous bespoke canvas mapped the x-range to plot width and the y-range
 * to plot height independently, stretching an isotropic embedding and misrepresenting inter-cluster
 * distance.
 *
 * A polygon drawn on an island is mapped back through the embedding's {@code SampleIdx}/{@code
 * RowIndex} columns to the original events and becomes a real population, reusing the index-defined
 * population machinery ({@link CytoPlot#isIndexGate}).
 *
 * Every automated step (debris cleanup, positivity threshold, marker exclusion) is a default the user
 * can edit or switch off.
 */
public class DimReductionController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final String MODE_DISCOVERY = "Discovery (all samples)";
    private static final String MODE_TARGETED  = "Targeted (one population)";
    private static final String MODE_TAGGED    = "Marker-defined (tags)";
    private static final String COLOR_DENSITY  = "Density (no marker)";
    private static final String COLOR_SAMPLE   = "Sample";
    private static final String COLOR_RULES    = "Marker-defined populations";
    private static final String COLOR_EXISTING = "Existing gated populations";
    /** A group whose samples are excluded from the embedding entirely (controls, spare tubes). */
    static final String GROUP_IGNORE = "Ignore";

    /** Filename markers of compensation/gating controls, which must never enter a biology embedding. */
    private static final java.util.regex.Pattern CONTROL_RE = java.util.regex.Pattern.compile(
            "(?i).*(unstain|fmo|single[ _-]?stain|comp[ _-]?bead|isotype|blank|zombie|viability).*");

    /** Distinct, colour-blind-safe-ish palette for population colouring. */
    private static final Color[] POP_PALETTE = {
            Color.web("#E4572E"), Color.web("#17BEBB"), Color.web("#FFC914"), Color.web("#2E282A"),
            Color.web("#76B041"), Color.web("#8367C7"), Color.web("#E36397"), Color.web("#3E92CC"),
            Color.web("#B84A62"), Color.web("#5B8C5A"), Color.web("#C17817"), Color.web("#4C6085")};

    @FXML private ComboBox<String> modeCombo, populationCombo, methodCombo, colorByCombo, transformCombo;
    @FXML private TextField cofactorField;
    @FXML private ComboBox<String> fscCombo, sscCombo, liveDeadCombo, controlCombo;
    @FXML private Label populationLabel, statusLabel;
    @FXML private Spinner<Integer> eventsSpinner, seedSpinner;
    @FXML private Button runButton, refreshButton, copyButton, exportButton, settingsButton,
                         addPopButton, autoThresholdButton, renameGateButton, removeGateButton,
                         manualThresholdButton, debrisInfoButton, tagsButton, thresholdsButton,
                         addRuleButton, removeRuleButton, applyRulesButton, suggestButton,
                         approveButton, rejectButton, nameSuggestionButton, inspectButton;
    @FXML private ToggleButton gateToggle, gridToggle, lightBgToggle, groupToggle;
    @FXML private Button groupsButton, coloursButton, panelsButton;

    /** Panel keys the user unticked in "Show…"; empty = show everything. Keys are "All samples",
     *  group names (By group on) or sample names (By group off). */
    private final java.util.Set<String> hiddenPanels = new java.util.HashSet<>();
    private static final String PANEL_ALL = "All samples";
    @FXML private ListView<String> markerList;
    @FXML private ListView<CytoPlot.Gate> gateList;
    @FXML private CheckBox debrisCheck, positivityCheck;
    @FXML private TextField thresholdField;
    @FXML private StackPane plotHost;
    @FXML private ScrollPane gridScroll;
    @FXML private FlowPane previewGrid;
    @FXML private VBox taggedPanel;
    @FXML private ListView<EmbeddingRules.Rule> ruleList;
    @FXML private ListView<Suggestion> suggestionList;
    @FXML private Spinner<Integer> kSpinner;
    @FXML private ComboBox<String> namingCombo, clusterMethodCombo, scaleCombo;
    @FXML private CheckBox qcCheck;
    @FXML private ScrollPane legendScroll;
    @FXML private VBox legendPane;

    private static final String NAME_THRESHOLD = "Thresholds";
    private static final String NAME_ZSCORE    = "Relative: standardized (z-score)";
    private static final String NAME_OTSU       = "Relative: data split (Otsu on events)";
    private static final String NAME_MEDIAN     = "Relative: above global median";

    /** Population names the user has UNticked in the external legend; those are hidden (drawn grey). */
    private final java.util.Set<String> hiddenPops = new java.util.HashSet<>();

    /**
     * A candidate population: a row set with a name and a dashed outline. The row set — never the
     * outline — defines membership. Approving turns the row set into a real population and the outline
     * from dashed into solid.
     */
    static final class Suggestion {
        String name;
        final boolean[] mask;
        final int count;
        final CytoPlot.Gate outline;   // may be null when the events are too few to bound an area
        final String provenance;       // how it was derived, for the audit log
        /** Index into the colour labels painted on the plot. NOT the list position: an empty cluster
         *  is never listed, so the two drift apart the moment FlowSOM returns one. */
        final int paintIndex;
        Suggestion(String name, boolean[] mask, int count, CytoPlot.Gate outline, String provenance, int paintIndex) {
            this.name = name; this.mask = mask; this.count = count;
            this.outline = outline; this.provenance = provenance; this.paintIndex = paintIndex;
        }
        @Override public String toString() { return name + "  (" + String.format("%,d", count) + ")"; }
    }

    private AppContext ctx;
    private final CytoPlot plot = new CytoPlot();
    /** Side-by-side group panels shown in the main area instead of {@link #plot} when "By group" is on. */
    private final HBox comparisonBox = new HBox(10);
    private final List<CytoPlot> comparePlots = new ArrayList<>();
    private final List<String> compareCaptions = new ArrayList<>();
    /** Applied to the main plot and every facet whenever the shared export settings change. */
    private final Runnable exportFormatListener = this::applyExportFormat;

    private EventData embedding;            // columns: [m1, m2, features…, SampleIdx, RowIndex]
    private List<String> embSamples = List.of();
    private List<String> embFeatures = List.of();
    private String axisX = "UMAP 1", axisY = "UMAP 2";
    private double xLo, xHi, yLo, yHi;      // shared axis limits, so facets are comparable
    private int gateSeq;                    // names drawn gates Pop 1, Pop 2, …
    private int focusedSample = -1;         // -1 = pooled view; otherwise an index into embSamples

    private final javafx.collections.ObservableList<EmbeddingRules.Rule> rules = FXCollections.observableArrayList();
    private final javafx.collections.ObservableList<Suggestion> suggestions = FXCollections.observableArrayList();
    /** Per-channel positivity thresholds used by the rules. Every one is user-set or user-confirmed. */
    private final Map<String, Double> markerThresholds = new LinkedHashMap<>();

    @FXML
    public void initialize() {
        modeCombo.setItems(FXCollections.observableArrayList(MODE_DISCOVERY, MODE_TARGETED, MODE_TAGGED));
        modeCombo.getSelectionModel().select(MODE_DISCOVERY);
        modeCombo.setOnAction(e -> updateModeVisibility());

        kSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(2, 40, 8, 1));
        namingCombo.setItems(FXCollections.observableArrayList(
                NAME_THRESHOLD, NAME_ZSCORE, NAME_OTSU, NAME_MEDIAN));
        namingCombo.getSelectionModel().select(NAME_ZSCORE);   // threshold-free default: captures whole regions

        clusterMethodCombo.setItems(FXCollections.observableArrayList(
                "flowsom", "phenograph", "bayesian", "flowgrid", "kmeans"));
        clusterMethodCombo.getSelectionModel().select("flowsom");
        scaleCombo.setItems(FXCollections.observableArrayList("zscore", "minmax", "none"));
        scaleCombo.getSelectionModel().select("zscore");
        // PhenoGraph, Bayesian (DP) and FlowGrid choose their own cluster count; k only applies to
        // FlowSOM / k-means, so grey the spinner out for the auto-count methods.
        kSpinner.disableProperty().bind(javafx.beans.binding.Bindings.createBooleanBinding(
                () -> { String m = clusterMethodCombo.getValue();
                        return "phenograph".equals(m) || "bayesian".equals(m) || "flowgrid".equals(m); },
                clusterMethodCombo.getSelectionModel().selectedItemProperty()));
        ruleList.setItems(rules);
        suggestionList.setItems(suggestions);
        suggestionList.getSelectionModel().selectedItemProperty().addListener((o, a, s) -> highlightSuggestion(s));

        methodCombo.setItems(FXCollections.observableArrayList("umap", "tsne"));
        methodCombo.getSelectionModel().select("umap");

        transformCombo.setItems(FXCollections.observableArrayList("logicle", "arcsinh", "log", "none"));
        transformCombo.getSelectionModel().select("logicle");
        // Cofactor only applies to arcsinh; grey it out otherwise so it doesn't imply it does something.
        cofactorField.disableProperty().bind(transformCombo.getSelectionModel()
                .selectedItemProperty().isNotEqualTo("arcsinh"));

        eventsSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(200, 100000, 5000, 500));
        seedSpinner.setValueFactory(new SpinnerValueFactory.IntegerSpinnerValueFactory(0, 999999, 42, 1));

        markerList.getSelectionModel().setSelectionMode(SelectionMode.MULTIPLE);

        plotHost.getChildren().add(plot);
        plot.prefWidthProperty().bind(plotHost.widthProperty());
        plot.prefHeightProperty().bind(plotHost.heightProperty());
        plot.setPlotType("pseudocolor");
        comparisonBox.setVisible(false);
        comparisonBox.setManaged(false);
        comparisonBox.setAlignment(javafx.geometry.Pos.CENTER);
        plotHost.getChildren().add(comparisonBox);

        // CytoPlot.finish() reports the new gate but does NOT keep it — every host must call addGate,
        // as GraphWindowController does. Without this the gate is drawn, reported, then dropped on the
        // next repaint, which is exactly the "it becomes a gate but disappears" bug.
        plot.setOnGateDrawn(g -> {
            g.name = "Pop " + (++gateSeq);
            plot.addGate(g);
            gateToggle.setSelected(false);
            refreshGateList();
            gateList.getSelectionModel().select(g);
            statusLabel.setText("Gate '" + g.name + "' drawn. Draw more, or add it as a population.");
        });

        gateList.setCellFactory(lv -> new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(CytoPlot.Gate g, boolean empty) {
                super.updateItem(g, empty);
                setText(empty || g == null ? null : g.name + "  (" + String.format("%,d", countIn(g)) + ")");
            }
        });
        gateList.getSelectionModel().selectedItemProperty().addListener((o, a, g) -> {
            boolean has = g != null;
            addPopButton.setDisable(!has);
            renameGateButton.setDisable(!has);
            removeGateButton.setDisable(!has);
            if (has) plot.selectGate(g);
        });

        gateToggle.selectedProperty().addListener((o, a, on) -> plot.setTool(on ? "Polygon" : "None"));
        colorByCombo.setOnAction(e -> applyColorBy());

        updateModeVisibility();
        setHasEmbedding(false);
    }

    /**
     * Render a channel list/combo with its marker tag ("CD4 (BV421-A)") while keeping the RAW channel
     * name as the value. Tags live in the app-wide {@link ChannelAliases}, the same store the Graph
     * Window edits — this module simply never displayed them.
     */
    private javafx.scene.control.ListCell<String> aliasCell() {
        return new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(String ch, boolean empty) {
                super.updateItem(ch, empty);
                setText(empty || ch == null ? null : ctx.aliases().label(ch));
            }
        };
    }

    private void showAliases(ComboBox<String> cb) {
        cb.setCellFactory(lv -> aliasCell());
        cb.setButtonCell(aliasCell());
    }

    @Override public void init(AppContext context) {
        this.ctx = context;
        plot.setChannelLabeler(ch -> ctx.aliases().label(ch));
        markerList.setCellFactory(lv -> aliasCell());
        showAliases(colorByCombo);
        showAliases(fscCombo);
        showAliases(sscCombo);
        showAliases(liveDeadCombo);
        ctx.workspace().sampleNames().addListener(
                (javafx.collections.ListChangeListener<String>) c -> refreshFromWorkspace());
        // Without this the export gear mutates AppSettings and nothing in this module ever listens,
        // which is why DPI / point size / font changes had no visible effect here.
        ctx.settings().addChangeListener(exportFormatListener);
        applyExportFormat();
        refreshFromWorkspace();
    }

    /** Push the shared export format (point size, fonts) onto the main plot and every facet. */
    private void applyExportFormat() {
        if (ctx == null) return;
        for (CytoPlot p : allPlots()) {
            p.setPointRadius(ctx.settings().exportPointSize());
            p.setAxisFontSize(ctx.settings().exportAxisFontSize());
            p.setLabelFontSize(ctx.settings().exportFontSize());
        }
    }

    private List<CytoPlot> allPlots() {
        List<CytoPlot> out = new ArrayList<>();
        out.add(plot);
        out.addAll(comparePlots);
        for (javafx.scene.Node n : previewGrid.getChildren())
            if (n instanceof VBox v && !v.getChildren().isEmpty() && v.getChildren().get(0) instanceof CytoPlot p)
                out.add(p);
        return out;
    }

    @FXML
    private void onToggleLightBg() {
        boolean on = lightBgToggle.isSelected();
        for (CytoPlot p : allPlots()) p.setLightMode(on);
    }

    private int countIn(CytoPlot.Gate g) {
        if (embedding == null || g == null) return 0;
        int n = 0;
        for (boolean b : CytoPlot.mask(embedding, g)) if (b) n++;
        return n;
    }

    private void refreshGateList() {
        gateList.setItems(FXCollections.observableArrayList(plot.gates()));
    }

    @FXML
    private void onRenameGate() {
        CytoPlot.Gate g = gateList.getSelectionModel().getSelectedItem();
        if (g == null) return;
        javafx.scene.control.TextInputDialog dlg = new javafx.scene.control.TextInputDialog(g.name);
        dlg.setTitle("Rename gate");
        dlg.setHeaderText(null);
        dlg.setContentText("Name:");
        AppIcons.theme(dlg, plotHost.getScene() == null ? null : plotHost.getScene().getWindow());
        dlg.showAndWait().filter(s -> !s.isBlank()).ifPresent(s -> {
            g.name = s.trim();
            plot.refresh();
            refreshGateList();
            gateList.getSelectionModel().select(g);
        });
    }

    @FXML
    private void onRemoveGate() {
        CytoPlot.Gate g = gateList.getSelectionModel().getSelectedItem();
        if (g == null) return;
        plot.gates().remove(g);
        plot.refresh();
        refreshGateList();
        statusLabel.setText("Removed '" + g.name + "' from the map. Populations already added are unaffected.");
    }

    @FXML private void onRefresh() { refreshFromWorkspace(); }

    @Override
    public void refreshFromWorkspace() {
        if (ctx == null) return;
        List<String> samples = new ArrayList<>(ctx.workspace().sampleNames());
        List<String> channels = new ArrayList<>(ctx.workspace().channelNames());
        if (samples.isEmpty() || channels.isEmpty()) {
            statusLabel.setText("Load FCS first.");
            runButton.setDisable(true);
            return;
        }
        runButton.setDisable(false);

        // Markers: all channels, with scatter/Time de-selected by default — a proposal, not a rule.
        markerList.setItems(FXCollections.observableArrayList(channels));
        markerList.getSelectionModel().clearSelection();
        for (int i = 0; i < channels.size(); i++)
            if (!isScatterOrTime(channels.get(i))) markerList.getSelectionModel().select(i);

        fillCombo(fscCombo, channels, firstMatching(channels, "FSC-A", "FSC-H"));
        fillCombo(sscCombo, channels, firstMatching(channels, "SSC-A", "SSC-H"));
        // Live/Dead has NO sensible fallback. Defaulting to the first channel silently offered to gate
        // on FSC-A as if it were a viability dye. Leave it empty and let the run refuse instead.
        String ld = firstMatching(channels, "Zombie", "Live", "Dead", "7-AAD", "Viability", "Aqua", "eFluor");
        liveDeadCombo.setItems(FXCollections.observableArrayList(channels));
        if (ld != null) liveDeadCombo.getSelectionModel().select(ld);
        else liveDeadCombo.getSelectionModel().clearSelection();
        fillCombo(controlCombo, samples, samples.get(0));
        populationCombo.setItems(FXCollections.observableArrayList(populationNames()));
        if (!populationCombo.getItems().isEmpty()) populationCombo.getSelectionModel().selectFirst();

        // A workspace that was saved with a map reopens showing that map, not a blank plot.
        if (embedding == null && restoreEmbedding()) return;
        statusLabel.setText(samples.size() + " sample(s) will be pooled with an equal number of events each.");
    }

    private static boolean isScatterOrTime(String c) {
        return c != null && c.matches("(?i).*(FSC|SSC|Time|Width|Event).*");
    }

    /** First channel whose name contains one of the needles, or null. Callers decide the fallback. */
    private static String firstMatching(List<String> pool, String... needles) {
        for (String n : needles)
            for (String c : pool)
                if (c.toLowerCase().contains(n.toLowerCase())) return c;
        return null;
    }

    private static void fillCombo(ComboBox<String> cb, List<String> items, String select) {
        cb.setItems(FXCollections.observableArrayList(items));
        if (select != null && items.contains(select)) cb.getSelectionModel().select(select);
        else if (!items.isEmpty()) cb.getSelectionModel().selectFirst();
    }

    /** Population names present in any sample's gating tree (targeted mode selects by name). */
    private List<String> populationNames() {
        java.util.LinkedHashSet<String> names = new java.util.LinkedHashSet<>();
        for (String s : ctx.workspace().sampleNames()) {
            if (!ctx.workspace().hasTree(s)) continue;
            for (PopNode n : ctx.workspace().treeFor(s).selfAndDescendants())
                if (!n.isRoot()) names.add(n.name());
        }
        return new ArrayList<>(names);
    }

    private void updateModeVisibility() {
        boolean targeted = MODE_TARGETED.equals(modeCombo.getValue());
        boolean tagged = MODE_TAGGED.equals(modeCombo.getValue());
        populationCombo.setVisible(targeted); populationCombo.setManaged(targeted);
        populationLabel.setVisible(targeted); populationLabel.setManaged(targeted);
        taggedPanel.setVisible(tagged); taggedPanel.setManaged(tagged);
    }

    private void setHasEmbedding(boolean has) {
        copyButton.setDisable(!has);
        exportButton.setDisable(!has);
        gateToggle.setDisable(!has);
        gridToggle.setDisable(!has);
        colorByCombo.setDisable(!has);
        lightBgToggle.setDisable(!has);
        renameGateButton.setDisable(true);
        removeGateButton.setDisable(true);
    }

    // ---- positivity ---------------------------------------------------------

    /** Otsu split from the chosen control. Written into an editable field — never applied silently. */
    @FXML
    private void onAutoThreshold() {
        if (ctx == null || liveDeadCombo.getValue() == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", controlCombo.getValue());
        args.putArray("channels").add(liveDeadCombo.getValue());
        statusLabel.setText("Computing positivity threshold from " + controlCombo.getValue() + "…");
        ctx.jobs().run(ctx.bridge().command("positivity_thresholds", args), r -> {
            JsonNode c = r.path("channels").path(0);
            if (!c.path("ok").asBoolean(false)) {
                statusLabel.setText("Threshold failed: " + c.path("error").asText("?"));
                return;
            }
            thresholdField.setText(String.format("%.1f", c.path("threshold").asDouble()));
            statusLabel.setText(String.format(
                    "Otsu threshold %.1f — %.1f%% of %s events are positive. Edit the value if needed.",
                    c.path("threshold").asDouble(), c.path("pct_positive").asDouble(), controlCombo.getValue()));
        });
    }

    // ---- run ----------------------------------------------------------------

    @FXML
    private void onRun() {
        if (ctx == null) return;
        List<String> features = new ArrayList<>(markerList.getSelectionModel().getSelectedItems());
        if (features.isEmpty()) { statusLabel.setText("Select at least one marker."); return; }

        // Samples put in the "Ignore" group never enter the embedding — that is what Ignore means.
        List<String> samples = new ArrayList<>();
        int ignored = 0;
        for (String s : ctx.workspace().sampleNames()) {
            if (GROUP_IGNORE.equals(ctx.workspace().groupOf(s))) { ignored++; continue; }
            samples.add(s);
        }
        if (samples.isEmpty()) {
            statusLabel.setText("Every sample is set to Ignore — nothing left to embed.");
            return;
        }
        final int nIgnored = ignored;
        ObjectNode args = JSON.createObjectNode();
        ArrayNode sarr = args.putArray("samples");

        if (MODE_TARGETED.equals(modeCombo.getValue())) {
            String pop = populationCombo.getValue();
            if (pop == null) { statusLabel.setText("Select a population."); return; }
            ObjectNode idx = args.putObject("indices");
            List<String> used = new ArrayList<>(), skipped = new ArrayList<>();
            for (String s : samples) {
                int[] rows = indicesForPopulation(s, pop);
                if (rows == null) { skipped.add(s); continue; }
                ArrayNode a = idx.putArray(s);
                for (int r : rows) a.add(r);
                used.add(s);
            }
            if (used.isEmpty()) {
                statusLabel.setText("No sample has '" + pop + "' with events loaded. Open those samples once, then retry.");
                return;
            }
            used.forEach(sarr::add);
            if (!skipped.isEmpty())
                statusLabel.setText("Skipping " + skipped.size() + " sample(s) without loaded events.");
        } else {
            samples.forEach(sarr::add);
        }

        args.put("method", methodCombo.getValue());
        args.put("n_per_sample", eventsSpinner.getValue());
        args.put("seed", seedSpinner.getValue());
        // The transform chosen here IS the app-wide setting (no separate Transformation tab); pass it
        // explicitly so this run uses exactly what the user sees, and persist it globally for Clustering.
        ObjectNode xf = args.putObject("transform");
        xf.put("type", transformCombo.getValue());
        xf.put("cofactor", parseOrDefault(cofactorField.getText(), 150.0));
        applyGlobalTransform();
        ArrayNode farr = args.putArray("features");
        features.forEach(farr::add);

        ObjectNode cleanup = args.putObject("cleanup");
        cleanup.put("debris", debrisCheck.isSelected());
        cleanup.put("qc", qcCheck.isSelected());
        if (fscCombo.getValue() != null) cleanup.put("fsc", fscCombo.getValue());
        if (sscCombo.getValue() != null) cleanup.put("ssc", sscCombo.getValue());

        if (positivityCheck.isSelected()) {
            if (liveDeadCombo.getValue() == null) {
                statusLabel.setText("Choose the Live/Dead channel, or untick \"Drop Live/Dead-positive events\". "
                        + "There is no safe default — gating on the wrong channel would delete real cells.");
                return;
            }
            if (thresholdField.getText().isBlank()) {
                statusLabel.setText("Set a Live/Dead threshold (or click Auto), or untick the option.");
                return;
            }
            try {
                ObjectNode p = args.putArray("positivity").addObject();
                p.put("channel", liveDeadCombo.getValue());
                p.put("threshold", Double.parseDouble(thresholdField.getText().trim()));
                p.put("drop", "positive");
            } catch (NumberFormatException e) {
                statusLabel.setText("Threshold must be a number (or click Auto).");
                return;
            }
        }

        runButton.setDisable(true);
        statusLabel.setText("Running " + methodCombo.getValue().toUpperCase() + " on " + samples.size()
                + " sample(s)" + (nIgnored > 0 ? ", ignoring " + nIgnored + " control(s)" : "") + "…");
        ctx.jobs().run(ctx.bridge().command("run_dimredux", args), this::loadEmbedding);
    }

    /** Row indices of {@code popName} within {@code sample}'s cached events, or null if unavailable. */
    private int[] indicesForPopulation(String sample, String popName) {
        EventData root = ctx.workspace().data(sample);
        if (root == null || !ctx.workspace().hasTree(sample)) return null;
        PopNode target = null;
        for (PopNode n : ctx.workspace().treeFor(sample).selfAndDescendants())
            if (!n.isRoot() && popName.equals(n.name())) { target = n; break; }
        if (target == null) return null;

        boolean[] keep = new boolean[root.rows()];
        java.util.Arrays.fill(keep, true);
        for (CytoPlot.Gate g : target.chain()) {
            boolean[] m = CytoPlot.mask(root, g);
            for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && m[i];
        }
        int n = 0; for (boolean b : keep) if (b) n++;
        if (n == 0) return null;
        int[] out = new int[n];
        for (int i = 0, k = 0; i < keep.length; i++) if (keep[i]) out[k++] = i;
        return out;
    }

    private void loadEmbedding(JsonNode r) {
        runButton.setDisable(false);
        try {
            List<String> chans = new ArrayList<>();
            r.path("channels").forEach(n -> chans.add(n.asText()));
            Path bin = Paths.get(r.path("file").asText());
            embedding = EventData.read(bin, chans, r.path("rows").asInt(), r.path("cols").asInt());
            try { Files.deleteIfExists(bin); } catch (Exception ignored) {}

            embSamples = new ArrayList<>(); r.path("samples").forEach(n -> embSamples.add(n.asText()));
            embFeatures = new ArrayList<>(); r.path("features").forEach(n -> embFeatures.add(n.asText()));
            axisX = chans.get(0); axisY = chans.get(1);

            computeSharedRanges();
            focusedSample = -1;
            plot.gates().clear();           // a new embedding has new coordinates; old outlines are meaningless
            gateSeq = 0;
            plot.setData(embedding);
            plot.setAxes(axisX, axisY);
            applyLockedRange(plot);
            plot.clearColorByChannel();
            applyExportFormat();
            plot.setLightMode(lightBgToggle.isSelected());

            List<String> colorItems = new ArrayList<>();
            colorItems.add(COLOR_DENSITY);
            colorItems.addAll(embFeatures);
            if (embSamples.size() > 1) colorItems.add(COLOR_SAMPLE);
            colorItems.add(COLOR_RULES);
            colorItems.add(COLOR_EXISTING);
            colorByCombo.setItems(FXCollections.observableArrayList(colorItems));
            colorByCombo.getSelectionModel().select(COLOR_DENSITY);
            suggestions.clear();

            refreshGateList();
            addPopButton.setDisable(true);
            setHasEmbedding(true);

            // Always name the transform: the same markers under logicle vs arcsinh give different maps,
            // and the Transformation tab is what chooses it.
            String msg = String.format("%s — %,d events from %d sample(s), %d markers, %s transform, seed %d.",
                    r.path("method").asText().toUpperCase(), embedding.rows(), embSamples.size(),
                    embFeatures.size(), r.path("transform").asText("arcsinh"), r.path("seed").asInt());
            statusLabel.setText(msg);
            ctx.auditLog().add(AuditLog.Type.ANALYSIS, "", msg + " (pooled, equal N per sample)");
            ctx.workspace().setEmbeddingSnapshot(snapshotOf(r));
            rebuildMainView();
            if (gridToggle.isSelected()) buildPreviewGrid();
        } catch (Exception e) {
            setHasEmbedding(false);
            statusLabel.setText("Could not read embedding: " + e.getMessage());
        }
    }

    // ---- saving and restoring the map ----------------------------------------

    /**
     * Serialise the embedding itself — coordinates and all — into the workspace.
     *
     * We store the numbers, not the arguments that produced them. Re-running t-SNE on reopen would
     * cost minutes and, more importantly, would hand back a DIFFERENT layout: the algorithm is
     * stochastic, and any change to the data, the transform or the library version moves every island.
     * Gates drawn on the old coordinates would then sit over the wrong cells.
     */
    private ObjectNode snapshotOf(JsonNode runResult) {
        ObjectNode snap = JSON.createObjectNode();
        snap.put("method", runResult.path("method").asText());
        snap.put("seed", runResult.path("seed").asInt());
        snap.put("transform", runResult.path("transform").asText("arcsinh"));
        snap.put("rows", embedding.rows());
        snap.put("cols", embedding.channels().size());
        ArrayNode chans = snap.putArray("channels");
        embedding.channels().forEach(chans::add);
        ArrayNode samps = snap.putArray("samples");
        embSamples.forEach(samps::add);
        ArrayNode feats = snap.putArray("features");
        embFeatures.forEach(feats::add);
        ArrayNode data = snap.putArray("data");
        int cols = embedding.channels().size();
        for (int r = 0; r < embedding.rows(); r++)
            for (int c = 0; c < cols; c++) data.add(embedding.get(r, c));
        return snap;
    }

    /** Rebuild the plot from a snapshot saved in the workspace. Returns false if there is none. */
    private boolean restoreEmbedding() {
        JsonNode snap = ctx.workspace().embeddingSnapshot();
        if (snap == null || !snap.has("data")) return false;
        try {
            List<String> chans = new ArrayList<>();
            snap.path("channels").forEach(n -> chans.add(n.asText()));
            int rows = snap.path("rows").asInt(), cols = snap.path("cols").asInt();
            JsonNode data = snap.path("data");
            if (chans.size() != cols || data.size() != rows * cols) return false;

            float[] flat = new float[rows * cols];
            for (int i = 0; i < flat.length; i++) flat[i] = (float) data.get(i).asDouble();
            embedding = new EventData(flat, rows, cols, chans);

            embSamples = new ArrayList<>(); snap.path("samples").forEach(n -> embSamples.add(n.asText()));
            embFeatures = new ArrayList<>(); snap.path("features").forEach(n -> embFeatures.add(n.asText()));
            axisX = chans.get(0); axisY = chans.get(1);

            computeSharedRanges();
            focusedSample = -1;
            plot.gates().clear();
            gateSeq = 0;
            plot.setData(embedding);
            plot.setAxes(axisX, axisY);
            applyLockedRange(plot);
            plot.clearColorByChannel();
            plot.clearColorByPopulation();
            applyExportFormat();
            plot.setLightMode(lightBgToggle.isSelected());

            List<String> colorItems = new ArrayList<>();
            colorItems.add(COLOR_DENSITY);
            colorItems.addAll(embFeatures);
            if (embSamples.size() > 1) colorItems.add(COLOR_SAMPLE);
            colorItems.add(COLOR_RULES);
            colorItems.add(COLOR_EXISTING);
            colorByCombo.setItems(FXCollections.observableArrayList(colorItems));
            colorByCombo.getSelectionModel().select(COLOR_DENSITY);

            suggestions.clear();
            refreshGateList();
            setHasEmbedding(true);
            rebuildMainView();
            statusLabel.setText(String.format(
                    "Restored the saved %s map — %,d events, %d sample(s), %s transform, seed %d. "
                    + "Nothing was recomputed, so the coordinates are exactly the ones you gated on.",
                    snap.path("method").asText().toUpperCase(), embedding.rows(), embSamples.size(),
                    snap.path("transform").asText("arcsinh"), snap.path("seed").asInt()));
            return true;
        } catch (Exception e) {
            statusLabel.setText("Saved map could not be restored (" + e.getMessage() + "). Re-run to rebuild it.");
            return false;
        }
    }

    /** Shared axis limits so the main plot and every facet cover exactly the same coordinate space. */
    private void computeSharedRanges() {
        int cx = embedding.indexOf(axisX), cy = embedding.indexOf(axisY);
        xLo = yLo = Double.MAX_VALUE; xHi = yHi = -Double.MAX_VALUE;
        for (int i = 0; i < embedding.rows(); i++) {
            double x = embedding.get(i, cx), y = embedding.get(i, cy);
            xLo = Math.min(xLo, x); xHi = Math.max(xHi, x);
            yLo = Math.min(yLo, y); yHi = Math.max(yHi, y);
        }
        double padX = 0.04 * Math.max(1e-9, xHi - xLo), padY = 0.04 * Math.max(1e-9, yHi - yLo);
        xLo -= padX; xHi += padX; yLo -= padY; yHi += padY;
    }

    private void applyLockedRange(CytoPlot p) {
        p.setXMin(xLo); p.setXMax(xHi); p.setYMin(yLo); p.setYMax(yHi);
    }

    private void applyColorBy() {
        String v = colorByCombo.getValue();
        plot.clearColorByPopulation();
        plot.setPopulationLegendInside(true);   // restore default for non-population colourings
        popColoring = null;
        boolean pop = COLOR_RULES.equals(v) || COLOR_EXISTING.equals(v);
        if (!pop) { legendScroll.setVisible(false); legendScroll.setManaged(false); }
        if (embedding == null || v == null || COLOR_DENSITY.equals(v)) {
            plot.clearColorByChannel();
        } else if (COLOR_SAMPLE.equals(v)) {
            plot.setColorByChannel("SampleIdx", true);
        } else if (COLOR_RULES.equals(v)) {
            plot.clearColorByChannel();
            colorByRules();
        } else if (COLOR_EXISTING.equals(v)) {
            plot.clearColorByChannel();
            colorByExistingPopulations();
        } else {
            plot.setColorByChannel(v, false);
        }
        if (comparisonBox.isVisible()) rebuildMainView();   // panels carry their own colour-by
        if (gridToggle.isSelected()) buildPreviewGrid();
    }

    /** Population colouring over the POOLED embedding rows; panels take subsets of {@code labels}. */
    private record PopColoring(int[] labels, Color[] colors, String[] names) {}

    /** Cached so the main plot and every group panel paint the same labels without recomputing. */
    private PopColoring popColoring;

    /** Paint the marker-defined rules. First matching rule wins the pixel; counts stay per-rule. */
    private void colorByRules() {
        popColoring = null;
        if (rules.isEmpty()) { statusLabel.setText("No rules defined yet — add one first."); return; }
        int[] labels = EmbeddingRules.labels(embedding, rules, markerThresholds);
        String[] names = new String[rules.size()];
        Color[] colors = new Color[rules.size()];
        Map<String, Integer> counts = EmbeddingRules.counts(embedding, rules, markerThresholds);
        for (int i = 0; i < rules.size(); i++) {
            String n = rules.get(i).name();
            names[i] = n + " (" + String.format("%,d", counts.getOrDefault(n, 0)) + ")";
            colors[i] = popColorFor(n, i);
        }
        popColoring = new PopColoring(labels, colors, names);
        applyPopColoring();
        int unassigned = 0;
        for (int l : labels) if (l < 0) unassigned++;
        statusLabel.setText(String.format(
                "%d rule(s). %,d of %,d events match none and are drawn grey. An event matching two rules "
                + "is counted in both but painted the first rule's colour.",
                rules.size(), unassigned, embedding.rows()));
    }

    /**
     * Paint the cached {@link #popColoring} onto the main plot (or panels) and rebuild the external,
     * non-occluding legend. Populations the user has unticked ({@link #hiddenPops}) are drawn grey but
     * stay in the legend so they can be turned back on.
     */
    private void applyPopColoring() {
        if (popColoring == null) { legendScroll.setVisible(false); legendScroll.setManaged(false); return; }
        plot.setPopulationLegendInside(false);
        plot.setColorByPopulation(displayLabels(popColoring.labels()), popColoring.colors(), popColoring.names());
        buildLegend();
    }

    /** Copy of {@code raw} with hidden populations remapped to -1 (grey). */
    private int[] displayLabels(int[] raw) {
        if (popColoring == null || hiddenPops.isEmpty()) return raw;
        String[] names = popColoring.names();
        boolean[] hide = new boolean[names.length];
        for (int i = 0; i < names.length; i++) hide[i] = hiddenPops.contains(stripCount(names[i]));
        int[] out = raw.clone();
        for (int i = 0; i < out.length; i++)
            if (out[i] >= 0 && out[i] < hide.length && hide[out[i]]) out[i] = -1;
        return out;
    }

    private static String stripCount(String legendName) {
        return legendName.replaceAll("\\s*\\(\\d[\\d,]*\\)\\s*$", "");
    }

    /**
     * The legend, outside the plot so it never covers data, with a checkbox per population. Unticking
     * hides that population (its events go grey) everywhere — main plot, group panels and preview grid.
     * Clicking the name focuses that population (dims the rest).
     */
    private void buildLegend() {
        legendPane.getChildren().clear();
        if (popColoring == null) { legendScroll.setVisible(false); legendScroll.setManaged(false); return; }
        String[] names = popColoring.names();
        Color[] colors = popColoring.colors();

        Label title = new Label("Populations");
        title.getStyleClass().add("subtitle");
        legendPane.getChildren().add(title);

        for (int i = 0; i < names.length; i++) {
            final String key = stripCount(names[i]);
            CheckBox cb = new CheckBox(names[i]);
            cb.setSelected(!hiddenPops.contains(key));
            cb.setWrapText(true);
            cb.setTextFill(colors[i]);
            cb.selectedProperty().addListener((o, was, on) -> {
                if (on) hiddenPops.remove(key); else hiddenPops.add(key);
                setGateHidden(key, !on);          // hide the dashed outline along with the events
                applyPopColoring();
                if (comparisonBox.isVisible()) rebuildMainView();
                if (gridToggle.isSelected()) buildPreviewGrid();
            });
            // Click the swatch label (not the tick) to isolate this population.
            final int idx = i;
            cb.setOnMouseClicked(e -> {
                if (e.getClickCount() == 2) { plot.setPopulationHighlight(idx); }
            });
            legendPane.getChildren().add(cb);
        }
        Label hint = new Label("Tick to show · untick to hide · double-click to isolate");
        hint.getStyleClass().add("subtitle");
        hint.setWrapText(true);
        legendPane.getChildren().add(hint);

        legendScroll.setVisible(true);
        legendScroll.setManaged(true);
    }

    /** A population's user-chosen colour, else the next palette entry. Colours are app-wide. */
    private Color popColorFor(String name, int index) {
        String web = ctx.workspace().markerColor(name);
        if (web != null) { try { return Color.web(web); } catch (Exception ignored) {} }
        return POP_PALETTE[index % POP_PALETTE.length];
    }

    /**
     * Paint the gating tree's EXISTING populations onto the map: each embedding row is mapped back to
     * its (sample, row) and tested against that sample's populations. A row in several nested
     * populations takes the deepest one, so CD4+ never hides Treg.
     */
    private void colorByExistingPopulations() {
        popColoring = null;
        int cs = embedding.indexOf("SampleIdx"), cr = embedding.indexOf("RowIndex");
        if (cs < 0 || cr < 0) return;

        List<String> names = populationNames();
        if (names.isEmpty()) { statusLabel.setText("No gated populations exist yet."); return; }

        // depth per population name, so the deepest (most specific) wins when they nest
        Map<String, Integer> depth = new LinkedHashMap<>();
        for (String s : ctx.workspace().sampleNames()) {
            if (!ctx.workspace().hasTree(s)) continue;
            for (PopNode n : ctx.workspace().treeFor(s).selfAndDescendants()) {
                if (n.isRoot()) continue;
                depth.merge(n.name(), n.chain().size(), Math::max);
            }
        }

        // masks[sample][popIndex] over that sample's original rows, built once
        Map<String, boolean[][]> masks = new LinkedHashMap<>();
        for (String s : embSamples) {
            EventData root = ctx.workspace().data(s);
            if (root == null || !ctx.workspace().hasTree(s)) continue;
            boolean[][] m = new boolean[names.size()][];
            for (PopNode n : ctx.workspace().treeFor(s).selfAndDescendants()) {
                if (n.isRoot()) continue;
                int pi = names.indexOf(n.name());
                if (pi < 0) continue;
                boolean[] keep = new boolean[root.rows()];
                java.util.Arrays.fill(keep, true);
                for (CytoPlot.Gate g : n.chain()) {
                    boolean[] gm = CytoPlot.mask(root, g);
                    for (int i = 0; i < keep.length; i++) keep[i] = keep[i] && gm[i];
                }
                m[pi] = keep;
            }
            masks.put(s, m);
        }
        if (masks.isEmpty()) {
            statusLabel.setText("Open those samples once so their events are loaded, then colour by population.");
            return;
        }

        int[] labels = new int[embedding.rows()];
        java.util.Arrays.fill(labels, -1);
        for (int i = 0; i < embedding.rows(); i++) {
            int si = (int) Math.round(embedding.get(i, cs));
            if (si < 0 || si >= embSamples.size()) continue;
            boolean[][] m = masks.get(embSamples.get(si));
            if (m == null) continue;
            int row = (int) Math.round(embedding.get(i, cr));
            int best = -1, bestDepth = -1;
            for (int p = 0; p < m.length; p++) {
                if (m[p] == null || row >= m[p].length || !m[p][row]) continue;
                int d = depth.getOrDefault(names.get(p), 0);
                if (d > bestDepth) { bestDepth = d; best = p; }
            }
            labels[i] = best;
        }

        String[] legend = new String[names.size()];
        Color[] colors = new Color[names.size()];
        int[] counts = new int[names.size()];
        for (int l : labels) if (l >= 0) counts[l]++;
        for (int i = 0; i < names.size(); i++) {
            legend[i] = names.get(i) + " (" + String.format("%,d", counts[i]) + ")";
            colors[i] = popColorFor(names.get(i), i);
        }
        popColoring = new PopColoring(labels, colors, legend);
        applyPopColoring();
        statusLabel.setText("Coloured by " + names.size()
                + " existing gated population(s). Counts are the embedded (downsampled) events only. "
                + "Use the legend on the right to show/hide each one.");
    }

    // ---- gate on the embedding -> real populations ---------------------------

    /** Map the selected polygon back through SampleIdx/RowIndex, adding a population per sample. */
    @FXML
    private void onAddPopulation() {
        CytoPlot.Gate g = gateList.getSelectionModel().getSelectedItem();
        if (ctx == null || embedding == null || g == null) return;
        addPopulationFromMask(g.name, CytoPlot.mask(embedding, g), g);
    }

    /**
     * Turn a mask over embedding rows into one index-defined population per contributing sample.
     * Shared by hand-drawn gates and (later) marker-defined rules, so both produce identical
     * populations from identical row sets.
     */
    /**
     * @param outline optional footprint carried onto the population purely so a reopened workspace can
     *                redraw it. It never decides membership — {@link CytoPlot#isIndexGate} routes every
     *                geometry path around it — so editing it cannot change a single count.
     */
    private void addPopulationFromMask(String rawName, boolean[] inside, CytoPlot.Gate outline) {
        int cs = embedding.indexOf("SampleIdx"), cr = embedding.indexOf("RowIndex");
        if (cs < 0 || cr < 0) { statusLabel.setText("Embedding is missing SampleIdx/RowIndex."); return; }

        Map<String, List<Integer>> bySample = new LinkedHashMap<>();
        for (int i = 0; i < embedding.rows(); i++) {
            if (!inside[i]) continue;
            int si = (int) Math.round(embedding.get(i, cs));
            if (si < 0 || si >= embSamples.size()) continue;
            bySample.computeIfAbsent(embSamples.get(si), k -> new ArrayList<>())
                    .add((int) Math.round(embedding.get(i, cr)));
        }
        if (bySample.isEmpty()) { statusLabel.setText("That selection contains no events."); return; }

        String name = (rawName != null && !rawName.isBlank()) ? rawName : axisX.split(" ")[0] + " gate";
        int added = 0;
        for (Map.Entry<String, List<Integer>> e : bySample.entrySet()) {
            int[] rows = e.getValue().stream().mapToInt(Integer::intValue).sorted().toArray();
            CytoPlot.Gate g = outline == null
                    ? new CytoPlot.Gate(name, "embedding", null, null, null, null)
                    : new CytoPlot.Gate(name, "embedding", outline.xChan, outline.yChan,
                                        outline.xs.clone(), outline.ys.clone());
            g.subSelected = rows;
            g.subBySample.put(e.getKey(), rows);

            PopNode root = ctx.workspace().treeFor(e.getKey());
            // Re-adding the same name replaces rather than stacks a duplicate (as applyGateToAllSamples does).
            root.children.removeIf(c -> name.equals(c.name()));
            PopNode node = new PopNode(g, root);
            node.count = rows.length;
            int total = ctx.workspace().eventCount(e.getKey());
            node.parentPct = total > 0 ? 100.0 * rows.length / total : 0;
            root.children.add(node);
            added++;
        }
        ctx.workspace().notifyTreeChanged();
        ctx.auditLog().add(AuditLog.Type.GATE, "", String.format(
                "'%s' gated on the %s embedding → %d population(s)", name, axisX.split(" ")[0], added));

        addPopButton.setDisable(true);
        // Only the events that entered the embedding (the equal-N downsample) can be selected, so the
        // population holds those rows — not every matching event in the file. Say so rather than imply
        // a full-file gate.
        statusLabel.setText(String.format(
                "Added '%s' to %d sample(s) — contains the embedded (downsampled) events only.", name, added));
    }

    // ---- marker-defined mode -------------------------------------------------

    private Window window() { return plotHost.getScene() == null ? null : plotHost.getScene().getWindow(); }

    /** Channels available to rules: the markers actually embedded, else every fluorescence channel. */
    private List<String> taggableChannels() {
        if (!embFeatures.isEmpty()) return embFeatures;
        List<String> out = new ArrayList<>();
        for (String c : ctx.workspace().channelNames()) if (!isScatterOrTime(c)) out.add(c);
        return out;
    }

    /** Tag = the existing channel→marker alias. "BV421-A is FoxP3" is already an app-wide concept. */
    @FXML
    private void onEditTags() {
        List<String> chans = taggableChannels();
        if (chans.isEmpty()) { statusLabel.setText("Load FCS first."); return; }

        GridPane gp = new GridPane();
        gp.setHgap(8); gp.setVgap(6);
        gp.add(new Label("Channel"), 0, 0);
        gp.add(new Label("Marker (tag)"), 1, 0);
        List<TextField> fields = new ArrayList<>();
        for (int i = 0; i < chans.size(); i++) {
            String ch = chans.get(i);
            TextField tf = new TextField(ctx.aliases().target(ch) == null ? "" : ctx.aliases().target(ch));
            tf.setPromptText("e.g. FoxP3");
            tf.setPrefWidth(160);
            fields.add(tf);
            gp.add(new Label(ch), 0, i + 1);
            gp.add(tf, 1, i + 1);
        }
        ScrollPane sp = new ScrollPane(gp);
        sp.setFitToWidth(true);
        sp.setPrefSize(380, 360);

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Tags — one marker per fluorochrome");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(sp);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;

        for (int i = 0; i < chans.size(); i++) {
            String t = fields.get(i).getText().trim();
            ctx.aliases().set(chans.get(i), t.isEmpty() ? null : t);
        }
        plot.refresh();
        refreshFromWorkspace();
        // A ComboBox's button cell does not repaint when the underlying label source changes.
        showAliases(colorByCombo); showAliases(fscCombo); showAliases(sscCombo); showAliases(liveDeadCombo);
        markerList.refresh();
        statusLabel.setText("Tags updated. They apply everywhere in the app — the Graph Window uses the same tags.");
    }

    /** One threshold per tagged marker: Auto (Otsu) or Manual (against the controls). */
    @FXML
    private void onMarkerThresholds() {
        List<String> chans = taggableChannels();
        if (chans.isEmpty()) { statusLabel.setText("Load FCS first."); return; }

        GridPane gp = new GridPane();
        gp.setHgap(8); gp.setVgap(6);
        gp.add(new Label("Marker"), 0, 0);
        gp.add(new Label("Threshold"), 1, 0);
        Map<String, TextField> fields = new LinkedHashMap<>();
        for (int i = 0; i < chans.size(); i++) {
            String ch = chans.get(i);
            TextField tf = new TextField(markerThresholds.containsKey(ch)
                    ? String.format("%.1f", markerThresholds.get(ch))
                    : (ctx.fmo().has(ch) ? String.format("%.1f", ctx.fmo().level(ch)) : ""));
            tf.setPrefWidth(90);
            fields.put(ch, tf);

            Button auto = new Button("Auto");
            auto.setOnAction(e -> {
                ObjectNode args = JSON.createObjectNode();
                args.put("sample", controlCombo.getValue());
                args.putArray("channels").add(ch);
                ctx.jobs().run(ctx.bridge().command("positivity_thresholds", args), r -> {
                    JsonNode c = r.path("channels").path(0);
                    if (c.path("ok").asBoolean(false)) tf.setText(String.format("%.1f", c.path("threshold").asDouble()));
                });
            });
            Button manual = new Button("Manual…");
            manual.setOnAction(e -> PositivityDialog
                    .show(window(), ctx, ch, parseOrNull(tf.getText()),
                            (s, cb) -> PositivityDialog.loadEvents(ctx, s, cb))
                    .ifPresent(v -> tf.setText(String.format("%.1f", v))));

            gp.add(new Label(ctx.aliases().label(ch)), 0, i + 1);
            gp.add(tf, 1, i + 1);
            gp.add(auto, 2, i + 1);
            gp.add(manual, 3, i + 1);
        }
        ScrollPane sp = new ScrollPane(gp);
        sp.setFitToWidth(true);
        sp.setPrefSize(480, 360);

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Marker thresholds");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(new VBox(8,
                subtitle("Auto is an Otsu split of the chosen control. Manual sets the threshold against "
                        + "the unstained and single-stain controls. Blank means \"unset\": a rule using "
                        + "that marker will match no events rather than guess."),
                sp));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;

        markerThresholds.clear();
        fields.forEach((ch, tf) -> {
            Double v = parseOrNull(tf.getText());
            if (v != null) markerThresholds.put(ch, v);
        });
        statusLabel.setText(markerThresholds.size() + " marker threshold(s) set.");
        if (COLOR_RULES.equals(colorByCombo.getValue())) applyColorBy();
    }

    private static Label subtitle(String text) {
        Label l = new Label(text);
        l.setWrapText(true);
        l.setMaxWidth(470);
        l.getStyleClass().add("subtitle");
        return l;
    }

    private static double parseOrDefault(String s, double dflt) {
        try { return s == null || s.isBlank() ? dflt : Double.parseDouble(s.trim()); }
        catch (NumberFormatException e) { return dflt; }
    }

    /** Persist the chosen transform globally (apply_transformation) so Clustering reads the same one. */
    private void applyGlobalTransform() {
        if (ctx == null) return;
        String m = transformCombo.getValue();
        if ("none".equals(m)) return;   // apply_transformation only records logicle/arcsinh/log
        ObjectNode a = JSON.createObjectNode();
        a.put("method", m);
        a.put("cofactor", parseOrDefault(cofactorField.getText(), 150.0));
        ctx.jobs().run(ctx.bridge().command("apply_transformation", a), r -> {});
    }

    private static Double parseOrNull(String s) {
        try { return s == null || s.isBlank() ? null : Double.parseDouble(s.trim()); }
        catch (NumberFormatException e) { return null; }
    }

    /** Build one boolean rule: a name plus a +/- state per marker. */
    @FXML
    private void onAddRule() {
        List<String> chans = taggableChannels();
        if (chans.isEmpty()) { statusLabel.setText("Load FCS first."); return; }

        TextField nameField = new TextField("Population " + (rules.size() + 1));
        GridPane gp = new GridPane();
        gp.setHgap(10); gp.setVgap(4);
        gp.add(new Label("Marker"), 0, 0);
        gp.add(new Label("State"), 1, 0);
        Map<String, ComboBox<String>> states = new LinkedHashMap<>();
        for (int i = 0; i < chans.size(); i++) {
            String ch = chans.get(i);
            ComboBox<String> cb = new ComboBox<>(FXCollections.observableArrayList("any", "positive", "negative"));
            cb.getSelectionModel().select("any");
            cb.setPrefWidth(110);
            states.put(ch, cb);
            String warn = markerThresholds.containsKey(ch) ? "" : "  (no threshold)";
            gp.add(new Label(ctx.aliases().label(ch) + warn), 0, i + 1);
            gp.add(cb, 1, i + 1);
        }
        ScrollPane sp = new ScrollPane(gp);
        sp.setFitToWidth(true);
        sp.setPrefSize(380, 320);

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Add rule");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(new VBox(8,
                new HBox(8, new Label("Name:"), nameField),
                subtitle("Markers left on \"any\" are ignored. All chosen states must hold (AND)."),
                sp));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;

        List<EmbeddingRules.Clause> clauses = new ArrayList<>();
        for (Map.Entry<String, ComboBox<String>> e : states.entrySet()) {
            String v = e.getValue().getValue();
            if ("positive".equals(v)) clauses.add(new EmbeddingRules.Clause(e.getKey(), true));
            else if ("negative".equals(v)) clauses.add(new EmbeddingRules.Clause(e.getKey(), false));
        }
        if (clauses.isEmpty()) { statusLabel.setText("A rule needs at least one positive or negative marker."); return; }
        String name = nameField.getText().isBlank() ? "Population " + (rules.size() + 1) : nameField.getText().trim();
        rules.add(new EmbeddingRules.Rule(name, clauses));
        statusLabel.setText("Added " + rules.get(rules.size() - 1) + ". Click Apply to see it on the map.");
    }

    @FXML
    private void onRemoveRule() {
        EmbeddingRules.Rule r = ruleList.getSelectionModel().getSelectedItem();
        if (r != null) rules.remove(r);
    }

    /** Colour the map by the rules and turn each into a dashed suggestion. */
    @FXML
    private void onApplyRules() {
        if (embedding == null) { statusLabel.setText("Run the embedding first."); return; }
        if (rules.isEmpty()) { statusLabel.setText("Add a rule first."); return; }

        List<String> missing = new ArrayList<>();
        for (EmbeddingRules.Rule r : rules)
            for (EmbeddingRules.Clause c : r.clauses())
                if (!markerThresholds.containsKey(c.channel()) && !missing.contains(c.channel()))
                    missing.add(c.channel());
        if (!missing.isEmpty()) {
            statusLabel.setText("Set a threshold for: " + String.join(", ", missing)
                    + ". Until then those rules match nothing.");
            return;
        }

        clearProvisionalGates();
        suggestions.clear();
        for (int i = 0; i < rules.size(); i++) {
            EmbeddingRules.Rule r = rules.get(i);
            addSuggestion(r.name(), EmbeddingRules.evaluate(embedding, r, markerThresholds), r.toString(), i);
        }
        colorByCombo.getSelectionModel().select(COLOR_RULES);
        applyColorBy();
    }

    /** FlowSOM on the marker values. The clusters are proposals; nothing becomes a population here. */
    @FXML
    private void onSuggest() {
        if (embedding == null) { statusLabel.setText("Run the embedding first."); return; }
        ObjectNode args = JSON.createObjectNode();
        args.put("k", kSpinner.getValue());
        args.put("method", clusterMethodCombo.getValue());
        args.put("scale", scaleCombo.getValue());
        args.put("seed", seedSpinner.getValue());
        suggestButton.setDisable(true);
        statusLabel.setText("Clustering markers with " + clusterMethodCombo.getValue()
                + " (scale " + scaleCombo.getValue() + ")…");
        ctx.jobs().run(ctx.bridge().command("cluster_embedding", args), r -> {
            suggestButton.setDisable(false);
            JsonNode labels = r.path("labels");
            if (labels.size() != embedding.rows()) {
                statusLabel.setText("Clustering returned " + labels.size()
                        + " labels for " + embedding.rows() + " events — ignoring.");
                return;
            }
            int k = r.path("n_clusters").asInt();
            // Name each cluster by its PHENOTYPE (median marker vs the raw threshold), the way single-
            // cell tools annotate clusters. Falls back to "Cluster N" when no thresholds are set.
            List<String> feats = new ArrayList<>();
            r.path("features").forEach(n -> feats.add(n.asText()));
            String[] pheno = clusterNames(r, feats, k);
            clearProvisionalGates();
            suggestions.clear();
            for (int c = 0; c < k; c++) {
                boolean[] m = new boolean[embedding.rows()];
                for (int i = 0; i < m.length; i++) m[i] = labels.get(i).asInt() == c;
                addSuggestion(pheno[c], m, r.path("actual").asText() + " k=" + k, c);
            }
            int[] lab = new int[embedding.rows()];
            for (int i = 0; i < lab.length; i++) lab[i] = labels.get(i).asInt();
            int[] counts = new int[k];
            for (int l : lab) if (l >= 0 && l < k) counts[l]++;
            Color[] colors = new Color[k];
            String[] names = new String[k];
            for (int c = 0; c < k; c++) {
                colors[c] = POP_PALETTE[c % POP_PALETTE.length];
                names[c] = pheno[c] + " (" + String.format("%,d", counts[c]) + ")";
            }
            plot.clearColorByChannel();
            plot.setPopulationLegendInside(false);
            plot.setColorByPopulation(lab, colors, names);
            // A shared external legend for the suggestions, too.
            popColoring = new PopColoring(lab, colors, names);
            buildLegend();
            String actual = r.path("actual").asText();
            statusLabel.setText(k + " candidate population(s) from " + actual.toUpperCase()
                    + (actual.equals(r.path("requested").asText()) ? "" : " (FlowSOM was unavailable)")
                    + ", named by " + namingCombo.getValue() + "."
                    + " Select one to inspect its stats, then Approve or Reject.");
        });
    }

    /**
     * A phenotype name from a cluster's raw median expression, e.g. "CD45+ CD4+ FoxP3+". Only markers
     * that carry BOTH a tag and a threshold contribute; a marker whose median exceeds its threshold is
     * "+". Lists positives (single-cell convention). Falls back to "Cluster N" when nothing resolves.
     */
    /**
     * Name every FlowSOM cluster by the method chosen in {@link #namingCombo}.
     *
     * THRESHOLD calls a marker + when a cluster's median exceeds the user's per-marker cut. The three
     * relative methods need no threshold: a whole high-marker region (the FOXP3 gradient a control cut
     * misses) is captured because the call is made from the between-cluster distribution, not per event.
     * Only TAGGED markers participate; a matching user rule wins the name, else the positive tags do.
     */
    private String[] clusterNames(JsonNode r, List<String> feats, int k) {
        String method = namingCombo.getValue();

        // cluster medians matrix restricted to tagged markers
        List<String> tagged = new ArrayList<>();
        List<Integer> col = new ArrayList<>();
        for (int j = 0; j < feats.size(); j++)
            if (ctx.aliases().target(feats.get(j)) != null) { tagged.add(feats.get(j)); col.add(j); }

        double[][] allMed = new double[k][tagged.size()];
        for (int c = 0; c < k; c++) {
            JsonNode m = r.path("clusters").path(c).path("medians");
            for (int t = 0; t < tagged.size(); t++) allMed[c][t] = m.path(col.get(t)).asDouble();
        }

        // Which markers are callable, and the cut per marker (null for z-score).
        EmbeddingRules.CallMethod cm;
        List<String> callChannels = new ArrayList<>();
        List<Integer> callCol = new ArrayList<>();
        double[] cut;
        if (NAME_THRESHOLD.equals(method)) {
            cm = EmbeddingRules.CallMethod.THRESHOLD;
            for (int t = 0; t < tagged.size(); t++)
                if (markerThresholds.containsKey(tagged.get(t))) { callChannels.add(tagged.get(t)); callCol.add(t); }
            cut = new double[callChannels.size()];
            for (int t = 0; t < callChannels.size(); t++) cut[t] = markerThresholds.get(callChannels.get(t));
        } else if (NAME_ZSCORE.equals(method)) {
            cm = EmbeddingRules.CallMethod.ZSCORE;
            callChannels.addAll(tagged);
            for (int t = 0; t < tagged.size(); t++) callCol.add(t);
            cut = null;
        } else {
            cm = NAME_OTSU.equals(method) ? EmbeddingRules.CallMethod.OTSU_EVENTS
                                          : EmbeddingRules.CallMethod.GLOBAL_MEDIAN;
            callChannels.addAll(tagged);
            for (int t = 0; t < tagged.size(); t++) callCol.add(t);
            cut = new double[tagged.size()];
            for (int t = 0; t < tagged.size(); t++) {
                double[] vals = channelValues(tagged.get(t));
                cut[t] = (cm == EmbeddingRules.CallMethod.OTSU_EVENTS) ? Stats.otsu(vals) : Stats.median(vals);
            }
        }

        // Sub-matrix of medians for the callable channels (z-score needs the full callable set).
        double[][] callMed = new double[k][callChannels.size()];
        for (int c = 0; c < k; c++)
            for (int t = 0; t < callChannels.size(); t++) callMed[c][t] = allMed[c][callCol.get(t)];

        String[] out = new String[k];
        for (int c = 0; c < k; c++) {
            Map<String, Boolean> calls = EmbeddingRules.relativeCalls(
                    callChannels, callMed[c], callMed, cut, cm);
            out[c] = EmbeddingRules.nameCluster(calls, rules, ch -> ctx.aliases().target(ch), "Cluster " + (c + 1));
        }
        return out;
    }

    /** All raw values of a feature column across the embedding (for Otsu / global-median cuts). */
    private double[] channelValues(String ch) {
        int idx = embedding.indexOf(ch);
        if (idx < 0) return new double[0];
        double[] v = new double[embedding.rows()];
        for (int i = 0; i < v.length; i++) v[i] = embedding.get(i, idx);
        return v;
    }

    /** A suggestion's outline is drawn dashed; its row set — not the outline — is the population. */
    private void addSuggestion(String name, boolean[] mask, String provenance, int paintIndex) {
        int n = 0;
        for (boolean b : mask) if (b) n++;
        if (n == 0) return;

        int cx = embedding.indexOf(axisX), cy = embedding.indexOf(axisY);
        double[] xs = new double[embedding.rows()], ys = new double[embedding.rows()];
        for (int i = 0; i < xs.length; i++) { xs[i] = embedding.get(i, cx); ys[i] = embedding.get(i, cy); }
        double[][] hull = EmbeddingRules.outline(xs, ys, mask, 1.4);

        CytoPlot.Gate g = null;
        if (hull != null) {
            g = new CytoPlot.Gate(name, "polygon", axisX, axisY, hull[0], hull[1]);
            g.provisional = true;
            // Colour the dashed outline to match this population's swatch and events, so overlapping
            // suggestions are told apart instead of all being one red.
            g.border = popColorFor(name, paintIndex);
            plot.addGate(g);
        }
        suggestions.add(new Suggestion(name, mask, n, g, provenance, paintIndex));
    }

    private void clearProvisionalGates() {
        plot.gates().removeIf(g -> g.provisional);
        plot.refresh();
        refreshGateList();
    }

    /** Hide/show the dashed outline(s) whose name matches a legend population key. */
    private void setGateHidden(String popKey, boolean hidden) {
        for (CytoPlot.Gate g : plot.gates())
            if (g.name != null && g.name.equals(popKey)) g.hidden = hidden;
        plot.refresh();
    }

    private void highlightSuggestion(Suggestion s) {
        if (s == null) { plot.setPopulationHighlight(-1); return; }
        plot.setPopulationHighlight(s.paintIndex);
        if (s.outline != null) plot.selectGate(s.outline);
    }

    /**
     * Inspect a candidate before committing: its event count, % of the embedding, and the median raw
     * intensity of every tagged marker (with +/- against the threshold). This is the "check the stats"
     * step — the population is not real until Approve.
     */
    @FXML
    private void onInspectSuggestion() {
        Suggestion s = suggestionList.getSelectionModel().getSelectedItem();
        if (s == null || embedding == null) { statusLabel.setText("Select a candidate first."); return; }

        javafx.scene.control.TableView<String[]> table = new javafx.scene.control.TableView<>();
        javafx.scene.control.TableColumn<String[], String> cMarker = new javafx.scene.control.TableColumn<>("Marker");
        cMarker.setCellValueFactory(c -> new javafx.beans.property.ReadOnlyStringWrapper(c.getValue()[0]));
        cMarker.setPrefWidth(150);
        javafx.scene.control.TableColumn<String[], String> cMed = new javafx.scene.control.TableColumn<>("Median (raw)");
        cMed.setCellValueFactory(c -> new javafx.beans.property.ReadOnlyStringWrapper(c.getValue()[1]));
        cMed.setPrefWidth(110);
        javafx.scene.control.TableColumn<String[], String> cCall = new javafx.scene.control.TableColumn<>("Call");
        cCall.setCellValueFactory(c -> new javafx.beans.property.ReadOnlyStringWrapper(c.getValue()[2]));
        cCall.setPrefWidth(70);
        table.getColumns().add(cMarker); table.getColumns().add(cMed); table.getColumns().add(cCall);

        for (String ch : embFeatures) {
            double[] vals = maskedValues(ch, s.mask);
            String med = vals.length == 0 ? "—" : StatKeys.fmt(Stats.median(vals));
            Double thr = markerThresholds.get(ch);
            String call = thr == null ? "" : (vals.length > 0 && Stats.median(vals) > thr ? "+" : "−");
            table.getItems().add(new String[]{ctx.aliases().label(ch), med, call});
        }
        table.setPrefHeight(Math.min(320, 60 + embFeatures.size() * 30));

        double pct = 100.0 * s.count / Math.max(1, embedding.rows());
        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Inspect — " + s.name);
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(new VBox(8,
                subtitle(String.format("%,d events — %.1f%% of the embedding. Derived from %s. "
                        + "Medians are raw; + / − is against the marker threshold.", s.count, pct, s.provenance)),
                table));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.CLOSE);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        plot.setPopulationHighlight(s.paintIndex);
        dlg.showAndWait();
    }

    /** Raw values of {@code ch} for the embedding rows in {@code mask}. */
    private double[] maskedValues(String ch, boolean[] mask) {
        int col = embedding.indexOf(ch);
        if (col < 0) return new double[0];
        int n = 0; for (boolean b : mask) if (b) n++;
        double[] out = new double[n];
        for (int i = 0, k = 0; i < mask.length; i++) if (mask[i]) out[k++] = embedding.get(i, col);
        return out;
    }

    @FXML
    private void onNameSuggestion() {
        Suggestion s = suggestionList.getSelectionModel().getSelectedItem();
        if (s == null) return;
        javafx.scene.control.TextInputDialog dlg = new javafx.scene.control.TextInputDialog(s.name);
        dlg.setTitle("Name population");
        dlg.setHeaderText(null);
        dlg.setContentText("Name:");
        AppIcons.theme(dlg, window());
        dlg.showAndWait().filter(v -> !v.isBlank()).ifPresent(v -> {
            s.name = v.trim();
            if (s.outline != null) s.outline.name = s.name;
            suggestionList.refresh();
            plot.refresh();
        });
    }

    /**
     * Approve: the row set becomes a real population per sample; the outline stops being dashed.
     * Membership is the row set, so editing the outline afterwards cannot change the counts.
     */
    @FXML
    private void onApproveSuggestion() {
        Suggestion s = suggestionList.getSelectionModel().getSelectedItem();
        if (s == null || embedding == null) return;
        addPopulationFromMask(s.name, s.mask, s.outline);
        if (s.outline != null) { s.outline.provisional = false; plot.refresh(); }
        suggestions.remove(s);
        refreshGateList();
        ctx.auditLog().add(AuditLog.Type.GATE, "",
                "'" + s.name + "' approved from " + s.provenance + " (" + s.count + " embedded events)");
    }

    @FXML
    private void onRejectSuggestion() {
        Suggestion s = suggestionList.getSelectionModel().getSelectedItem();
        if (s == null) return;
        if (s.outline != null) plot.gates().remove(s.outline);
        suggestions.remove(s);
        plot.setPopulationHighlight(-1);
        plot.refresh();
        refreshGateList();
    }

    // ---- controls over the automated steps -----------------------------------

    @FXML
    private void onManualThreshold() {
        if (ctx == null || liveDeadCombo.getValue() == null) return;
        PositivityDialog.show(window(), ctx, liveDeadCombo.getValue(), parseOrNull(thresholdField.getText()),
                        (s, cb) -> PositivityDialog.loadEvents(ctx, s, cb))
                .ifPresent(v -> {
                    thresholdField.setText(String.format("%.1f", v));
                    statusLabel.setText(String.format("Live/Dead threshold set to %.1f from the controls.", v));
                });
    }

    /** Say plainly what the automatic cleanup does, rather than leaving it as a mystery checkbox. */
    @FXML
    private void onDebrisInfo() {
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle("Auto-remove debris");
        a.setHeaderText("Central-percentile scatter cleanup");
        TextArea ta = new TextArea(
                "Events whose FSC or SSC falls in the extreme tails of the scatter distribution are "
                + "dropped before the embedding. It is the same routine the Compensation wizard uses.\n\n"
                + "WHAT IT REMOVES\n"
                + "Sub-cellular debris (very low FSC) and saturated or aggregated events (very high FSC/SSC). "
                + "It does NOT remove doublets, and it is not a viability gate.\n\n"
                + "CHANNELS\n"
                + "The FSC and SSC channels chosen beside the checkbox. Change them if your panel names "
                + "them unusually.\n\n"
                + "IF YOU WANT CONTROL\n"
                + "Uncheck it and every event is kept. Or gate the scatter yourself in a graph window, then "
                + "run in Targeted mode on that population — the embedding will use exactly your gate and "
                + "nothing else.\n\n"
                + "The event count before and after appears in the status line after each run.");
        ta.setEditable(false); ta.setWrapText(true); ta.setPrefSize(520, 320);
        a.getDialogPane().setContent(ta);
        AppIcons.theme(a, window());
        a.showAndWait();
    }

    // ---- colours -------------------------------------------------------------

    /**
     * Edit colours. When colouring by populations, each population gets a picker and the choice is
     * saved app-wide ({@link WorkspaceModel#setMarkerColor}). When colouring by a marker's intensity,
     * offer the density gradient (Jet / Viridis / …) instead, since there is no per-population colour.
     */
    @FXML
    private void onEditColours() {
        if (embedding == null) { statusLabel.setText("Run an embedding first."); return; }
        String v = colorByCombo.getValue();
        boolean byPop = (COLOR_RULES.equals(v) || COLOR_EXISTING.equals(v)) && popColoring != null;

        if (byPop) {
            editPopulationColours();
        } else {
            editGradient();
        }
    }

    private void editPopulationColours() {
        String[] names = popColoring.names();
        Color[] colors = popColoring.colors();
        GridPane gp = new GridPane();
        gp.setHgap(10); gp.setVgap(6);
        List<javafx.scene.control.ColorPicker> pickers = new ArrayList<>();
        // Strip the trailing " (12,345)" count so the saved key is the population name itself.
        List<String> keys = new ArrayList<>();
        for (int i = 0; i < names.length; i++) {
            String key = names[i].replaceAll("\\s*\\(\\d[\\d,]*\\)\\s*$", "");
            keys.add(key);
            javafx.scene.control.ColorPicker cp = new javafx.scene.control.ColorPicker(colors[i]);
            pickers.add(cp);
            gp.add(new Label(key), 0, i);
            gp.add(cp, 1, i);
        }
        ScrollPane sp = new ScrollPane(gp);
        sp.setFitToWidth(true);
        sp.setPrefSize(360, Math.min(420, 60 + names.length * 34));

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Population colours");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(new VBox(8,
                subtitle("Each population's colour is used everywhere it appears — here, the preview grid, "
                        + "and every group panel."), sp));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;

        for (int i = 0; i < keys.size(); i++)
            ctx.workspace().setMarkerColor(keys.get(i), toWeb(pickers.get(i).getValue()));
        applyColorBy();     // recompute with the new colours
        statusLabel.setText("Population colours updated (saved app-wide).");
    }

    private void editGradient() {
        ComboBox<String> palette = new ComboBox<>(FXCollections.observableArrayList(CytoPlot.paletteNames()));
        palette.getSelectionModel().select(plot.palette());
        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Intensity gradient");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(new VBox(8,
                subtitle("The colour ramp used when colouring by a marker's intensity or by event density."),
                new HBox(8, new Label("Gradient:"), palette)));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK || palette.getValue() == null) return;
        for (CytoPlot p : allPlots()) p.setPalette(palette.getValue());
        statusLabel.setText("Gradient set to " + palette.getValue() + ".");
    }

    private static String toWeb(Color c) {
        return String.format("#%02X%02X%02X",
                (int) Math.round(c.getRed() * 255),
                (int) Math.round(c.getGreen() * 255),
                (int) Math.round(c.getBlue() * 255));
    }

    // ---- experimental groups -------------------------------------------------

    /**
     * Assign each FCS file to a named group. Any number of groups; add them as you go.
     *
     * Grouping never changes how the embedding is COMPUTED — every sample is still pooled into one
     * coordinate space. It only changes how the map is split for display, which is the only way two
     * panels can be compared: an island at (10, 30) means the same thing in every panel precisely
     * because they share one embedding.
     */
    @FXML
    private void onEditGroups() {
        if (ctx == null) return;
        List<String> samples = new ArrayList<>(ctx.workspace().sampleNames());
        if (samples.isEmpty()) { statusLabel.setText("Load FCS first."); return; }

        javafx.collections.ObservableList<String> groupNames =
                FXCollections.observableArrayList(ctx.workspace().groupNames());
        if (!groupNames.contains(GROUP_IGNORE)) groupNames.add(0, GROUP_IGNORE);
        if (groupNames.size() == 1) groupNames.addAll("Group 1", "Group 2");

        Map<String, ComboBox<String>> assign = new LinkedHashMap<>();
        GridPane gp = new GridPane();
        gp.setHgap(10); gp.setVgap(4);
        gp.add(new Label("Sample"), 0, 0);
        gp.add(new Label("Group"), 1, 0);
        for (int i = 0; i < samples.size(); i++) {
            String s = samples.get(i);
            ComboBox<String> cb = new ComboBox<>(groupNames);
            cb.setEditable(true);              // type a new group name straight into the row
            cb.setPrefWidth(160);
            // The editable ComboBox's arrow is drawn on the editor's light background, so the default
            // light-grey arrow disappears. Force a dark one.
            cb.setStyle("-fx-mark-color:#333333;");
            // Default every unassigned sample to Ignore: the user opts samples INTO groups, rather than
            // opting controls out. Controls then never leak into the embedding by omission.
            String cur = ctx.workspace().groupOf(s);
            cb.setValue(cur != null ? cur : GROUP_IGNORE);
            assign.put(s, cb);
            gp.add(new Label(shortName(s)), 0, i + 1);
            gp.add(cb, 1, i + 1);
        }
        ScrollPane sp = new ScrollPane(gp);
        sp.setFitToWidth(true);
        sp.setPrefSize(460, 380);

        Label hint = subtitle("");

        // Controls must not enter a biology embedding: an unstained tube has no marker signal, and an
        // FMO's whole point is a missing colour, so both would form their own artefactual islands.
        Button guess = new Button("Mark controls as Ignore");
        guess.setOnAction(e -> {
            int n = 0;
            for (Map.Entry<String, ComboBox<String>> en : assign.entrySet())
                if (CONTROL_RE.matcher(en.getKey()).matches()) { en.getValue().setValue(GROUP_IGNORE); n++; }
            hint.setText(n + " file(s) matched a control naming pattern and were set to Ignore. Check them.");
        });

        Button addGroup = new Button("Add group…");
        addGroup.setOnAction(e -> {
            javafx.scene.control.TextInputDialog td =
                    new javafx.scene.control.TextInputDialog("Group " + (groupNames.size() + 1));
            td.setTitle("New group");
            td.setHeaderText(null);
            td.setContentText("Name:");
            AppIcons.theme(td, window());
            td.showAndWait().filter(v -> !v.isBlank())
                    .ifPresent(v -> { if (!groupNames.contains(v.trim())) groupNames.add(v.trim()); });
        });

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Experimental groups");
        dlg.setHeaderText(null);
        dlg.setResizable(true);
        dlg.getDialogPane().setContent(new VBox(8,
                subtitle("Add as many groups as you need, then assign each file. You can also type a new "
                        + "group name directly into a row. Set a file to \"Ignore\" to keep it out of the "
                        + "embedding entirely — do this for unstained, FMO and single-stain controls. A blank "
                        + "row is still embedded, just not shown as its own panel. All embedded samples share "
                        + "ONE map; grouping only splits the display."),
                new HBox(8, addGroup, guess), hint, sp));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;

        ctx.workspace().clearGroups();
        assign.forEach((s, cb) -> ctx.workspace().setGroup(s, cb.getValue()));
        int nIgnored = ctx.workspace().samplesInGroup(GROUP_IGNORE).size();
        List<String> shown = comparisonGroups();
        statusLabel.setText(shown.isEmpty()
                ? "No groups assigned." + (nIgnored > 0 ? " " + nIgnored + " file(s) ignored." : "")
                : shown.size() + " group(s): " + String.join(", ", shown)
                  + (nIgnored > 0 ? "; " + nIgnored + " ignored (not embedded)" : "")
                  + ". Turn on \"By group\" to compare them side by side. Re-run to apply Ignore.");
        if (groupToggle.isSelected()) rebuildMainView();
        if (gridToggle.isSelected()) buildPreviewGrid();
    }

    // ---- side-by-side group comparison ---------------------------------------

    /** Group names that get a panel: everything except Ignore, which is not in the embedding at all. */
    private List<String> comparisonGroups() {
        List<String> out = new ArrayList<>();
        for (String g : ctx.workspace().groupNames()) if (!GROUP_IGNORE.equals(g)) out.add(g);
        return out;
    }

    @FXML
    private void onToggleGroup() {
        rebuildMainView();
        if (gridToggle.isSelected()) buildPreviewGrid();
    }

    /** The panels currently available to show, in draw order (depends on the By-group toggle). */
    private List<String> availablePanels() {
        List<String> keys = new ArrayList<>();
        keys.add(PANEL_ALL);
        if (groupToggle.isSelected()) keys.addAll(comparisonGroups());
        else keys.addAll(embSamples);
        return keys;
    }

    private boolean panelShown(String key) { return !hiddenPanels.contains(key); }

    /**
     * Choose which panels appear side by side. Applies to the "By group" main view AND the preview
     * grid, so the display area shows exactly what the user asks for — one group, both, all samples,
     * or any subset of individual files.
     */
    @FXML
    private void onSelectPanels() {
        if (embedding == null) { statusLabel.setText("Run an embedding first."); return; }
        List<String> keys = availablePanels();
        VBox box = new VBox(4);
        List<CheckBox> checks = new ArrayList<>();
        for (String k : keys) {
            CheckBox cb = new CheckBox(k);
            cb.setSelected(panelShown(k));
            checks.add(cb);
            box.getChildren().add(cb);
        }
        ScrollPane sp = new ScrollPane(box);
        sp.setFitToWidth(true);
        sp.setPrefSize(300, Math.min(420, 60 + keys.size() * 28));

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Show panels");
        dlg.setHeaderText(null);
        dlg.getDialogPane().setContent(new VBox(8,
                subtitle(groupToggle.isSelected()
                        ? "Tick the groups to display side by side."
                        : "Tick the samples to display. Turn on \"By group\" first to pick whole groups."),
                sp));
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, window());
        if (dlg.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;

        for (int i = 0; i < keys.size(); i++)
            if (checks.get(i).isSelected()) hiddenPanels.remove(keys.get(i));
            else hiddenPanels.add(keys.get(i));

        if (comparisonBox.isVisible()) rebuildMainView();
        if (gridToggle.isSelected()) buildPreviewGrid();
        long shown = keys.stream().filter(this::panelShown).count();
        statusLabel.setText("Showing " + shown + " of " + keys.size() + " panel(s).");
    }

    /**
     * Swap the main area between the single pooled plot and one panel per group.
     *
     * Every panel is a view of the SAME embedding with the SAME locked axis limits, so an island at
     * (10, 30) is the same island in each. Gating is disabled here for the same reason it is disabled
     * on a focused facet: a panel holds a subset of the embedding's rows, and a polygon drawn on it
     * would not line up with the pooled rows that populations are mapped back through.
     */
    private void rebuildMainView() {
        boolean byGroup = groupToggle.isSelected() && embedding != null;
        comparePlots.clear();
        compareCaptions.clear();
        comparisonBox.getChildren().clear();

        if (!byGroup) {
            comparisonBox.setVisible(false); comparisonBox.setManaged(false);
            plot.setVisible(true); plot.setManaged(true);
            gateToggle.setDisable(embedding == null || focusedSample >= 0);
            return;
        }

        List<String> groups = comparisonGroups();
        if (groups.isEmpty()) {
            groupToggle.setSelected(false);
            statusLabel.setText("No groups assigned yet — click \"Groups…\" first.");
            rebuildMainView();
            return;
        }

        int cs = embedding.indexOf("SampleIdx");
        if (panelShown(PANEL_ALL))
            comparisonBox.getChildren().add(comparePanel(PANEL_ALL, embedding, null));
        for (String grp : groups) {
            if (!panelShown(grp)) continue;
            java.util.Set<Integer> idx = new java.util.HashSet<>();
            for (String s : ctx.workspace().samplesInGroup(grp)) {
                int si = embSamples.indexOf(s);
                if (si >= 0) idx.add(si);
            }
            boolean[] keep = new boolean[embedding.rows()];
            for (int i = 0; i < keep.length; i++) keep[i] = idx.contains((int) Math.round(embedding.get(i, cs)));
            EventData facet = embedding.subset(keep);
            if (facet.rows() > 0) comparisonBox.getChildren().add(comparePanel(grp, facet, keep));
        }

        plot.setVisible(false); plot.setManaged(false);
        comparisonBox.setVisible(true); comparisonBox.setManaged(true);
        gateToggle.setSelected(false);
        gateToggle.setDisable(true);
        applyExportFormat();
        statusLabel.setText(groups.size() + " group(s) side by side on one shared embedding. "
                + "Copy and Export capture all panels. Gating is disabled here — switch off \"By group\" to gate.");
    }

    /** {@code keep} is the pooled-row mask this panel was subset from, or null for the full embedding. */
    private VBox comparePanel(String title, EventData data, boolean[] keep) {
        CytoPlot p = new CytoPlot();
        p.setChannelLabeler(ch -> ctx.aliases().label(ch));
        p.setData(data);
        p.setAxes(axisX, axisY);
        p.setPlotType("pseudocolor");
        p.setLightMode(lightBgToggle.isSelected());
        applyLockedRange(p);
        applyColorByTo(p, keep);
        p.prefHeightProperty().bind(plotHost.heightProperty().subtract(28));
        p.setPrefWidth(360);
        comparePlots.add(p);

        String capText = title + "  (" + String.format("%,d", data.rows()) + " events)";
        compareCaptions.add(capText);
        Label cap = new Label(capText);
        cap.getStyleClass().add("subtitle");
        VBox v = new VBox(2, p, cap);
        v.setAlignment(javafx.geometry.Pos.CENTER);
        HBox.setHgrow(v, Priority.ALWAYS);
        return v;
    }

    /**
     * Apply the current Colour-by choice to a panel. {@code keep} is the pooled-row mask the panel's
     * data was subset from (null = the whole embedding). Population colourings — rules and existing
     * gates — are computed once over the pooled rows ({@link #popColoring}); each panel just takes the
     * matching slice, so the same population is the same colour in every panel.
     */
    private void applyColorByTo(CytoPlot p, boolean[] keep) {
        String v = colorByCombo.getValue();
        if (v == null || COLOR_DENSITY.equals(v)) return;
        if (COLOR_SAMPLE.equals(v)) { p.setColorByChannel("SampleIdx", true); return; }
        if (COLOR_RULES.equals(v) || COLOR_EXISTING.equals(v)) {
            if (popColoring == null) return;
            int[] labels = displayLabels(popColoring.labels());   // honour show/hide ticks
            if (keep != null) {
                int n = 0; for (boolean b : keep) if (b) n++;
                int[] sub = new int[n];
                for (int i = 0, k = 0; i < keep.length; i++) if (keep[i]) sub[k++] = labels[i];
                labels = sub;
            }
            p.setPopulationLegendInside(false);   // the external legend serves every panel
            p.setColorByPopulation(labels, popColoring.colors(), popColoring.names());
            return;
        }
        p.setColorByChannel(v, false);
    }

    // ---- preview grid -------------------------------------------------------

    @FXML
    private void onToggleGrid() {
        boolean on = gridToggle.isSelected();
        gridScroll.setVisible(on); gridScroll.setManaged(on);
        if (on) buildPreviewGrid(); else previewGrid.getChildren().clear();
    }

    /** Facet the SAME embedding per sample (or per group): identical axes, so tiles are comparable. */
    private void buildPreviewGrid() {
        previewGrid.getChildren().clear();
        if (embedding == null) return;
        int cs = embedding.indexOf("SampleIdx");
        if (cs < 0) return;

        if (panelShown(PANEL_ALL))
            previewGrid.getChildren().add(facetTile(-1, embedding, null, PANEL_ALL));

        if (groupToggle.isSelected()) {
            List<String> groups = comparisonGroups();   // Ignore never gets a panel: it is not embedded
            if (groups.isEmpty()) {
                statusLabel.setText("No groups assigned yet — click \"Groups…\" first.");
            }
            for (String grp : groups) {
                if (!panelShown(grp)) continue;
                java.util.Set<Integer> idx = new java.util.HashSet<>();
                for (String s : ctx.workspace().samplesInGroup(grp)) {
                    int si = embSamples.indexOf(s);
                    if (si >= 0) idx.add(si);
                }
                boolean[] keep = new boolean[embedding.rows()];
                for (int i = 0; i < keep.length; i++) keep[i] = idx.contains((int) Math.round(embedding.get(i, cs)));
                EventData facet = embedding.subset(keep);
                // A group whose samples were all skipped (e.g. no events) gets no tile rather than an
                // empty one that would read as "this group has no cells".
                if (facet.rows() > 0)
                    previewGrid.getChildren().add(facetTile(-1, facet, keep,
                            grp + " (" + idx.size() + " file" + (idx.size() == 1 ? "" : "s") + ")"));
            }
        } else {
            for (int si = 0; si < embSamples.size(); si++) {
                if (!panelShown(embSamples.get(si))) continue;
                boolean[] keep = new boolean[embedding.rows()];
                for (int i = 0; i < keep.length; i++) keep[i] = Math.round(embedding.get(i, cs)) == si;
                EventData facet = embedding.subset(keep);
                if (facet.rows() > 0) previewGrid.getChildren().add(facetTile(si, facet, keep, shortName(embSamples.get(si))));
            }
        }
        applyExportFormat();
    }

    /** One tile. {@code si == -1} is a pooled tile (all samples, or a group) that clears the focus. */
    private VBox facetTile(int si, EventData facet, boolean[] keep, String caption) {
        CytoPlot p = new CytoPlot();
        p.setChannelLabeler(ch -> ctx.aliases().label(ch));
        p.setPrefSize(190, 170);
        p.setData(facet);
        p.setAxes(axisX, axisY);
        p.setPlotType("pseudocolor");
        p.setLightMode(lightBgToggle.isSelected());
        applyLockedRange(p);            // identical coordinate space across every facet
        applyColorByTo(p, keep);
        Label cap = new Label(caption + "  (" + String.format("%,d", facet.rows()) + ")");
        cap.setStyle("-fx-font-size:10;");

        VBox tile = new VBox(2, p, cap);
        tile.setUserData(si);          // group tiles are all -1, so position cannot identify them
        tile.setStyle(si == focusedSample ? FOCUSED_TILE : UNFOCUSED_TILE);
        tile.setOnMouseClicked(e -> focusSample(si));
        return tile;
    }

    private static final String FOCUSED_TILE   = "-fx-border-color:#4DA3FF; -fx-border-width:2; -fx-padding:2;";
    private static final String UNFOCUSED_TILE = "-fx-border-color:transparent; -fx-border-width:2; -fx-padding:2;";

    /**
     * Show one sample's events in the main plot, keeping the pooled axis limits so the view never jumps.
     * Gate drawing is disabled while focused: the focused plot holds a SUBSET of the embedding, so a
     * polygon's row indices would not line up with the pooled rows that {@code addPopulationFromMask}
     * maps back through. Returning to "All samples" re-enables it.
     */
    private void focusSample(int si) {
        if (embedding == null) return;
        focusedSample = si;
        EventData shown = embedding;
        if (si >= 0) {
            int cs = embedding.indexOf("SampleIdx");
            boolean[] keep = new boolean[embedding.rows()];
            for (int i = 0; i < keep.length; i++) keep[i] = Math.round(embedding.get(i, cs)) == si;
            shown = embedding.subset(keep);
        }
        plot.setData(shown);
        plot.setAxes(axisX, axisY);
        applyLockedRange(plot);

        boolean pooled = si < 0;
        gateToggle.setSelected(false);
        gateToggle.setDisable(!pooled);
        statusLabel.setText(pooled
                ? "Showing all samples pooled. You can draw gates."
                : "Focused on " + shortName(embSamples.get(si))
                  + String.format(" (%,d events). ", shown.rows())
                  + "Gate drawing is disabled while focused — go back to \"All samples\" to gate.");

        for (javafx.scene.Node n : previewGrid.getChildren()) {
            if (!(n instanceof VBox tile) || !(tile.getUserData() instanceof Integer idx)) continue;
            tile.setStyle(idx == si ? FOCUSED_TILE : UNFOCUSED_TILE);
        }
    }

    private static String shortName(String s) { return s == null ? "" : s.replaceAll("(?i)\\.fcs$", ""); }

    // ---- copy / export / settings -------------------------------------------

    /** DPI-scaled snapshot on a white background. The old code snapshotted the raw canvas with no
     *  SnapshotParameters, so Copy produced a transparent (black-pasting) image at screen resolution. */
    /**
     * Composite the figure on a WHITE canvas: each panel is rendered via {@code exportImage} (which is
     * already white and cropped) and laid left-to-right with its caption, then a legend column of the
     * TICKED populations is drawn on the right. Compositing — rather than snapshotting the on-screen
     * {@code comparisonBox} — is what removes the dark navy surround the snapshot used to capture, and
     * it lets the legend travel with every Copy/Export by default.
     */
    private WritableImage renderForExport() {
        double scale = ctx == null ? 1.0 : ctx.settings().exportScale();

        List<CytoPlot> panels = new ArrayList<>();
        List<String> caps = new ArrayList<>();
        if (comparisonBox.isVisible() && !comparePlots.isEmpty()) {
            panels.addAll(comparePlots);
            caps.addAll(compareCaptions);
        } else {
            panels.add(plot);
            caps.add(null);
        }

        // Render each panel to a white, cropped image (light mode on for the export, then restored).
        List<javafx.scene.image.Image> imgs = new ArrayList<>();
        for (CytoPlot p : panels) {
            boolean prev = p.isLightMode();
            p.setLightMode(true);
            imgs.add(p.exportImage(scale));
            p.setLightMode(prev);
        }

        double gap = 12 * scale, capH = 22 * scale, pad = 10 * scale;
        double panelsW = 0, panelsH = 0;
        for (javafx.scene.image.Image im : imgs) { panelsW += im.getWidth() + gap; panelsH = Math.max(panelsH, im.getHeight()); }
        panelsW = Math.max(0, panelsW - gap);

        List<String> legend = visibleLegendEntries();
        List<Color> legendColors = visibleLegendColors();
        double legendW = legend.isEmpty() ? 0 : (200 * scale);

        double W = pad * 2 + panelsW + (legendW > 0 ? gap + legendW : 0);
        double H = pad * 2 + panelsH + capH;

        javafx.scene.canvas.Canvas canvas = new javafx.scene.canvas.Canvas(W, H);
        var g = canvas.getGraphicsContext2D();
        g.setFill(Color.WHITE);
        g.fillRect(0, 0, W, H);

        double x = pad;
        for (int i = 0; i < imgs.size(); i++) {
            javafx.scene.image.Image im = imgs.get(i);
            g.drawImage(im, x, pad);
            if (caps.get(i) != null) {
                g.setFill(Color.web("#222222"));
                g.setFont(javafx.scene.text.Font.font(12 * scale));
                g.setTextAlign(javafx.scene.text.TextAlignment.CENTER);
                g.fillText(caps.get(i), x + im.getWidth() / 2, pad + panelsH + 15 * scale);
            }
            x += im.getWidth() + gap;
        }

        if (legendW > 0) drawExportLegend(g, x, pad, legendW, scale, legend, legendColors);

        javafx.scene.SnapshotParameters sp = new javafx.scene.SnapshotParameters();
        sp.setFill(Color.WHITE);
        return canvas.snapshot(sp, null);
    }

    /** Ticked populations only (legend reflects show/hide), in paint order. */
    private List<String> visibleLegendEntries() {
        List<String> out = new ArrayList<>();
        if (popColoring == null) return out;
        for (String n : popColoring.names())
            if (!hiddenPops.contains(stripCount(n))) out.add(n);
        return out;
    }

    private List<Color> visibleLegendColors() {
        List<Color> out = new ArrayList<>();
        if (popColoring == null) return out;
        String[] names = popColoring.names();
        Color[] colors = popColoring.colors();
        for (int i = 0; i < names.length; i++)
            if (!hiddenPops.contains(stripCount(names[i]))) out.add(colors[i]);
        return out;
    }

    private void drawExportLegend(javafx.scene.canvas.GraphicsContext g, double x, double y, double w,
                                  double scale, List<String> names, List<Color> colors) {
        double fs = 12 * scale, row = fs + 6 * scale, sw = 11 * scale;
        g.setFont(javafx.scene.text.Font.font(fs));
        g.setTextAlign(javafx.scene.text.TextAlignment.LEFT);
        g.setFill(Color.web("#222222"));
        g.fillText("Populations", x, y + fs);
        double ly = y + row + 4 * scale;
        for (int i = 0; i < names.size(); i++) {
            g.setFill(colors.get(i));
            g.fillRect(x, ly - sw + 2 * scale, sw, sw);
            g.setFill(Color.web("#222222"));
            g.fillText(names.get(i), x + sw + 6 * scale, ly);
            ly += row;
        }
    }

    @FXML
    private void onCopy() {
        if (embedding == null) return;
        ClipboardContent cc = new ClipboardContent();
        cc.putImage(renderForExport());
        Clipboard.getSystemClipboard().setContent(cc);
        statusLabel.setText(String.format("Copied at %d DPI.", ctx.settings().exportDpi()));
    }

    @FXML
    private void onExport() {
        if (embedding == null) return;
        FileChooser fc = new FileChooser();
        fc.setTitle("Export embedding");
        fc.getExtensionFilters().addAll(
                new FileChooser.ExtensionFilter("PNG image (*.png)", "*.png"),
                new FileChooser.ExtensionFilter("TIFF image (*.tif)", "*.tif"),
                new FileChooser.ExtensionFilter("SVG vector (*.svg)", "*.svg"));
        File f = fc.showSaveDialog(plotHost.getScene().getWindow());
        if (f == null) return;
        String lower = f.getName().toLowerCase();
        int dpi = ctx.settings().exportDpi();
        try {
            if (lower.endsWith(".svg")) {
                exportSvg(f);
            } else {
                java.awt.image.BufferedImage bi =
                        javafx.embed.swing.SwingFXUtils.fromFXImage(renderForExport(), null);
                if (lower.endsWith(".tif") || lower.endsWith(".tiff")) {
                    if (!javax.imageio.ImageIO.write(bi, "tiff", f))
                        throw new java.io.IOException("no TIFF writer available");
                } else {
                    writePngWithDpi(bi, f, dpi);
                }
                statusLabel.setText("Saved " + f.getName() + " at " + dpi + " DPI.");
            }
        } catch (Exception e) {
            statusLabel.setText("Export failed: " + e.getMessage());
        }
    }

    /**
     * SVG export. A single plot writes one vector file. In the side-by-side group view, ask the user
     * whether to write ONE composite SVG (panels laid out in translated groups + legend) or ONE file
     * PER panel with general auto-derived names.
     */
    private void exportSvg(File f) throws Exception {
        boolean multi = comparisonBox.isVisible() && comparePlots.size() > 1;
        if (!multi) {
            Files.writeString(f.toPath(), plot.exportSvg());
            statusLabel.setText("Saved " + f.getName() + " (vector).");
            return;
        }

        ButtonType composite = new ButtonType("Composite (one file)", ButtonBar.ButtonData.OK_DONE);
        ButtonType perPanel = new ButtonType("One per panel", ButtonBar.ButtonData.OTHER);
        Alert ask = new Alert(Alert.AlertType.CONFIRMATION,
                "The comparison has " + comparePlots.size() + " panels. Export as one composite SVG, "
                + "or one SVG file per panel?", composite, perPanel, ButtonType.CANCEL);
        ask.setTitle("SVG export");
        ask.setHeaderText(null);
        AppIcons.theme(ask, window());
        ButtonType choice = ask.showAndWait().orElse(ButtonType.CANCEL);
        if (choice == ButtonType.CANCEL) return;

        String base = f.getName().replaceAll("(?i)\\.svg$", "");
        File dir = f.getParentFile();
        if (choice == perPanel) {
            int n = 0;
            for (int i = 0; i < comparePlots.size(); i++) {
                String tag = compareCaptions.get(i).replaceAll("\\s*\\(.*$", "").replaceAll("[^A-Za-z0-9_-]", "_");
                File out = new File(dir, base + "_" + tag + ".svg");
                Files.writeString(out.toPath(), comparePlots.get(i).exportSvg());
                n++;
            }
            statusLabel.setText("Saved " + n + " SVG file(s) beside " + f.getName() + ".");
        } else {
            Files.writeString(f.toPath(), compositeSvg());
            statusLabel.setText("Saved " + f.getName() + " (composite vector).");
        }
    }

    /** One SVG with each panel's body wrapped in a translated group, captions, and a right-side legend. */
    private String compositeSvg() {
        double gap = 14, capH = 22, pad = 10;
        double panelsW = 0, panelsH = 0;
        for (CytoPlot p : comparePlots) { panelsW += p.svgWidth() + gap; panelsH = Math.max(panelsH, p.svgHeight()); }
        panelsW = Math.max(0, panelsW - gap);

        List<String> legend = visibleLegendEntries();
        List<Color> colors = visibleLegendColors();
        double legendW = legend.isEmpty() ? 0 : 200;
        double W = pad * 2 + panelsW + (legendW > 0 ? gap + legendW : 0);
        double H = pad * 2 + panelsH + capH;

        StringBuilder s = new StringBuilder();
        s.append("<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='")
                .append((int) W).append("' height='").append((int) H).append("' font-family='Segoe UI, sans-serif'>");
        s.append("<rect width='100%' height='100%' fill='white'/>");
        double x = pad;
        for (int i = 0; i < comparePlots.size(); i++) {
            CytoPlot p = comparePlots.get(i);
            s.append("<g transform='translate(").append(x).append(",").append(pad).append(")'>")
                    .append(p.exportSvgBody()).append("</g>");
            s.append("<text x='").append(x + p.svgWidth() / 2.0).append("' y='").append(pad + panelsH + 16)
                    .append("' text-anchor='middle' font-size='12' fill='#222'>")
                    .append(compareCaptions.get(i).replace("&", "&amp;").replace("<", "&lt;")).append("</text>");
            x += p.svgWidth() + gap;
        }
        if (legendW > 0) {
            double ly = pad + 14;
            s.append("<text x='").append(x).append("' y='").append(ly).append("' font-size='12' fill='#222'>Populations</text>");
            ly += 20;
            for (int i = 0; i < legend.size(); i++) {
                s.append("<rect x='").append(x).append("' y='").append(ly - 10).append("' width='11' height='11' fill='")
                        .append(toWeb(colors.get(i))).append("'/>");
                s.append("<text x='").append(x + 17).append("' y='").append(ly).append("' font-size='12' fill='#222'>")
                        .append(legend.get(i).replace("&", "&amp;").replace("<", "&lt;")).append("</text>");
                ly += 18;
            }
        }
        s.append("</svg>");
        return s.toString();
    }

    /** Write a PNG and stamp the real DPI into its pHYs chunk, so Word/Illustrator report e.g. 300 DPI
     *  instead of assuming 96. The pixel dimensions are already scaled by {@code exportScale()}. */
    private static void writePngWithDpi(java.awt.image.BufferedImage bi, File out, int dpi) throws Exception {
        javax.imageio.ImageWriter w = javax.imageio.ImageIO.getImageWritersByFormatName("png").next();
        javax.imageio.ImageWriteParam p = w.getDefaultWriteParam();
        javax.imageio.metadata.IIOMetadata meta =
                w.getDefaultImageMetadata(new javax.imageio.ImageTypeSpecifier(bi), p);
        int pixelsPerMetre = (int) Math.round(dpi / 25.4 * 1000.0);
        javax.imageio.metadata.IIOMetadataNode phys = new javax.imageio.metadata.IIOMetadataNode("pHYs");
        phys.setAttribute("pixelsPerUnitXAxis", String.valueOf(pixelsPerMetre));
        phys.setAttribute("pixelsPerUnitYAxis", String.valueOf(pixelsPerMetre));
        phys.setAttribute("unitSpecifier", "meter");
        javax.imageio.metadata.IIOMetadataNode root =
                new javax.imageio.metadata.IIOMetadataNode("javax_imageio_png_1.0");
        root.appendChild(phys);
        meta.mergeTree("javax_imageio_png_1.0", root);
        try (javax.imageio.stream.ImageOutputStream os = javax.imageio.ImageIO.createImageOutputStream(out)) {
            w.setOutput(os);
            w.write(meta, new javax.imageio.IIOImage(bi, null, meta), p);
        } finally {
            w.dispose();
        }
    }

    @FXML
    private void onSettings() {
        if (ctx != null) CopySettingsController.open(ctx.settings());
    }
}
