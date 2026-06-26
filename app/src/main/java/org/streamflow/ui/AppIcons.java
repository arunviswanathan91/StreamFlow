package org.streamflow.ui;

import javafx.scene.image.Image;
import javafx.stage.Stage;

import java.util.ArrayList;
import java.util.List;

/**
 * Loads the bundled StreamFLOW window icons once and applies them to any {@link Stage}, so every
 * window (main shell, graph windows, the compensation wizard, …) shows the app logo in the title
 * bar and taskbar instead of the default Java cup. Also exposes the large logo for About/Help panels.
 */
public final class AppIcons {

    private static final String[] SIZES = {
            "streamflow-32.png", "streamflow-64.png", "streamflow-128.png", "streamflow-256.png"
    };

    private static List<Image> cached;

    private AppIcons() {}

    private static List<Image> icons() {
        if (cached == null) {
            cached = new ArrayList<>();
            for (String s : SIZES) {
                var url = AppIcons.class.getResource("/org/streamflow/" + s);
                if (url != null) {
                    try { cached.add(new Image(url.toExternalForm())); } catch (Exception ignored) {}
                }
            }
        }
        return cached;
    }

    /** Set the StreamFLOW icon set on a stage (no-op on any failure). */
    public static void apply(Stage stage) {
        try {
            if (stage != null && !icons().isEmpty()) stage.getIcons().setAll(icons());
        } catch (Exception ignored) {}
    }

    /** The full-resolution logo for About / Help panels, or null if unavailable. */
    public static Image logo() {
        try {
            var url = AppIcons.class.getResource("/org/streamflow/streamflow.png");
            return url == null ? null : new Image(url.toExternalForm());
        } catch (Exception e) {
            return null;
        }
    }
}
