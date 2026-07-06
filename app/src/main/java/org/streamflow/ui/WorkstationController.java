package org.streamflow.ui;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.fxml.FXML;
import javafx.geometry.Insets;
import javafx.scene.Scene;
import javafx.scene.control.Alert;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonType;
import javafx.scene.control.ComboBox;
import javafx.scene.control.ContextMenu;
import javafx.scene.control.Label;
import javafx.scene.control.MenuItem;
import javafx.scene.control.SeparatorMenuItem;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.TextArea;
import javafx.scene.control.TreeCell;
import javafx.scene.control.TreeItem;
import javafx.scene.control.TreeView;
import javafx.scene.layout.Priority;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.scene.shape.Circle;
import javafx.stage.FileChooser;
import javafx.stage.Stage;

import java.io.File;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * Workstation — the single-window home of the experiment (Setup is merged in; FCS import is on
 * {@code File ▸ Load FCS…}). One {@link TreeView} shows every sample as a top-level row with its
 * gating hierarchy + live counts/percentages nested underneath, so samples, gates and statistics
 * live in one place instead of three panels. All actions are on the right-click context menu
 * (open, apply-to-all, statistics, keywords, export, copy gate).
 */
public class WorkstationController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Label experimentLabel;
    @FXML private Label statusLabel;
    @FXML private Button helpButton;
    @FXML private Button panelCheckButton;
    @FXML private Button exportGatingMlButton;
    @FXML private TreeView<Object> tree;

    // §9 overview tab
    @FXML private javafx.scene.control.TabPane mainTabs;
    @FXML private javafx.scene.control.Tab overviewTab;
    @FXML private ComboBox<String> overviewXCombo;
    @FXML private ComboBox<String> overviewYCombo;
    @FXML private javafx.scene.layout.FlowPane overviewGrid;
    @FXML private Label overviewStatusLabel;

    private AppContext ctx;
    private final Set<String> expandedSamples = new HashSet<>();   // remember expansion across rebuilds
    private double medianEvents = 0;     // for the QC indicator (event-count deviation)

    @FXML
    public void initialize() {
        tree.setShowRoot(false);
        tree.setCellFactory(tv -> new TreeCell<>() {
            @Override protected void updateItem(Object v, boolean empty) {
                super.updateItem(v, empty);
                if (empty || v == null) { setText(null); setContextMenu(null); return; }
                if (v instanceof String sample) {            // sample row
                    setText(sampleLabel(sample));
                    setGraphic(qcDot(sample));
                    setContextMenu(sampleMenu(sample));
                } else if (v instanceof PopNode n) {         // population row
                    setText(popLabel(n));
                    setGraphic(null);
                    setContextMenu(popMenu(getTreeItem()));
                }
            }
        });

        tree.setOnMouseClicked(e -> {
            if (e.getClickCount() != 2) return;
            TreeItem<Object> it = tree.getSelectionModel().getSelectedItem();
            if (it == null) return;
            if (it.getValue() instanceof String sample) openSample(sample, null);
            else if (it.getValue() instanceof PopNode n)
                openSample(sampleOf(it), n.isRoot() ? null : n);
        });

        // Delete key removes the selected population everywhere (no confirm; undo is in the graph window).
        tree.setOnKeyPressed(e -> {
            if (e.getCode() == javafx.scene.input.KeyCode.DELETE) {
                TreeItem<Object> it = tree.getSelectionModel().getSelectedItem();
                if (it != null && it.getValue() instanceof PopNode n && !n.isRoot())
                    removePopulation(sampleOf(it), n);
            }
        });
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        ctx.workspace().sampleNames().addListener(
                (javafx.collections.ListChangeListener<String>) c -> rebuild());
        ctx.workspace().addTreeChangeListener(this::rebuild);
        ctx.workspace().addDataChangeListener(s -> rebuild());
        // §9: refresh overview when the Overview tab is selected
        if (mainTabs != null) {
            mainTabs.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> {
                if (b == overviewTab) refreshOverview();
            });
        }
        rebuild();
    }

    // ---- tree construction ---------------------------------------------------

    private void rebuild() {
        if (ctx == null) return;
        // capture currently-expanded sample names
        TreeItem<Object> oldRoot = tree.getRoot();
        if (oldRoot != null) {
            for (TreeItem<Object> s : oldRoot.getChildren())
                if (s.getValue() instanceof String name) {
                    if (s.isExpanded()) expandedSamples.add(name); else expandedSamples.remove(name);
                }
        }
        // QC reference: median event count across samples.
        int[] counts = ctx.workspace().sampleNames().stream()
                .mapToInt(s -> Math.max(0, ctx.workspace().eventCount(s))).sorted().toArray();
        medianEvents = counts.length > 0 ? counts[counts.length / 2] : 0;

        TreeItem<Object> root = new TreeItem<>("root");
        for (String sample : ctx.workspace().sampleNames()) {
            TreeItem<Object> sItem = new TreeItem<>(sample);
            sItem.setExpanded(expandedSamples.contains(sample));
            PopNode popRoot = ctx.workspace().hasTree(sample)
                    ? ctx.workspace().treeFor(sample) : new PopNode(null, null);
            sItem.getChildren().add(buildPop(popRoot));
            root.getChildren().add(sItem);
        }
        tree.setRoot(root);
        int n = ctx.workspace().sampleNames().size();
        int chans = ctx.workspace().channelNames().size();
        experimentLabel.setText(n == 0 ? "—" : n + " sample(s) · " + chans + " channel(s)");
        if (n == 0) statusLabel.setText("Load FCS (File ▸ Load FCS…) to begin.");
    }

    private TreeItem<Object> buildPop(PopNode n) {
        TreeItem<Object> item = new TreeItem<>(n);
        item.setExpanded(true);
        for (PopNode ch : n.children) item.getChildren().add(buildPop(ch));
        return item;
    }

    private String sampleLabel(String sample) {
        StringBuilder sb = new StringBuilder(sample);
        int ev = ctx.workspace().eventCount(sample);
        if (ev >= 0) sb.append("   ").append(String.format("%,d", ev)).append(" events");
        if (ctx.workspace().hasTree(sample)) {
            int gates = ctx.workspace().treeFor(sample).selfAndDescendants().size() - 1;
            if (gates > 0) sb.append("   ·   ").append(gates).append(" gate").append(gates == 1 ? "" : "s");
        }
        return sb.toString();
    }

    /** QC indicator (restored from the old Setup view): event-count deviation from the median.
     *  Clickable — opens the per-FCS diagnostics window (keywords + parameters + summary). */
    private Circle qcDot(String sample) {
        int ev = ctx.workspace().eventCount(sample);
        Circle dot = new Circle(5);
        String verdict;
        if (ev < 0 || medianEvents <= 0) {
            dot.setFill(Color.web("#3A4A5E"));
            verdict = "QC: not yet computed" + (ev >= 0 ? String.format("  (%,d events)", ev) : "");
        } else {
            double dev = Math.abs(ev - medianEvents) / medianEvents;
            dot.setFill(dev < 0.20 ? Color.web("#4CAF50") : dev < 0.50 ? Color.web("#FFC107") : Color.web("#F44336"));
            String status = dev < 0.20 ? "PASS" : dev < 0.50 ? "WARN" : "FAIL";
            verdict = String.format("QC: %s — %.0f%% from median (%,d events)", status, dev * 100, ev);
        }
        dot.setStyle("-fx-cursor:hand;");
        javafx.scene.control.Tooltip.install(dot,
                new javafx.scene.control.Tooltip(verdict + "\nClick for full FCS diagnostics."));
        dot.setOnMouseClicked(e -> { FcsDiagnosticsController.open(ctx, sample, verdict); e.consume(); });
        return dot;
    }

    private static String popLabel(PopNode n) {
        if (n.isRoot()) return "All Events" + (n.count >= 0 ? "   —   " + n.count : "");
        String pct = Double.isNaN(n.parentPct) ? "" : String.format("   (%.1f%%)", n.parentPct);
        // "✎ … (edited)" flags a gate changed in a graph window but not yet re-applied to all samples.
        String editMark = n.edited ? "✎ " : "";
        String editTail = n.edited ? "   • edited" : "";
        return editMark + n.name() + (n.count >= 0 ? "   —   " + n.count + pct : "") + editTail;
    }

    // ---- context menus -------------------------------------------------------

    private ContextMenu sampleMenu(String sample) {
        ContextMenu m = new ContextMenu();
        MenuItem open = new MenuItem("Open in Graph Window");
        open.setOnAction(e -> openSample(sample, null));
        MenuItem stats = new MenuItem("Compute statistics…");
        stats.setOnAction(e -> computeSampleStats(sample));
        MenuItem applyAll = new MenuItem("Apply gates → all samples");
        applyAll.setOnAction(e -> applyAll(sample));
        MenuItem kw = new MenuItem("FCS keywords…");
        kw.setOnAction(e -> showKeywords(sample));
        MenuItem exp = new MenuItem("Export FCS…");
        exp.setOnAction(e -> exportFcs(sample));
        m.getItems().addAll(open, stats, new SeparatorMenuItem(), applyAll);
        if (ctx.workspace().gateClipboard() != null) {
            MenuItem paste = new MenuItem("Paste gate (under All Events)");
            paste.setOnAction(e -> pasteGate(sample, ctx.workspace().treeFor(sample)));
            m.getItems().add(paste);
        }
        m.getItems().addAll(new SeparatorMenuItem(), kw, exp);
        return m;
    }

    private ContextMenu popMenu(TreeItem<Object> item) {
        PopNode n = (PopNode) item.getValue();
        String sample = sampleOf(item);
        ContextMenu m = new ContextMenu();
        MenuItem open = new MenuItem(n.isRoot() ? "Open in Graph Window" : "Open population in Graph Window");
        open.setOnAction(e -> openSample(sample, n.isRoot() ? null : n));
        MenuItem stats = new MenuItem("Compute statistics…");
        stats.setOnAction(e -> computeStats(sample, n.isRoot() ? null : n));
        MenuItem cc = new MenuItem("Cell cycle analysis…");
        cc.setOnAction(e -> runCellCycle(sample, n));
        m.getItems().addAll(open, stats, cc);
        if (!n.isRoot()) {
            MenuItem copy = new MenuItem("Copy gate");
            copy.setOnAction(e -> copyGate(n));
            MenuItem applyAll = new MenuItem("Apply gate → all samples");
            applyAll.setOnAction(e -> applyGateToAll(sample, n));
            MenuItem excl = new MenuItem(n.gate.invert ? "Clear exclusion gate" : "Set as exclusion gate");
            excl.setOnAction(e -> toggleExclusion(sample, n));
            MenuItem rm = new MenuItem("Remove gate");
            rm.setOnAction(e -> removePopulation(sample, n));
            m.getItems().addAll(new SeparatorMenuItem(), copy, applyAll, excl, new SeparatorMenuItem(), rm);
        }
        if (ctx.workspace().gateClipboard() != null) {
            MenuItem paste = new MenuItem("Paste gate here");
            paste.setOnAction(e -> pasteGate(sample, n));
            m.getItems().add(paste);
        }
        return m;
    }

    private String sampleOf(TreeItem<Object> item) {
        for (TreeItem<Object> n = item; n != null; n = n.getParent())
            if (n.getValue() instanceof String s) return s;
        return null;
    }

    // ---- actions -------------------------------------------------------------

    private void openSample(String sample, PopNode focus) {
        if (sample == null || ctx == null) return;
        // §14: focus an already-open graph window instead of spawning a duplicate
        javafx.stage.Stage existing = ctx.workspace().openWindowFor(sample);
        if (existing != null && focus == null) { existing.toFront(); return; }
        List<String> all = List.copyOf(ctx.workspace().sampleNames());
        int idx = Math.max(0, all.indexOf(sample));
        if (focus == null || focus.isRoot()) {
            GraphWindowController.open(ctx, all, idx, ctx.workspace().channelNames());
        } else {
            GraphWindowController.openChild(ctx, sample, focus);
        }
    }

    // ---- §9 overview thumbnail grid -----------------------------------------

    @FXML
    private void onOverviewRefresh() { refreshOverview(); }

    private void refreshOverview() {
        if (ctx == null || overviewGrid == null) return;
        List<String> channels = ctx.workspace().channelNames();
        // Populate axis combos on first use (or after channels change).
        if (overviewXCombo != null && (overviewXCombo.getItems().isEmpty()
                || overviewXCombo.getItems().size() != channels.size())) {
            List<String> yOpts = new ArrayList<>(); yOpts.add("(Histogram)"); yOpts.addAll(channels);
            overviewXCombo.setItems(FXCollections.observableArrayList(channels));
            overviewYCombo.setItems(FXCollections.observableArrayList(yOpts));
            selectPreferred(overviewXCombo, channels, "FSC-A");
            selectPreferred(overviewYCombo, yOpts, "SSC-A");
        }
        overviewGrid.getChildren().clear();
        String xch = overviewXCombo != null && overviewXCombo.getValue() != null
                ? overviewXCombo.getValue() : (channels.isEmpty() ? null : channels.get(0));
        String ych = overviewYCombo != null && overviewYCombo.getValue() != null
                ? overviewYCombo.getValue() : (channels.size() > 1 ? channels.get(1) : "(Histogram)");
        if (xch == null) { if (overviewStatusLabel != null) overviewStatusLabel.setText("Load FCS first."); return; }
        boolean hist = "(Histogram)".equals(ych);
        CytoPlot.Scale xs = scaleFor(xch), ys = hist ? CytoPlot.Scale.LINEAR : scaleFor(ych);
        final String finalX = xch, finalY = ych;
        final CytoPlot.Scale finalXs = xs, finalYs = ys;

        List<String> all = new ArrayList<>(ctx.workspace().sampleNames());
        if (overviewStatusLabel != null) overviewStatusLabel.setText("Loading " + all.size() + " sample(s)…");

        // Add a placeholder card for every sample immediately, then fill data as it arrives.
        for (String sample : all) {
            CytoPlot mini = new CytoPlot();
            mini.setMinSize(180, 140); mini.setPrefSize(180, 140); mini.setMaxSize(180, 140);
            mini.setChannelLabeler(ch -> ctx.aliases().label(ch));
            if (ctx.workspace().hasTree(sample))
                for (PopNode ch : ctx.workspace().treeFor(sample).children) mini.addGate(ch.gate);

            Label cap = new Label(shortName(sample));
            cap.setStyle("-fx-font-size:10; -fx-text-fill:#8AABB5;");
            javafx.scene.layout.VBox card = new javafx.scene.layout.VBox(2, cap, mini);
            card.setStyle("-fx-border-color:#1E3350; -fx-border-radius:3; -fx-padding:4; -fx-background-color:#0D1B2A; -fx-background-radius:3;");
            card.setOnMouseClicked(e -> { if (e.getClickCount() == 2) openSample(sample, null); });
            overviewGrid.getChildren().add(card);

            // Set data immediately if cached; otherwise fetch in background.
            EventData cached = ctx.workspace().data(sample);
            if (cached != null) {
                mini.setData(cached);
                mini.setView(finalX, hist ? null : finalY, finalXs, finalYs, "pseudocolor");
            } else {
                ensureData(sample, () -> {
                    EventData d = ctx.workspace().data(sample);
                    if (d != null) {
                        mini.setData(d);
                        mini.setView(finalX, hist ? null : finalY, finalXs, finalYs, "pseudocolor");
                    }
                });
            }
        }
        if (overviewStatusLabel != null)
            overviewStatusLabel.setText(all.isEmpty()
                    ? "Load FCS first." : all.size() + " sample(s) — double-click to open");
    }

    /** Scatter/Time/Width → Linear; everything else → Logicle (matches GraphWindowController). */
    private static CytoPlot.Scale scaleFor(String channel) {
        if (channel == null) return CytoPlot.Scale.LINEAR;
        return channel.matches("(?i).*(FSC|SSC|Time|Width).*")
                ? CytoPlot.Scale.LINEAR : CytoPlot.Scale.LOGICLE;
    }

    private static String shortName(String s) {
        if (s == null) return "";
        int slash = Math.max(s.lastIndexOf('/'), s.lastIndexOf('\\'));
        String base = slash >= 0 ? s.substring(slash + 1) : s;
        return base.length() > 22 ? base.substring(0, 20) + "…" : base;
    }

    private static void selectPreferred(ComboBox<String> combo, List<String> opts, String pref) {
        opts.stream().filter(c -> c.equalsIgnoreCase(pref)).findFirst()
                .or(() -> opts.isEmpty() ? java.util.Optional.empty() : java.util.Optional.of(opts.get(0)))
                .ifPresent(v -> combo.getSelectionModel().select(v));
    }

    private void applyAll(String sample) {
        if (ctx == null) return;
        List<String> others = ctx.workspace().sampleNames().stream()
                .filter(s -> !s.equals(sample)).toList();
        if (others.isEmpty()) { statusLabel.setText("Only one sample — nothing to apply to."); return; }
        Alert c = new Alert(Alert.AlertType.CONFIRMATION);
        c.setTitle("Apply gates to all samples");
        c.setHeaderText("Apply all gates from '" + sample + "' to " + others.size() + " other sample(s)?");
        c.setContentText("Replaces the gating tree on those samples. Counts recompute when each opens.");
        if (c.showAndWait().orElse(ButtonType.CANCEL) != ButtonType.OK) return;
        PopNode src = ctx.workspace().treeFor(sample);
        for (String o : others) ctx.workspace().replaceTree(o, src.cloneTree(null));
        ctx.workspace().notifyTreeChanged();
        statusLabel.setText("Gates applied to " + others.size() + " sample(s).");
    }

    private void removePopulation(String sample, PopNode n) {
        if (n.parent != null) n.parent.children.remove(n);
        ctx.workspace().notifyTreeChanged();
        statusLabel.setText("Removed " + n.name() + " from " + sample + ".");
    }

    /** Copy a gate (and its sub-tree) to the shared clipboard for pasting onto another sample/population. */
    private void copyGate(PopNode n) {
        ctx.workspace().setGateClipboard(n.cloneTree(null));
        ctx.workspace().notifyTreeChanged();   // refresh menus so "Paste gate" appears
        statusLabel.setText("Copied gate '" + n.name() + "' (right-click a sample/population → Paste gate).");
    }

    /** Paste the clipboard gate sub-tree under {@code target} (its data-space geometry transfers directly). */
    private void pasteGate(String sample, PopNode target) {
        PopNode clip = ctx.workspace().gateClipboard();
        if (clip == null || target == null) return;
        PopNode copy = clip.cloneTree(target);
        target.children.add(copy);
        ctx.workspace().notifyTreeChanged();
        statusLabel.setText("Pasted '" + copy.name() + "' under " + target.name() + " in " + sample + ".");
    }

    /** "Copy analysis to all": clone this gate sub-tree onto every other sample (under All Events). */
    private void applyGateToAll(String sample, PopNode n) {
        List<String> others = ctx.workspace().sampleNames().stream().filter(s -> !s.equals(sample)).toList();
        if (others.isEmpty()) { statusLabel.setText("Only one sample — nothing to apply to."); return; }
        n.edited = false;
        for (String o : others) {
            PopNode root = ctx.workspace().treeFor(o);
            // Replace an existing same-named top-level gate instead of stacking a duplicate (BUG-15).
            root.children.removeIf(c -> java.util.Objects.equals(c.name(), n.name()));
            root.children.add(n.cloneTree(root));
        }
        ctx.workspace().notifyTreeChanged();
        statusLabel.setText("Applied gate '" + n.name() + "' to " + others.size() + " other sample(s).");
    }

    /** Toggle a gate between inclusion and exclusion (NOT-gate). Counts recompute when the sample opens. */
    private void toggleExclusion(String sample, PopNode n) {
        n.gate.invert = !n.gate.invert;
        ctx.workspace().notifyTreeChanged();
        statusLabel.setText(n.name() + " is now " + (n.gate.invert ? "an EXCLUSION gate" : "an inclusion gate")
                + " — reopen " + sample + " to recompute counts.");
    }

    /** Per-POPULATION statistics for a sample (All Events + every gate) — count, %total, per-channel median.
     *  This is computed in Java from the gating tree, so drawn populations (e.g. P1) show up. */
    private void computeSampleStats(String sample) {
        if (ctx == null || sample == null) return;
        ensureData(sample, () -> {
            EventData d = ctx.workspace().data(sample);
            if (d == null) { statusLabel.setText("Could not load events for " + sample + "."); return; }
            List<String> fluor = new ArrayList<>();
            for (String ch : d.channels()) if (!isScatter(ch)) fluor.add(ch);
            PopNode root = ctx.workspace().hasTree(sample) ? ctx.workspace().treeFor(sample) : new PopNode(null, null);
            int total = d.rows();

            ObservableList<String[]> rows = FXCollections.observableArrayList();
            for (PopNode n : root.selfAndDescendants()) {
                boolean[] keep = new boolean[d.rows()];
                java.util.Arrays.fill(keep, true);
                for (CytoPlot.Gate g : n.chain()) {
                    boolean[] mk = CytoPlot.mask(d, g);
                    for (int i = 0; i < keep.length && i < mk.length; i++) keep[i] &= mk[i];
                }
                EventData sub = n.isRoot() ? d : d.subset(keep);
                int cnt = sub.rows();
                String[] row = new String[3 + fluor.size()];
                row[0] = n.name();
                row[1] = String.format("%,d", cnt);
                row[2] = total > 0 ? String.format("%.1f%%", 100.0 * cnt / total) : "—";
                for (int j = 0; j < fluor.size(); j++) row[3 + j] = fmt(median(sub, sub.indexOf(fluor.get(j))));
                rows.add(row);
            }

            TableView<String[]> table = new TableView<>(rows);
            List<String> heads = new ArrayList<>(List.of("Population", "Count", "% Total"));
            heads.addAll(fluor);
            for (int i = 0; i < heads.size(); i++) {
                final int idx = i;
                TableColumn<String[], String> c = new TableColumn<>(heads.get(i));
                c.setCellValueFactory(cd -> new ReadOnlyStringWrapper(idx < cd.getValue().length ? cd.getValue()[idx] : ""));
                c.setPrefWidth(i == 0 ? 160 : 95);
                table.getColumns().add(c);
            }
            VBox box = new VBox(new Label(sample + " — " + rows.size() + " population(s), median MFI per channel"), table);
            box.setSpacing(6); box.setPadding(new Insets(12));
            VBox.setVgrow(table, Priority.ALWAYS);
            Stage st = new Stage();
            st.setTitle("Statistics — " + sample);
            st.setScene(new Scene(box, 760, 520));
            st.show();
            if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sample, "Per-population statistics");
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

    /** Right-click → Cell cycle: fit the gated population's DNA channel, lay down adjustable
     *  interval gates (G0/G1, S, G2/M) on its histogram, and open it in a graph window to export. */
    private void runCellCycle(String sample, PopNode node) {
        if (ctx == null || sample == null) return;
        String dna = guessDnaChannel();
        if (dna == null) {
            List<String> chans = ctx.workspace().channelNames();
            if (chans.isEmpty()) { statusLabel.setText("Load FCS first."); return; }
            javafx.scene.control.ChoiceDialog<String> dlg = new javafx.scene.control.ChoiceDialog<>(chans.get(0), chans);
            dlg.setTitle("Cell cycle analysis");
            dlg.setHeaderText("Select the DNA-content channel (PI / DAPI / 7-AAD / Hoechst)");
            dlg.setContentText("Channel:");
            var pick = dlg.showAndWait();
            if (pick.isEmpty()) return;
            dna = pick.get();
        }
        final String dnaCh = dna;
        ensureData(sample, () -> {
            EventData d = ctx.workspace().data(sample);
            if (d == null) { statusLabel.setText("Could not load events for " + sample + "."); return; }
            int col = d.indexOf(dnaCh);
            if (col < 0) { statusLabel.setText("Channel not found: " + dnaCh); return; }
            boolean[] keep = new boolean[d.rows()];
            java.util.Arrays.fill(keep, true);
            for (CytoPlot.Gate g : node.chain()) {
                boolean[] mk = CytoPlot.mask(d, g);
                for (int k = 0; k < keep.length && k < mk.length; k++) keep[k] &= mk[k];
            }
            ArrayNode vals = JSON.createArrayNode();
            for (int r = 0; r < d.rows(); r++) {
                if (!keep[r]) continue;
                float v = d.get(r, col);
                if (v > 0 && Float.isFinite(v)) vals.add(v);
            }
            if (vals.size() < 100) { statusLabel.setText("Too few events in " + node.name() + " for cell-cycle fitting."); return; }
            ObjectNode args = JSON.createObjectNode();
            args.put("channel", dnaCh);
            args.put("model", "watson");
            args.set("values", vals);
            statusLabel.setText("Fitting cell cycle on " + node.name() + " (" + dnaCh + ")…");
            ctx.jobs().run(ctx.bridge().command("run_cell_cycle", args), res -> applyCellCycleGates(sample, node, dnaCh, res));
        });
    }

    private void applyCellCycleGates(String sample, PopNode node, String dnaCh, com.fasterxml.jackson.databind.JsonNode res) {
        com.fasterxml.jackson.databind.JsonNode b = res.path("boundaries");
        double lo = b.path("lo").asDouble(), g1 = b.path("g1_hi").asDouble(),
               g2 = b.path("g2_lo").asDouble(), hi = b.path("hi").asDouble();
        node.viewX = dnaCh; node.viewY = "(Histogram)"; node.viewXScale = "Linear"; node.viewYScale = "Linear";
        // Replace any previous cell-cycle phase gates so re-running doesn't duplicate them.
        node.children.removeIf(ch -> ch.gate != null
                && (ch.gate.name.equals("G0/G1") || ch.gate.name.equals("S") || ch.gate.name.equals("G2/M")));
        addPhaseGate(node, "G0/G1", dnaCh, lo, g1);
        addPhaseGate(node, "S", dnaCh, g1, g2);
        addPhaseGate(node, "G2/M", dnaCh, g2, hi);
        ctx.workspace().notifyTreeChanged();
        com.fasterxml.jackson.databind.JsonNode ph = res.path("phases");
        statusLabel.setText(String.format(
                "Cell cycle on %s: G0/G1 %.1f%% · S %.1f%% · G2/M %.1f%% (R²=%.3f) — adjust the interval gates, then Copy/SVG to export.",
                node.name(), ph.path("G0G1").asDouble(), ph.path("S").asDouble(), ph.path("G2M").asDouble(),
                res.path("fit").path("r2").asDouble()));
        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sample, "Cell cycle on " + node.name() + " (" + dnaCh + ")");
        openSample(sample, node.isRoot() ? null : node);
    }

    private void addPhaseGate(PopNode parent, String name, String ch, double lo, double hi) {
        // "line" gates render as thin boundary dividers (not full-height filled chunks).
        CytoPlot.Gate g = new CytoPlot.Gate(name, "line", ch, null, new double[]{lo, hi}, null);
        parent.children.add(new PopNode(g, parent));
    }

    private String guessDnaChannel() {
        for (String c : ctx.workspace().channelNames())
            if (c.toUpperCase().matches(".*(PI|DAPI|7.?AAD|HOECHST|DNA|FXCYCLE|DRAQ).*")) return c;
        return null;
    }

    /** #24 — per-channel mean/median/CV for a sample (or a gated population), in a dialog. */
    private void computeStats(String sample, PopNode focus) {
        if (ctx == null || sample == null) return;
        ensureData(sample, () -> {
            EventData d = ctx.workspace().data(sample);
            if (d == null) { statusLabel.setText("Could not load events for " + sample + "."); return; }
            EventData scope = d;
            String title = sample + " — All Events";
            if (focus != null && !focus.isRoot()) {
                boolean[] keep = new boolean[d.rows()];
                java.util.Arrays.fill(keep, true);
                for (CytoPlot.Gate g : focus.chain()) {
                    boolean[] mk = CytoPlot.mask(d, g);
                    for (int i = 0; i < keep.length && i < mk.length; i++) keep[i] &= mk[i];
                }
                scope = d.subset(keep);
                title = sample + " — " + focus.name();
            }
            showStatsDialog(title, scope);
        });
    }

    private void showStatsDialog(String title, EventData d) {
        ObservableList<String[]> rows = FXCollections.observableArrayList();
        for (String ch : d.channels()) {
            int col = d.indexOf(ch);
            int n = d.rows();
            double[] vals = new double[n];
            for (int r = 0; r < n; r++) vals[r] = d.get(r, col);
            java.util.Arrays.sort(vals);
            double mean = 0; for (double v : vals) mean += v; mean = n > 0 ? mean / n : 0;
            double median = n == 0 ? 0 : (n % 2 == 1 ? vals[n / 2] : (vals[n / 2 - 1] + vals[n / 2]) / 2);
            double sd = 0; for (double v : vals) sd += (v - mean) * (v - mean);
            sd = n > 1 ? Math.sqrt(sd / (n - 1)) : 0;
            double cv = mean != 0 ? 100.0 * sd / Math.abs(mean) : 0;
            rows.add(new String[]{ch, fmt(median), fmt(mean), fmt(sd), String.format("%.1f%%", cv)});
        }
        TableView<String[]> table = new TableView<>(rows);
        String[] heads = {"Channel", "Median", "Mean", "SD", "CV"};
        for (int i = 0; i < heads.length; i++) {
            final int idx = i;
            TableColumn<String[], String> c = new TableColumn<>(heads[i]);
            c.setCellValueFactory(cd -> new ReadOnlyStringWrapper(cd.getValue()[idx]));
            c.setPrefWidth(i == 0 ? 200 : 110);
            table.getColumns().add(c);
        }
        VBox box = new VBox(new Label(d.rows() + " events"), table);
        box.setSpacing(6); box.setPadding(new Insets(12));
        VBox.setVgrow(table, Priority.ALWAYS);
        Stage st = new Stage();
        st.setTitle("Statistics — " + title);
        st.setScene(new Scene(box, 640, 520));
        st.show();
        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, title, "Computed channel statistics");
    }

    private static String fmt(double v) {
        if (Math.abs(v) >= 100000) return String.format("%.2e", v);
        return String.format("%.1f", v);
    }

    private void showKeywords(String sample) {
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        ctx.jobs().run(ctx.bridge().command("get_metadata", args), result -> {
            ObservableList<String[]> krows = FXCollections.observableArrayList();
            result.path("keywords").fields().forEachRemaining(e -> krows.add(new String[]{e.getKey(), e.getValue().asText()}));
            krows.sort((a, b) -> a[0].compareToIgnoreCase(b[0]));
            TableView<String[]> table = new TableView<>(krows);
            TableColumn<String[], String> k = new TableColumn<>("Keyword"); k.setPrefWidth(240);
            k.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[0]));
            TableColumn<String[], String> val = new TableColumn<>("Value"); val.setPrefWidth(380);
            val.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[1]));
            table.getColumns().addAll(List.of(k, val));
            VBox box = new VBox(table); box.setPadding(new Insets(12));
            VBox.setVgrow(table, Priority.ALWAYS);
            Stage st = new Stage();
            st.setTitle("FCS Keywords — " + sample + " (" + result.path("count").asInt() + ")");
            st.setScene(new Scene(box, 680, 520));
            st.show();
        });
    }

    private void exportFcs(String sample) {
        FileChooser fc = new FileChooser();
        fc.setTitle("Export FCS 3.1");
        fc.setInitialFileName(sample.replaceAll("(?i)\\.fcs$", "") + "_export.fcs");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("FCS files (*.fcs)", "*.fcs"));
        File f = fc.showSaveDialog(window());
        if (f == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        args.put("compensated", false);
        ctx.jobs().run(ctx.bridge().command("export_fcs", args),
                r -> statusLabel.setText(String.format("FCS exported: %,d events × %d channels → %s",
                        r.path("n").asInt(), r.path("channels").asInt(), f.getName())));
    }

    @FXML
    private void onPanelCheck() {
        if (ctx == null) return;
        List<String> channels = ctx.workspace().channelNames();
        if (channels.isEmpty()) { statusLabel.setText("Load FCS first."); return; }
        List<String> labels = new ArrayList<>();
        for (String ch : channels) {
            labels.add(ch);
            String t = ctx.aliases().target(ch);
            if (t != null && !t.isBlank()) labels.add(t);
        }
        List<SpectralReference.Conflict> conflicts = SpectralReference.conflicts(labels, 30);
        StringBuilder sb = new StringBuilder();
        long matched = labels.stream().filter(l -> SpectralReference.matchFluor(l) != null).count();
        sb.append(matched).append(" channel label(s) matched to known fluorochromes.\n\n");
        if (conflicts.isEmpty()) sb.append("No emission conflicts within 30 nm — panel looks spectrally clean.");
        else {
            sb.append(conflicts.size()).append(" potential spillover pair(s) (≤30 nm apart):\n\n");
            for (SpectralReference.Conflict c : conflicts)
                sb.append(String.format("• %s (%s, %dnm)  ↔  %s (%s, %dnm)  — Δ%dnm%n",
                        c.chanA(), c.fluorA(), c.emA(), c.chanB(), c.fluorB(), c.emB(), Math.abs(c.emA() - c.emB())));
        }
        TextArea area = new TextArea(sb.toString());
        area.setEditable(false); area.setWrapText(true);
        VBox box = new VBox(area); box.setPadding(new Insets(12));
        VBox.setVgrow(area, Priority.ALWAYS);
        Stage st = new Stage();
        st.setTitle("Panel spectral check (" + conflicts.size() + " conflict(s))");
        st.setScene(new Scene(box, 560, 420));
        st.show();
    }

    @FXML
    private void onExportGatingMl() {
        if (ctx == null) return;
        ObjectNode gatesNode = serializeGates(ctx.workspace());
        if (gatesNode.isEmpty()) { statusLabel.setText("No gates to export. Draw gates first."); return; }
        FileChooser fc = new FileChooser();
        fc.setTitle("Export GatingML 2.0");
        fc.setInitialFileName("gates.xml");
        fc.getExtensionFilters().add(new FileChooser.ExtensionFilter("GatingML 2.0 (*.xml)", "*.xml"));
        File f = fc.showSaveDialog(window());
        if (f == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("file", f.getAbsolutePath().replace('\\', '/'));
        args.set("gates", gatesNode);
        ctx.jobs().run(ctx.bridge().command("save_gatingml", args),
                r -> statusLabel.setText("GatingML exported: " + r.path("n_gates").asInt() + " gate(s) → " + f.getName()));
    }

    @FXML
    private void onHelp() {
        String msg =
            "THE WORKSTATION\n" +
            "This is the home of your experiment. Every loaded sample is a row; expand it to see its " +
            "gating hierarchy with live event counts and percentages — samples, gates and statistics in one place.\n\n" +
            "DO THINGS BY RIGHT-CLICK\n" +
            "• Right-click a SAMPLE → Open in Graph Window, Compute statistics, Apply gates → all samples, " +
            "FCS keywords, Export FCS.\n" +
            "• Right-click a POPULATION → Open it in a Graph Window, Compute statistics for just that population, " +
            "or Remove it.\n" +
            "• Double-click a sample or population to open it.\n\n" +
            "LOADING DATA\n" +
            "File ▸ Load FCS… imports a folder. Gates you draw in any Graph Window appear here instantly.\n\n" +
            "Experiment-wide tools (Panel check, Export GatingML) are in the toolbar above.";
        Alert a = new Alert(Alert.AlertType.INFORMATION);
        a.setTitle("About the Workstation");
        a.setHeaderText("One window: samples, gating trees and statistics");
        TextArea ta = new TextArea(msg);
        ta.setEditable(false); ta.setWrapText(true); ta.setPrefSize(560, 360);
        a.getDialogPane().setContent(ta);
        a.showAndWait();
    }

    // ---- helpers -------------------------------------------------------------

    /** Ensure the sample's events are cached (fetch from the engine if needed), then run {@code onReady}. */
    private void ensureData(String sample, Runnable onReady) {
        EventData cached = ctx.workspace().data(sample);
        if (cached != null && cached.rows() > 0) { onReady.run(); return; }
        statusLabel.setText("Loading " + sample + "…");
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
                onReady.run();
            } catch (Exception ex) {
                statusLabel.setText("Could not load events: " + ex.getMessage());
            }
        });
    }

    private ObjectNode serializeGates(WorkspaceModel ws) {
        ObjectNode out = JSON.createObjectNode();
        for (String sample : ws.samples()) {
            ArrayNode arr = JSON.createArrayNode();
            PopNode root = ws.treeFor(sample);
            Map<PopNode, String> ids = new LinkedHashMap<>();
            int seq = 0;
            for (PopNode n : root.selfAndDescendants()) if (!n.isRoot()) ids.put(n, sample + "_G" + (++seq));
            for (PopNode n : root.selfAndDescendants()) {
                if (n.isRoot()) continue;
                CytoPlot.Gate g = n.gate;
                ObjectNode gn = JSON.createObjectNode();
                gn.put("id", ids.get(n));
                gn.put("name", g.name);
                if (n.parent != null && !n.parent.isRoot()) gn.put("parent_id", ids.get(n.parent));
                gn.put("type", g.type);
                gn.put("x_channel", g.xChan);
                if (g.yChan != null && !g.yChan.isBlank()) gn.put("y_channel", g.yChan);
                ArrayNode xs = gn.putArray("xs"); if (g.xs != null) for (double x : g.xs) xs.add(x);
                ArrayNode ys = gn.putArray("ys"); if (g.ys != null) for (double y : g.ys) ys.add(y);
                arr.add(gn);
            }
            if (arr.size() > 0) out.set(sample, arr);
        }
        return out;
    }

    private javafx.stage.Window window() {
        return tree.getScene() == null ? null : tree.getScene().getWindow();
    }
}
