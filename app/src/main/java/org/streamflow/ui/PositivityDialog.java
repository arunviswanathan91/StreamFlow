package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.geometry.Insets;
import javafx.geometry.Pos;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonType;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Dialog;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.TextField;
import javafx.scene.control.ToggleButton;
import javafx.scene.control.Tooltip;
import javafx.scene.control.cell.ComboBoxTableCell;
import javafx.scene.layout.FlowPane;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.StackPane;
import javafx.scene.layout.VBox;
import javafx.scene.paint.Color;
import javafx.stage.Window;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.function.Consumer;

/**
 * Set a channel's positive/negative threshold against its CONTROLS, not against the data itself.
 *
 * The unstained control shows where negative sits; the single stain shows where positive sits; an FMO,
 * when present, shows where the spread of every other colour pushes the negative boundary. All three
 * are overlaid as peak-normalised histograms on one axis with a draggable threshold line.
 *
 * Unstained and single stain are <b>required</b>. If they are missing we refuse rather than quietly
 * fall back to Otsu-on-the-data — an Otsu split of a stained sample is not a control-derived threshold,
 * and presenting it as one would be a lie in the methods section. FMO is optional and its absence is
 * stated.
 *
 * Auto seeds the line from the engine's Otsu split of the single stain; the value stays editable.
 */
public final class PositivityDialog {

    private PositivityDialog() {}

    private static final ObjectMapper JSON = new ObjectMapper();
    static final String ROLE_FMO = "FMO";

    private static final Color C_UNSTAINED = Color.web("#9AA5B1");
    private static final Color C_FMO       = Color.web("#F2A94B");

    /** Mutable row for the role table. */
    public static final class Row {
        private final String sample;
        private String role;
        Row(String sample, String role) { this.sample = sample; this.role = role; }
        public String getSample() { return sample; }
        public String getRole() { return role; }
        public void setRole(String r) { this.role = r; }
    }

    /**
     * Opens modally. Returns the chosen threshold, or empty on cancel.
     *
     * @param loader fetches a sample's events (cached or from the engine) and calls back on the FX thread
     */
    public static Optional<Double> show(Window owner, AppContext ctx, String channel, Double initial,
                                        java.util.function.BiConsumer<String, Consumer<EventData>> loader) {
        List<String> samples = new ArrayList<>(ctx.workspace().sampleNames());
        if (samples.isEmpty()) return Optional.empty();

        ObservableList<Row> rows = FXCollections.observableArrayList();
        for (String s : samples) rows.add(new Row(s, guessRole(s)));

        TableView<Row> table = new TableView<>(rows);
        table.setEditable(true);
        table.setPrefSize(300, 240);
        TableColumn<Row, String> sampleCol = new TableColumn<>("Sample");
        sampleCol.setCellValueFactory(new javafx.scene.control.cell.PropertyValueFactory<>("sample"));
        sampleCol.setPrefWidth(180);
        TableColumn<Row, String> roleCol = new TableColumn<>("Role");
        roleCol.setCellValueFactory(new javafx.scene.control.cell.PropertyValueFactory<>("role"));
        roleCol.setCellFactory(ComboBoxTableCell.forTableColumn(
                CompWizardController.ROLE_SINGLE, CompWizardController.ROLE_UNSTAINED,
                ROLE_FMO, CompWizardController.ROLE_IGNORE));
        roleCol.setPrefWidth(110);
        table.getColumns().add(sampleCol);
        table.getColumns().add(roleCol);

        CytoPlot plot = new CytoPlot();
        plot.setChannelLabeler(c -> ctx.aliases().label(c));
        plot.setPlotType("histogram");
        // Logicle, not linear: a viability dye's signal is otherwise squashed into the far-left few
        // pixels of a 0..262143 axis and cannot be gated by eye. Logicle spreads the negative and
        // low-positive region the way FlowJo does.
        plot.setXScale(CytoPlot.Scale.LOGICLE);
        plot.setInterceptVisible(true);
        plot.setTool("None");
        StackPane plotHost = new StackPane(plot);
        plotHost.setPrefSize(430, 300);
        plot.prefWidthProperty().bind(plotHost.widthProperty());
        plot.prefHeightProperty().bind(plotHost.heightProperty());

        TextField thrField = new TextField(initial == null ? "" : String.format("%.1f", initial));
        thrField.setPrefWidth(110);
        ComboBox<String> methodCombo = new ComboBox<>(FXCollections.observableArrayList(
                "otsu", "yen", "triangle", "li", "isodata"));
        methodCombo.getSelectionModel().select("otsu");
        methodCombo.setPrefWidth(100);
        Tooltip.install(methodCombo, new Tooltip(
                "otsu — two classes of similar spread (default)\n"
                + "yen — better when the positive peak is small\n"
                + "triangle — one dominant negative peak with a long positive tail\n"
                + "li — heavily overlapping peaks\n"
                + "isodata — iterative intermeans, less sensitive to peak shape"));
        Button autoBtn = new Button("Auto");
        Button applyBtn = new Button("Apply to line");
        ToggleButton drawBtn = new ToggleButton("Draw threshold");
        Tooltip.install(drawBtn, new Tooltip(
                "Drag an interval across the positive events. Its LEFT edge becomes the threshold — "
                + "a polygon would add nothing, because positivity on one marker is a single cut."));
        Label status = new Label();
        status.setWrapText(true);
        status.getStyleClass().add("subtitle");

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Positivity threshold — " + ctx.aliases().label(channel));
        dlg.setHeaderText(null);
        dlg.setResizable(true);

        Runnable[] rebuild = new Runnable[1];
        double[] threshold = {initial == null ? Double.NaN : initial};

        plot.setOnInterceptChanged((x, y) -> {
            threshold[0] = x;
            thrField.setText(String.format("%.1f", x));
        });

        applyBtn.setOnAction(e -> {
            try {
                double v = Double.parseDouble(thrField.getText().trim());
                threshold[0] = v;
                plot.setInterceptX(v);
            } catch (NumberFormatException ex) {
                status.setText("Threshold must be a number.");
            }
        });

        // Drawing an interval across the positive events sets the threshold from its left edge. Give
        // real feedback: a crosshair cursor and an explicit instruction, because on a viability dye the
        // signal is squashed to the far left and the line-move is otherwise easy to miss.
        drawBtn.selectedProperty().addListener((o, a, on) -> {
            plot.setTool(on ? "Interval" : "None");
            plotHost.setCursor(on ? javafx.scene.Cursor.CROSSHAIR : javafx.scene.Cursor.DEFAULT);
            if (on) status.setText("Draw mode: drag left-to-right across the plot. Where you START the drag "
                    + "becomes the threshold. Release to set it.");
        });
        plot.setOnGateDrawn(g -> {
            drawBtn.setSelected(false);
            plotHost.setCursor(javafx.scene.Cursor.DEFAULT);
            if (g.xs == null || g.xs.length == 0) return;
            double lo = g.xs[0];
            for (double x : g.xs) lo = Math.min(lo, x);
            threshold[0] = lo;
            thrField.setText(String.format("%.1f", lo));
            plot.setInterceptX(lo);
            status.setText(String.format("Threshold set to %.1f (the gate's left edge). Edit it or drag the dashed line to fine-tune.", lo));
        });

        autoBtn.setOnAction(e -> {
            String single = firstWithRole(rows, CompWizardController.ROLE_SINGLE);
            if (single == null) { status.setText("Assign a single-stain control first."); return; }
            String method = methodCombo.getValue();
            ObjectNode args = JSON.createObjectNode();
            args.put("sample", single);
            args.put("method", method);
            args.putArray("channels").add(channel);
            status.setText("Computing " + method + " split from " + single + "…");
            ctx.jobs().run(ctx.bridge().command("positivity_thresholds", args), r -> {
                JsonNode c = r.path("channels").path(0);
                if (!c.path("ok").asBoolean(false)) {
                    status.setText("Auto failed: " + c.path("error").asText("?"));
                    return;
                }
                threshold[0] = c.path("threshold").asDouble();
                thrField.setText(String.format("%.1f", threshold[0]));
                plot.setInterceptX(threshold[0]);
                // Report the method that RAN, not the one requested — they differ if skimage is absent.
                String actual = c.path("method").asText(method);
                status.setText(String.format("%s split at %.1f — %.1f%% of %s is positive.%s Drag the line or draw a gate to adjust.",
                        actual, threshold[0], c.path("pct_positive").asDouble(), single,
                        actual.equals(method) ? "" : " (" + method + " unavailable)"));
            });
        });

        // Rebuild the overlay whenever a role changes: the picture must always match the table.
        rebuild[0] = () -> {
            String single = firstWithRole(rows, CompWizardController.ROLE_SINGLE);
            String unstained = firstWithRole(rows, CompWizardController.ROLE_UNSTAINED);
            String fmo = firstWithRole(rows, ROLE_FMO);

            if (single == null || unstained == null) {
                plot.setData(null);
                status.setText("Assign BOTH an unstained control and a single stain. "
                        + "Without them there is no control-derived threshold — and an Otsu split of "
                        + "stained data is not the same thing.");
                return;
            }
            status.setText(fmo == null
                    ? "No FMO assigned. Using unstained + single stain only."
                    : "Using unstained, single stain and FMO (" + fmo + ").");

            loader.accept(single, main -> {
                if (main == null || main.indexOf(channel) < 0) {
                    status.setText("Channel " + channel + " is not present in " + single + ".");
                    return;
                }
                plot.setData(main);
                plot.setAxes(channel, null);
                if (!Double.isNaN(threshold[0])) plot.setInterceptX(threshold[0]);
                else plot.centerIntercept();

                List<CytoPlot.HistOverlay> ov = new ArrayList<>();
                loader.accept(unstained, u -> {
                    if (u != null) ov.add(new CytoPlot.HistOverlay("Unstained", u, C_UNSTAINED));
                    if (fmo == null) { plot.setHistogramOverlays(ov); return; }
                    loader.accept(fmo, f -> {
                        if (f != null) ov.add(new CytoPlot.HistOverlay("FMO", f, C_FMO));
                        plot.setHistogramOverlays(ov);
                    });
                });
            });
        };
        // ONE handler: write the role, then rebuild. Splitting these across setOnEditCommit and
        // addEventHandler leaves their order unspecified, and when the rebuild ran first it read the
        // old role, found no single stain, and blanked the plot with no further event to recover from.
        roleCol.setOnEditCommit(e -> {
            e.getRowValue().setRole(e.getNewValue());
            table.refresh();
            rebuild[0].run();
        });
        rebuild[0].run();

        // Typing a value and pressing Enter moves the line — one less button to hunt for.
        thrField.setOnAction(e -> applyBtn.fire());

        FlowPane controls = new FlowPane(8, 6,
                new Label("Threshold:"), thrField, applyBtn, drawBtn,
                new Label("Method:"), methodCombo, autoBtn);
        controls.setAlignment(Pos.CENTER_LEFT);

        // Spell out what each control does — the buttons are not self-evident.
        Label legend = new Label(
                "Threshold: the positive/negative cut (the dashed line).   "
                + "Apply to line: move the line to the typed value (or press Enter).   "
                + "Draw threshold: drag on the plot to set it by eye.   "
                + "Method + Auto: compute the split from the single-stain control.   "
                + "You can also just drag the dashed line directly.");
        legend.setWrapText(true);
        legend.getStyleClass().add("subtitle");

        VBox right = new VBox(8,
                new Label("Single stain (filled) with its controls overlaid. "
                        + "Each trace is normalised to its own peak, so the heights are shapes, not counts."),
                plotHost, controls, legend, status);
        right.getChildren().get(0).getStyleClass().add("subtitle");
        ((Label) right.getChildren().get(0)).setWrapText(true);
        VBox.setVgrow(plotHost, Priority.ALWAYS);

        // Fix the controls column at a width that fits both columns, and let the table fill the height,
        // so it can never collapse into the crushed top-left box it did before. Without a min width the
        // HBox gave everything to the (hgrow) plot side and squeezed the table to a few pixels.
        VBox left = new VBox(6, new Label("Controls"), table);
        left.getChildren().get(0).getStyleClass().add("subtitle");
        left.setMinWidth(320);
        left.setPrefWidth(320);
        left.setMaxWidth(320);
        table.setMaxHeight(Double.MAX_VALUE);
        table.setColumnResizePolicy(TableView.CONSTRAINED_RESIZE_POLICY);
        VBox.setVgrow(table, Priority.ALWAYS);

        HBox root = new HBox(12, left, right);
        root.setPadding(new Insets(4));
        HBox.setHgrow(right, Priority.ALWAYS);

        dlg.getDialogPane().setContent(root);
        // Open at a usable size so the table and plot are both visible immediately.
        dlg.getDialogPane().setPrefSize(940, 560);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, owner);

        // OK must not hand back a threshold that was never derived from controls.
        javafx.scene.Node ok = dlg.getDialogPane().lookupButton(ButtonType.OK);
        ok.disableProperty().bind(thrField.textProperty().isEmpty());

        Optional<ButtonType> res = dlg.showAndWait();
        if (res.isEmpty() || res.get() != ButtonType.OK) return Optional.empty();
        if (firstWithRole(rows, CompWizardController.ROLE_SINGLE) == null
                || firstWithRole(rows, CompWizardController.ROLE_UNSTAINED) == null) return Optional.empty();
        try {
            double v = Double.parseDouble(thrField.getText().trim());
            ctx.fmo().set(channel, v, firstWithRole(rows, CompWizardController.ROLE_SINGLE));
            return Optional.of(v);
        } catch (NumberFormatException e) {
            return Optional.empty();
        }
    }

    private static String firstWithRole(List<Row> rows, String role) {
        for (Row r : rows) if (role.equals(r.getRole())) return r.getSample();
        return null;
    }

    /** Same filename heuristic the compensation wizard uses; the user can always override it. */
    private static String guessRole(String name) {
        String n = name.toLowerCase();
        if (n.contains("unstain") || n.contains("blank") || n.contains("negative")) return CompWizardController.ROLE_UNSTAINED;
        if (n.contains("fmo")) return ROLE_FMO;
        return CompWizardController.ROLE_IGNORE;
    }

    /** Cached events, or a fetch from the engine. Reusable by any dialog that needs a sample's events. */
    public static void loadEvents(AppContext ctx, String sample, Consumer<EventData> onReady) {
        EventData cached = ctx.workspace().data(sample);
        if (cached != null && cached.rows() > 0) { onReady.accept(cached); return; }
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
                onReady.accept(d);
            } catch (Exception e) {
                onReady.accept(null);
            }
        });
    }
}
