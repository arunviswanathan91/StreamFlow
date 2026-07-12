package org.streamflow.ui;

import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.geometry.Insets;
import javafx.scene.control.Button;
import javafx.scene.control.ButtonType;
import javafx.scene.control.CheckBox;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Dialog;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.TextField;
import javafx.scene.layout.HBox;
import javafx.scene.layout.Priority;
import javafx.scene.layout.VBox;
import javafx.stage.Window;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * FlowJo's "Add Statistic" window: pick a Statistic, the Population it applies to, and the Parameter
 * it is computed on. Statistics attach to a population <i>by name</i>, so they follow that population
 * onto every sample — which is what "apply to all samples" means here; there is nothing per-sample to
 * copy. The extra checkbox makes the chosen set the default for populations that have no set of their
 * own.
 *
 * The dialog edits a working copy and only returns on OK, so Cancel truly cancels.
 */
public final class AddStatisticDialog {

    private AddStatisticDialog() {}

    /** Display name -> key prefix, in FlowJo's order. */
    private static final Map<String, String> STATS = new LinkedHashMap<>();
    static {
        STATS.put("Median", StatKeys.MEDIAN);
        STATS.put("Mean", StatKeys.MEAN);
        STATS.put("Geometric Mean", StatKeys.GEOMEAN);
        STATS.put("Mode", StatKeys.MODE);
        STATS.put("Robust CV", StatKeys.RCV);
        STATS.put("Robust SD", StatKeys.RSD);
        STATS.put("CV", StatKeys.CV);
        STATS.put("SD", StatKeys.SD);
        STATS.put("Median Abs Dev", StatKeys.MAD);
        STATS.put("Percentile", StatKeys.PCT);
        STATS.put("Correlation", StatKeys.CORRELATION);
        STATS.put("Min", StatKeys.MIN);
        STATS.put("Max", StatKeys.MAX);
        STATS.put("Count", StatKeys.COUNT);
        STATS.put("Freq. of Parent", StatKeys.PARENT);
        STATS.put("Freq. of Grandparent", StatKeys.GRANDPARENT);
        STATS.put("Freq. of Total", StatKeys.TOTAL);
        STATS.put("Freq. of", StatKeys.FREQ);
    }

    /** Keys per population name, and whether they also become the default for unconfigured gates. */
    public record Result(Map<String, List<String>> byPopulation, boolean setAsDefault) {}

    public static Optional<Result> show(Window owner, List<String> populations, String focusPop,
                                        List<String> channels, ChannelAliases aliases,
                                        Map<String, List<String>> current, boolean defaultOn) {
        if (populations.isEmpty()) return Optional.empty();

        Map<String, ObservableList<String>> working = new LinkedHashMap<>();
        for (String p : populations)
            working.put(p, FXCollections.observableArrayList(current.getOrDefault(p, List.of())));

        ListView<String> statList = new ListView<>(FXCollections.observableArrayList(STATS.keySet()));
        ListView<String> popList = new ListView<>(FXCollections.observableArrayList(populations));
        ListView<String> paramList = new ListView<>(FXCollections.observableArrayList(channels));
        statList.setPrefSize(170, 240);
        popList.setPrefSize(170, 240);
        paramList.setPrefSize(190, 240);

        // Show aliases (CD4) rather than raw detectors (BV421-A) — the same names used everywhere else.
        paramList.setCellFactory(lv -> new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(String ch, boolean empty) {
                super.updateItem(ch, empty);
                setText(empty || ch == null ? null : aliases.label(ch));
            }
        });

        TextField pctField = new TextField("50");
        pctField.setPrefWidth(70);
        ComboBox<String> vsCombo = new ComboBox<>(FXCollections.observableArrayList(channels));
        vsCombo.setPrefWidth(170);
        ComboBox<String> freqCombo = new ComboBox<>(FXCollections.observableArrayList(populations));
        freqCombo.setPrefWidth(170);

        ListView<String> shownList = new ListView<>();
        shownList.setPrefSize(300, 240);
        shownList.setCellFactory(lv -> new javafx.scene.control.ListCell<>() {
            @Override protected void updateItem(String key, boolean empty) {
                super.updateItem(key, empty);
                setText(empty || key == null ? null : StatKeys.describe(key, aliases));
            }
        });

        Button add = new Button("Add →");
        Button remove = new Button("← Remove");
        CheckBox asDefault = new CheckBox("Also use these statistics for populations with none of their own");
        asDefault.setSelected(defaultOn);
        Label hint = new Label();
        hint.setWrapText(true);
        hint.getStyleClass().add("subtitle");

        popList.getSelectionModel().selectedItemProperty().addListener((o, a, pop) -> {
            shownList.setItems(pop == null ? FXCollections.observableArrayList() : working.get(pop));
        });
        popList.getSelectionModel().select(populations.contains(focusPop) ? focusPop : populations.get(0));

        Runnable syncEnabled = () -> {
            String stat = STATS.get(statList.getSelectionModel().getSelectedItem());
            boolean chan = stat != null && StatKeys.needsData(stat);
            paramList.setDisable(!chan);
            pctField.setDisable(!StatKeys.PCT.equals(stat));
            vsCombo.setDisable(!StatKeys.CORRELATION.equals(stat));
            freqCombo.setDisable(!StatKeys.FREQ.equals(stat));
            hint.setText(hintFor(stat));
        };
        statList.getSelectionModel().selectedItemProperty().addListener((o, a, b) -> syncEnabled.run());
        statList.getSelectionModel().selectFirst();
        paramList.getSelectionModel().selectFirst();
        vsCombo.getSelectionModel().selectFirst();
        freqCombo.getSelectionModel().selectFirst();
        syncEnabled.run();

        add.setOnAction(e -> {
            String pop = popList.getSelectionModel().getSelectedItem();
            String stat = STATS.get(statList.getSelectionModel().getSelectedItem());
            if (pop == null || stat == null) return;
            String key = buildKey(stat, paramList.getSelectionModel().getSelectedItem(),
                    vsCombo.getValue(), freqCombo.getValue(), pctField.getText());
            if (key == null) { hint.setText("Select a parameter first."); return; }
            ObservableList<String> keys = working.get(pop);
            if (!keys.contains(key)) keys.add(key);
        });
        remove.setOnAction(e -> {
            String key = shownList.getSelectionModel().getSelectedItem();
            if (key != null) shownList.getItems().remove(key);
        });

        VBox mid = new VBox(6, add, remove);
        mid.setPadding(new Insets(90, 0, 0, 0));

        HBox lists = new HBox(8,
                labelled("Statistic", statList),
                labelled("Population", popList),
                labelled("Parameter", paramList),
                mid,
                labelled("Shown on the gate label", shownList));
        HBox.setHgrow(lists, Priority.ALWAYS);

        HBox args = new HBox(8,
                new Label("Percentile:"), pctField,
                new Label("Correlate with:"), vsCombo,
                new Label("Freq. of:"), freqCombo);
        args.setAlignment(javafx.geometry.Pos.CENTER_LEFT);

        VBox root = new VBox(10, lists, args, asDefault, hint);
        root.setPadding(new Insets(4));

        Dialog<ButtonType> dlg = new Dialog<>();
        dlg.setTitle("Add Statistic");
        dlg.setHeaderText(null);
        dlg.setResizable(true);
        dlg.getDialogPane().setContent(root);
        dlg.getDialogPane().getButtonTypes().addAll(ButtonType.OK, ButtonType.CANCEL);
        dlg.setResultConverter(b -> b);
        AppIcons.theme(dlg, owner);

        Optional<ButtonType> res = dlg.showAndWait();
        if (res.isEmpty() || res.get() != ButtonType.OK) return Optional.empty();

        Map<String, List<String>> out = new LinkedHashMap<>();
        working.forEach((pop, keys) -> out.put(pop, new ArrayList<>(keys)));
        return Optional.of(new Result(out, asDefault.isSelected()));
    }

    private static VBox labelled(String title, javafx.scene.Node n) {
        Label l = new Label(title);
        l.getStyleClass().add("subtitle");
        VBox v = new VBox(4, l, n);
        VBox.setVgrow(n, Priority.ALWAYS);
        return v;
    }

    private static String buildKey(String stat, String param, String vs, String freqPop, String pctText) {
        switch (stat) {
            case StatKeys.COUNT, StatKeys.PARENT, StatKeys.GRANDPARENT, StatKeys.TOTAL:
                return stat;
            case StatKeys.FREQ:
                return freqPop == null ? null : StatKeys.FREQ + ":" + freqPop;
            case StatKeys.PCT: {
                if (param == null) return null;
                double p;
                try { p = Double.parseDouble(pctText.trim()); } catch (Exception e) { return null; }
                if (p < 0 || p > 100) return null;
                return StatKeys.PCT + ":" + StatKeys.trimPct(p) + ":" + param;
            }
            case StatKeys.CORRELATION:
                return (param == null || vs == null) ? null : StatKeys.CORRELATION + ":" + param + ":" + vs;
            default:
                return param == null ? null : stat + ":" + param;
        }
    }

    private static String hintFor(String stat) {
        if (stat == null) return "";
        return switch (stat) {
            case StatKeys.GEOMEAN -> "Geometric mean excludes events at or below zero (compensation produces "
                    + "negatives). The label reports how many were excluded.";
            case StatKeys.RSD -> "Robust SD = (P84.13 − P15.87) / 2 — insensitive to outliers.";
            case StatKeys.RCV -> "Robust CV = 100 × Robust SD / Median.";
            case StatKeys.CV -> "CV = 100 × SD / Mean, using the population SD.";
            case StatKeys.MAD -> "Median absolute deviation: median(|x − median|), unscaled.";
            case StatKeys.MODE -> "Peak of the smoothed distribution, not the most frequent raw value.";
            case StatKeys.PCT -> "Linear interpolation between order statistics (R type 7).";
            case StatKeys.FREQ -> "This population's events as a percentage of any other population.";
            default -> "";
        };
    }
}
