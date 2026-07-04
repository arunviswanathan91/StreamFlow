package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.beans.property.ReadOnlyStringWrapper;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;
import javafx.collections.transformation.FilteredList;
import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.Label;
import javafx.scene.control.TableColumn;
import javafx.scene.control.TableView;
import javafx.scene.control.TextField;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;

import java.util.ArrayList;
import java.util.List;

/**
 * Per-FCS diagnostics window, opened by clicking a sample's QC knob in the Workstation. Pulls the
 * file's full TEXT segment via {@code get_metadata} and presents it in three dark-themed tabs:
 * a curated Summary (acquisition + QC), a Parameters/Channels table (PnN/PnS/PnR/PnB/PnE derived
 * from the keywords), and the complete, filterable keyword list — so a user can see exactly what
 * the FCS parser read and why a file passed or failed QC.
 */
public class FcsDiagnosticsController {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private Label titleLabel, qcLabel, statusLabel;
    @FXML private TableView<String[]> summaryTable, paramTable, kwTable;
    @FXML private TableColumn<String[], String> sumKeyCol, sumValCol;
    @FXML private TableColumn<String[], String> pIdxCol, pNameCol, pMarkerCol, pRangeCol, pBitsCol, pScaleCol;
    @FXML private TableColumn<String[], String> kwKeyCol, kwValCol;
    @FXML private TextField searchField;
    @FXML private Button closeButton;

    private final ObservableList<String[]> summaryRows = FXCollections.observableArrayList();
    private final ObservableList<String[]> paramRows = FXCollections.observableArrayList();
    private final ObservableList<String[]> kwRows = FXCollections.observableArrayList();

    private AppContext ctx;
    private String sample;
    private String qcText;
    private Stage stage;

    /** Open the diagnostics window for a sample; {@code qcText} is the Workstation's QC verdict line. */
    public static void open(AppContext ctx, String sample, String qcText) {
        try {
            FXMLLoader loader = new FXMLLoader(
                    FcsDiagnosticsController.class.getResource("/org/streamflow/ui/fcs-diagnostics.fxml"));
            VBox root = loader.load();
            FcsDiagnosticsController c = loader.getController();
            c.ctx = ctx; c.sample = sample; c.qcText = qcText;
            Scene scene = new Scene(root);
            scene.getStylesheets().add(FcsDiagnosticsController.class
                    .getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
            Stage stage = new Stage();
            stage.setTitle("StreamFLOW — FCS Diagnostics · " + sample);
            stage.setScene(scene);
            AppIcons.apply(stage);
            c.stage = stage;
            c.load();
            stage.show();
        } catch (Exception e) {
            throw new RuntimeException("Could not open FCS diagnostics: " + e.getMessage(), e);
        }
    }

    @FXML
    public void initialize() {
        sumKeyCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[0]));
        sumValCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[1]));
        summaryTable.setItems(summaryRows);

        pIdxCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[0]));
        pNameCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[1]));
        pMarkerCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[2]));
        pRangeCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[3]));
        pBitsCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[4]));
        pScaleCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[5]));
        paramTable.setItems(paramRows);

        kwKeyCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[0]));
        kwValCol.setCellValueFactory(c -> new ReadOnlyStringWrapper(c.getValue()[1]));
        FilteredList<String[]> filtered = new FilteredList<>(kwRows, x -> true);
        kwTable.setItems(filtered);
        searchField.textProperty().addListener((o, a, b) -> {
            String q = b == null ? "" : b.toLowerCase().trim();
            filtered.setPredicate(row -> q.isEmpty()
                    || row[0].toLowerCase().contains(q) || row[1].toLowerCase().contains(q));
        });
    }

    private void load() {
        titleLabel.setText("FCS Diagnostics · " + sample);
        qcLabel.setText(qcText == null ? "" : qcText);
        if (ctx == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        statusLabel.setText("Reading FCS header…");
        ctx.jobs().run(ctx.bridge().command("get_metadata", args), this::showMetadata);
    }

    private void showMetadata(JsonNode result) {
        JsonNode kw = result.path("keywords");

        // FCS readers vary on keyword casing / leading '$' (flowio lowercases and strips '$'),
        // so normalise every key to lower-case, '$'-stripped for robust lookups.
        java.util.Map<String, String> map = new java.util.HashMap<>();
        List<String[]> all = new ArrayList<>();
        kw.fields().forEachRemaining(e -> {
            all.add(new String[]{e.getKey(), e.getValue().asText()});
            map.put(norm(e.getKey()), e.getValue().asText());
        });
        all.sort((a, b) -> {
            boolean ad = a[0].startsWith("$"), bd = b[0].startsWith("$");
            if (ad != bd) return ad ? -1 : 1;
            return a[0].compareToIgnoreCase(b[0]);
        });
        kwRows.setAll(all);

        // ---- summary (curated acquisition keywords + QC) ----
        summaryRows.clear();
        summaryRows.add(new String[]{"Sample", result.path("sample").asText(sample)});
        if (qcText != null && !qcText.isBlank()) summaryRows.add(new String[]{"QC", qcText});
        addIfPresent(map, "Total events ($TOT)", "tot");
        addIfPresent(map, "Parameters ($PAR)", "par");
        addIfPresent(map, "Cytometer ($CYT)", "cyt");
        addIfPresent(map, "System ($SYS)", "sys");
        addIfPresent(map, "Acquisition date ($DATE)", "date");
        addIfPresent(map, "Begin time ($BTIM)", "btim");
        addIfPresent(map, "End time ($ETIM)", "etim");
        addIfPresent(map, "Operator ($OP)", "op");
        addIfPresent(map, "Source ($SRC)", "src");
        addIfPresent(map, "Experiment ($EXP)", "exp");
        addIfPresent(map, "FCS version ($FCSversion)", "fcsversion");
        addIfPresent(map, "Byte order ($BYTEORD)", "byteord");
        addIfPresent(map, "Data type ($DATATYPE)", "datatype");
        summaryRows.add(new String[]{"Keyword count", String.valueOf(result.path("count").asInt())});

        // ---- parameters table from PnN/PnS/PnR/PnB/PnE ----
        paramRows.clear();
        int par = parseIntSafe(map.getOrDefault("par", ""), 0);
        for (int j = 1; j <= par; j++) {
            String n = map.getOrDefault("p" + j + "n", "");
            String s = map.getOrDefault("p" + j + "s", "");
            String r = map.getOrDefault("p" + j + "r", "");
            String b = map.getOrDefault("p" + j + "b", "");
            String e = map.getOrDefault("p" + j + "e", "");
            paramRows.add(new String[]{String.valueOf(j), n.isEmpty() ? "P" + j : n, s, r, b, e});
        }

        statusLabel.setText(result.path("count").asInt() + " keyword(s), " + par + " parameter(s).");
    }

    /** lower-case + strip a single leading '$' so "$PAR", "PAR", "par" all match. */
    private static String norm(String key) {
        String k = key == null ? "" : key.trim().toLowerCase();
        return k.startsWith("$") ? k.substring(1) : k;
    }

    private void addIfPresent(java.util.Map<String, String> map, String label, String normKey) {
        String v = map.get(normKey);
        if (v != null && !v.isEmpty()) summaryRows.add(new String[]{label, v});
    }

    private static int parseIntSafe(String s, int dflt) {
        try { return Integer.parseInt(s.trim()); } catch (Exception e) { return dflt; }
    }

    @FXML private void onClose() { if (stage != null) stage.close(); }
}
