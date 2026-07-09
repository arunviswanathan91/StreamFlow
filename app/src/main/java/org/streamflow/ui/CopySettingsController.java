package org.streamflow.ui;

import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.control.Slider;
import javafx.scene.control.TextField;
import javafx.scene.input.KeyCode;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;

import java.util.function.DoubleConsumer;

/**
 * The app-wide "Copy / Export Format" window (Graph Window ▸ gear button). Adjusting any control
 * updates {@link AppSettings}, which broadcasts to every open plot so the change is live and every
 * subsequent Copy — in any graph window — uses the same format until changed here again. A single
 * shared window is reused (brought to front if already open). Each slider also has a typable field
 * (no need to press Enter — committing on focus-lost works too) clamped to that slider's own range.
 */
public class CopySettingsController {

    @FXML private Slider pointSlider, axisSlider, labelSlider, chartTitleSlider, chartLegendSlider, dpiSlider;
    @FXML private TextField pointField, axisField, labelField, chartTitleField, chartLegendField, dpiField;
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
            stage.setResizable(false);   // fixed-size utility dialog — no maximize/drag-resize
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
        wireSliderField(pointSlider, pointField, settings.exportPointSize(),
                v -> settings.setExportPointSize((int) Math.round(v)));
        wireSliderField(axisSlider, axisField, settings.exportAxisFontSize(), settings::setExportAxisFontSize);
        wireSliderField(labelSlider, labelField, settings.exportFontSize(), settings::setExportFontSize);
        wireSliderField(chartTitleSlider, chartTitleField, settings.chartTitleFontSize(), settings::setChartTitleFontSize);
        wireSliderField(chartLegendSlider, chartLegendField, settings.chartLegendFontSize(), settings::setChartLegendFontSize);
        wireSliderField(dpiSlider, dpiField, settings.exportDpi(),
                v -> settings.setExportDpi((int) Math.round(v)));

        gateLabelsCheck.setSelected(settings.exportGateLabels());
        gateLabelsCheck.selectedProperty().addListener((o, a, b) -> settings.setExportGateLabels(b));
    }

    /** Link a Slider to a typable TextField, both ways: dragging the slider updates the field text;
     *  typing a number and pressing Enter OR clicking/tabbing away commits it to the slider (and thus
     *  {@code onChange}) — Enter is never required. Anything outside [slider.getMin(), slider.getMax()]
     *  snaps to the nearest limit. */
    private static void wireSliderField(Slider slider, TextField field, double initial, DoubleConsumer onChange) {
        slider.setValue(initial);
        field.setText(fmt(initial));
        boolean[] updatingFromSlider = {false};
        slider.valueProperty().addListener((o, a, b) -> {
            updatingFromSlider[0] = true;
            field.setText(fmt(b.doubleValue()));
            updatingFromSlider[0] = false;
            onChange.accept(b.doubleValue());
        });
        Runnable commit = () -> {
            if (updatingFromSlider[0]) return;
            double parsed;
            try { parsed = Double.parseDouble(field.getText().trim()); }
            catch (NumberFormatException ex) { field.setText(fmt(slider.getValue())); return; }
            double clamped = Math.max(slider.getMin(), Math.min(slider.getMax(), parsed));
            slider.setValue(clamped);   // triggers the listener above, which re-formats the field text
        };
        field.setOnAction(e -> commit.run());              // Enter still works…
        field.focusedProperty().addListener((o, was, isNow) -> { if (!isNow) commit.run(); });   // …but isn't required
        field.setOnKeyPressed(e -> { if (e.getCode() == KeyCode.ESCAPE) field.setText(fmt(slider.getValue())); });
    }

    private static String fmt(double v) { return String.valueOf((long) Math.round(v)); }

    @FXML private void onClose() { if (stage != null) stage.close(); }
}
