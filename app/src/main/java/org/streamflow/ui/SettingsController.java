package org.streamflow.ui;

import javafx.fxml.FXML;
import javafx.fxml.FXMLLoader;
import javafx.scene.Scene;
import javafx.scene.control.Button;
import javafx.scene.control.CheckBox;
import javafx.scene.layout.VBox;
import javafx.stage.Stage;

/**
 * The app-wide "Settings" window (File ▸ Settings). Replaces the old unstyled inline Dialog that
 * bundled a bare DPI spinner with a duplicate of the About dialog. General preferences live here;
 * Export format is a shortcut into the existing {@link CopySettingsController} dialog rather than a
 * second, inconsistent copy of the same controls. A single shared window is reused if already open.
 */
public class SettingsController {

    @FXML private CheckBox autoSaveCheck, lightBackgroundCheck, interceptCheck;
    @FXML private Button closeButton;

    private AppSettings settings;
    private Stage stage;

    private static Stage openStage;

    public static void open(AppSettings settings) {
        if (openStage != null && openStage.isShowing()) { openStage.toFront(); openStage.requestFocus(); return; }
        try {
            FXMLLoader loader = new FXMLLoader(
                    SettingsController.class.getResource("/org/streamflow/ui/settings.fxml"));
            VBox root = loader.load();
            SettingsController c = loader.getController();
            c.settings = settings;
            Scene scene = new Scene(root);
            scene.getStylesheets().add(SettingsController.class
                    .getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());
            Stage stage = new Stage();
            stage.setTitle("StreamFLOW — Settings");
            stage.setScene(scene);
            stage.setResizable(false);   // fixed-size utility dialog, consistent with Copy/Export Format
            AppIcons.apply(stage);
            c.stage = stage;
            openStage = stage;
            stage.setOnHidden(e -> { if (openStage == stage) openStage = null; });
            c.bind();
            stage.show();
        } catch (Exception e) {
            throw new RuntimeException("Could not open settings window: " + e.getMessage(), e);
        }
    }

    private void bind() {
        autoSaveCheck.setSelected(settings.defaultAutoSave());
        autoSaveCheck.selectedProperty().addListener((o, a, b) -> settings.setDefaultAutoSave(b));
        lightBackgroundCheck.setSelected(settings.defaultLightBackground());
        lightBackgroundCheck.selectedProperty().addListener((o, a, b) -> settings.setDefaultLightBackground(b));
        interceptCheck.setSelected(settings.defaultInterceptLines());
        interceptCheck.selectedProperty().addListener((o, a, b) -> settings.setDefaultInterceptLines(b));
    }

    @FXML private void onOpenCopySettings() { CopySettingsController.open(settings); }
    @FXML private void onClose() { if (stage != null) stage.close(); }
}
