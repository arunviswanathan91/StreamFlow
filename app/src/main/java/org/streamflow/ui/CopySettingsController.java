package org.streamflow.ui;

import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.Label;
import javafx.scene.control.Slider;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;

/**
 * The app-wide "Copy / Export Format" window (Graph Window ▸ gear button). Adjusting any control
 * updates {@link AppSettings}, which broadcasts to every open plot so the change is live and every
 * subsequent Copy — in any graph window — uses the same format until changed here again. A single
 * shared window is reused (brought to front if already open).
 */
public class CopySettingsController {

    @FXML private Slider pointSlider, axisSlider, labelSlider, chartTitleSlider, chartLegendSlider, dpiSlider;
    @FXML private Label pointVal, axisVal, labelVal, chartTitleVal, chartLegendVal, dpiVal;
    @FXML private CheckBox gateLabelsCheck;
    @FXML private Button closeButton;

    private AppSettings settings;
    private Stage stage;

    private static Stage openStage;   // reuse one window

    /** Open (or focus) the shared Copy-Settings window bound to the given settings. */
    public static void open(AppSettings settings) {
        if (openStage != null && openStage.isShowing()) { openStage.toFront(); openStage.requestFocus(); return; }
        try {
            FXMLLoader loader = new FXMLLoader(
                    CopySettingsController.class.getResource("/org/streamflow/ui/copy-settings.fxml"));
            VBox root = loader.load();
            CopySettingsController c = loader.getController();
            c.settings = settings;
            Scene scene = new Scene(root);
            scene.getStylesheets().add(CopySettingsController.class
                    .getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
            Stage stage = new Stage();
            stage.setTitle("StreamFLOW — Copy / Export Format");
            stage.setScene(scene);
            AppIcons.apply(stage);
            c.stage = stage;
            openStage = stage;
            stage.setOnHidden(e -> { if (openStage == stage) openStage = null; });
            c.bind();
            stage.show();
        } catch (Exception e) {
            throw new RuntimeException("Could not open copy-settings window: " + e.getMessage(), e);
        }
    }

    /** Seed controls from the current settings and wire each to write straight back (live). */
    private void bind() {
        pointSlider.setValue(settings.exportPointSize());
        pointVal.setText(label(settings.exportPointSize()));
        pointSlider.valueProperty().addListener((o, a, b) -> {
            int v = (int) Math.round(b.doubleValue());
            pointVal.setText(label(v));
            settings.setExportPointSize(v);
        });

        axisSlider.setValue(settings.exportAxisFontSize());
        axisVal.setText(fmt(settings.exportAxisFontSize()));
        axisSlider.valueProperty().addListener((o, a, b) -> {
            axisVal.setText(fmt(b.doubleValue()));
            settings.setExportAxisFontSize(b.doubleValue());
        });

        labelSlider.setValue(settings.exportFontSize());
        labelVal.setText(fmt(settings.exportFontSize()));
        labelSlider.valueProperty().addListener((o, a, b) -> {
            labelVal.setText(fmt(b.doubleValue()));
            settings.setExportFontSize(b.doubleValue());
        });

        chartTitleSlider.setValue(settings.chartTitleFontSize());
        chartTitleVal.setText(fmt(settings.chartTitleFontSize()));
        chartTitleSlider.valueProperty().addListener((o, a, b) -> {
            chartTitleVal.setText(fmt(b.doubleValue()));
            settings.setChartTitleFontSize(b.doubleValue());
        });

        chartLegendSlider.setValue(settings.chartLegendFontSize());
        chartLegendVal.setText(fmt(settings.chartLegendFontSize()));
        chartLegendSlider.valueProperty().addListener((o, a, b) -> {
            chartLegendVal.setText(fmt(b.doubleValue()));
            settings.setChartLegendFontSize(b.doubleValue());
        });

        dpiSlider.setValue(settings.exportDpi());
        dpiVal.setText(settings.exportDpi() + " dpi");
        dpiSlider.valueProperty().addListener((o, a, b) -> {
            int v = (int) Math.round(b.doubleValue());
            dpiVal.setText(v + " dpi");
            settings.setExportDpi(v);
        });

        gateLabelsCheck.setSelected(settings.exportGateLabels());
        gateLabelsCheck.selectedProperty().addListener((o, a, b) -> settings.setExportGateLabels(b));
    }

    private static String label(int pointRadius) { return pointRadius == 0 ? "1 px" : (2 * pointRadius + 1) + " px"; }
    private static String fmt(double v) { return String.format("%.0f px", v); }

    @FXML private void onClose() { if (stage != null) stage.close(); }
}
