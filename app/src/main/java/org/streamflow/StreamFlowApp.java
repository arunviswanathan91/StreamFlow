package org.streamflow;

import javafx.application.Application;
import javafx.application.Platform;
import javafx.fxml.FXMLLoader;
import javafx.scene.Scene;
import javafx.scene.control.Alert;
import javafx.scene.layout.BorderPane;
import javafx.stage.Stage;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;
import org.streamflow.bridge.BridgeService;
import org.streamflow.ui.MainController;

/**
 * JavaFX entry point. Loads the shell immediately, then starts the R engine on
 * a background thread so the window appears without waiting on R's ~seconds-long
 * library load. The {@link MainController} is notified when the engine is ready.
 */
public class StreamFlowApp extends Application {

    private static final Logger log = LoggerFactory.getLogger(StreamFlowApp.class);

    private BridgeService bridge;
    private MainController controller;

    @Override
    public void start(Stage stage) throws Exception {
        FXMLLoader loader = new FXMLLoader(getClass().getResource("/org/streamflow/ui/main.fxml"));
        BorderPane root = loader.load();
        controller = loader.getController();

        // Fit the initial window to the screen's USABLE area (excludes the taskbar), so on small or
        // display-scaled screens the window doesn't open larger than the screen (spilling past the edge
        // and under the taskbar/menu bar). Clamp the preferred size and the minimums to what fits.
        javafx.geometry.Rectangle2D vb = javafx.stage.Screen.getPrimary().getVisualBounds();
        double w = Math.min(1400, vb.getWidth());
        double h = Math.min(900, vb.getHeight());

        Scene scene = new Scene(root, w, h);
        scene.getStylesheets().add(
                getClass().getResource("/org/streamflow/ui/streamflow-dark.css").toExternalForm());

        stage.setTitle("StreamFLOW");
        stage.setMinWidth(Math.min(1100, vb.getWidth()));
        stage.setMinHeight(Math.min(700, vb.getHeight()));
        stage.setScene(scene);
        // Center the restored size, then maximize so the first-run window fills the usable screen.
        stage.setX(vb.getMinX() + Math.max(0, (vb.getWidth() - w) / 2));
        stage.setY(vb.getMinY() + Math.max(0, (vb.getHeight() - h) / 2));
        stage.setMaximized(true);
        org.streamflow.ui.AppIcons.apply(stage);
        // Intercept the window close so unsaved gating changes can be saved first.
        stage.setOnCloseRequest(controller::confirmCloseAndExit);
        stage.show();

        startEngine();
    }

    private void startEngine() {
        controller.setEngineStatus("Starting analysis engine…");
        Thread t = new Thread(() -> {
            try {
                BridgeService svc = BridgeService.start();
                Platform.runLater(() -> {
                    this.bridge = svc;
                    controller.bindBridge(svc);
                    controller.setEngineStatus("Engine ready");
                });
            } catch (Exception e) {
                log.error("Failed to start R engine", e);
                Platform.runLater(() -> {
                    controller.setEngineStatus("Engine failed to start");
                    Alert a = new Alert(Alert.AlertType.ERROR,
                            "The R analysis engine could not be started:\n\n" + e.getMessage());
                    a.setHeaderText("Analysis Engine Error");
                    a.showAndWait();
                });
            }
        }, "engine-start");
        t.setDaemon(true);
        t.start();
    }

    @Override
    public void stop() {
        if (bridge != null) bridge.close();
    }

    public static void main(String[] args) {
        // Last-resort net so a crash on any thread is recorded to the log file
        // (critical once packaged as a windowed exe with no console).
        Thread.setDefaultUncaughtExceptionHandler(
                (t, e) -> log.error("Uncaught exception in thread {}", t.getName(), e));
        log.info("StreamFLOW starting (java {})", System.getProperty("java.version"));
        launch(args);
    }
}
