package org.streamflow.ui;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.collections.FXCollections;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.control.ListView;
import javafx.scene.control.SelectionMode;
import javafx.scene.control.Spinner;

/**
 * Phase 2 — Transformation. Apply a logicle / arcsinh / log transform to the
 * (compensated) fluorescence channels. Channels default to all fluors; the user
 * can restrict the selection.
 */
public class TransformationController implements ContextAware, Refreshable {

    private static final ObjectMapper JSON = new ObjectMapper();

    @FXML private ComboBox<String> methodCombo;
    @FXML private Spinner<Integer> cofactorSpinner;
    @FXML private ListView<String> channelList;
    @FXML private Button refreshButton;
    @FXML private Button applyButton;
    @FXML private Button aboutButton;
    @FXML private Label statusLabel;

    private AppContext ctx;

    @FXML
    public void initialize() {
        methodCombo.setItems(FXCollections.observableArrayList("logicle", "arcsinh", "log"));
        methodCombo.getSelectionModel().select("logicle");
        cofactorSpinner.setValueFactory(
                new javafx.scene.control.SpinnerValueFactory.IntegerSpinnerValueFactory(1, 10000, 150, 10));
        cofactorSpinner.disableProperty().bind(
                methodCombo.getSelectionModel().selectedItemProperty().isNotEqualTo("arcsinh"));
        channelList.getSelectionModel().setSelectionMode(SelectionMode.MULTIPLE);
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refreshChannels();
    }

    @FXML
    private void onRefresh() {
        refreshChannels();
    }

    @FXML
    private void onAbout() {
        String msg =
            "WHAT TRANSFORMATION DOES\n" +
            "Flow data is roughly log-normal and spans many decades with values near (and below) zero. " +
            "A transform reshapes those values so populations separate cleanly instead of bunching against the axis.\n\n" +
            "METHODS\n" +
            "• Logicle (biexponential) — log-like for bright signal, linear near zero; handles negatives from " +
            "compensation. Best default for fluorescence.\n" +
            "• Arcsinh — similar shape; the cofactor sets where it switches from linear to log (CyTOF: ~5; " +
            "fluorescence: ~150).\n" +
            "• Log — classic log10; only valid for strictly positive data.\n\n" +
            "TRANSFORM vs AXIS SCALE\n" +
            "• Axis scale (in a Graph Window) changes only how that plot is DRAWN — nothing to the data. " +
            "For gating and viewing, just pick the axis scale; that's all you need.\n" +
            "• This module transforms the STORED events that engine analyses consume — Clustering, " +
            "Dimensionality Reduction and cross-sample statistics run on the transformed values. " +
            "Apply it before those analyses if you want them computed on transformed data.\n\n" +
            "Tip: the channel list auto-suggests Logicle when negatives are present (post-compensation), " +
            "otherwise arcsinh.";
        javafx.scene.control.Alert a = new javafx.scene.control.Alert(javafx.scene.control.Alert.AlertType.INFORMATION);
        a.setTitle("About Transformation");
        a.setHeaderText("Logicle / Arcsinh / Log — and how it differs from axis scale");
        javafx.scene.control.TextArea ta = new javafx.scene.control.TextArea(msg);
        ta.setEditable(false); ta.setWrapText(true); ta.setPrefSize(560, 380);
        a.getDialogPane().setContent(ta);
        a.showAndWait();
    }

    @Override public void refreshFromWorkspace() { refreshChannels(); }

    private void refreshChannels() {
        if (ctx == null) return;
        ctx.jobs().run(ctx.bridge().command("list_fluor_channels", null), result -> {
            channelList.getItems().clear();
            result.path("channels").forEach(n -> channelList.getItems().add(n.asText()));
            int n = channelList.getItems().size();
            statusLabel.setText(n + " fluorescence channel(s) — analysing…");
            ctx.jobs().run(ctx.bridge().command("suggest_transform", null), this::applySuggestion);
        });
    }

    private void applySuggestion(com.fasterxml.jackson.databind.JsonNode result) {
        int logicleCount = result.path("summary").path("logicle_count").asInt(0);
        int total = channelList.getItems().size();
        if (logicleCount > 0) {
            methodCombo.getSelectionModel().select("logicle");
            statusLabel.setText(String.format(
                    "%d channel(s) — Tip: Logicle recommended (%d/%d channels have negative values)",
                    total, logicleCount, total));
        } else {
            methodCombo.getSelectionModel().select("arcsinh");
            statusLabel.setText(total + " channel(s) — arcsinh/150 recommended (no negative values).");
        }
    }

    @FXML
    private void onApply() {
        if (ctx == null) return;
        ObjectNode args = JSON.createObjectNode();
        args.put("method", methodCombo.getValue());
        if ("arcsinh".equals(methodCombo.getValue())) {
            args.put("cofactor", cofactorSpinner.getValue());
        }
        var selected = channelList.getSelectionModel().getSelectedItems();
        if (!selected.isEmpty()) {
            var arr = args.putArray("channels");
            selected.forEach(arr::add);
        }
        statusLabel.setText("Applying " + methodCombo.getValue() + " transform…");
        ctx.jobs().run(ctx.bridge().command("apply_transformation", args), r ->
                statusLabel.setText("Transformed " + r.path("channels").size()
                        + " channel(s) with " + r.path("method").asText() + "."));
    }

    private void setDisabled(boolean d) {
        methodCombo.setDisable(d);
        channelList.setDisable(d);
        refreshButton.setDisable(d);
        applyButton.setDisable(d);
    }
}
