package org.streamflow.ui;

import javafx.animation.FadeTransition;
import javafx.animation.RotateTransition;
import javafx.animation.ScaleTransition;
import javafx.scene.Node;
import javafx.scene.Parent;
import javafx.util.Duration;

/**
 * Small, reusable JavaFX-native animation helpers (the {@code Timeline}/{@code Transition} API is
 * the platform's equivalent of CSS animations — there is no literal {@code @keyframes} in JavaFX).
 * Applied opt-in, per-screen, in a few low-risk spots — not a global sweep. See ui-bug-log's
 * "UI/UX modernization" entry for the rationale.
 */
final class UiFx {
    private UiFx() {}

    /** Subtle grow-on-hover for a toolbar-style control (icon button, etc.). ~120ms each way. */
    static void hoverPulse(Node n) {
        ScaleTransition grow = new ScaleTransition(Duration.millis(120), n);
        grow.setToX(1.08); grow.setToY(1.08);
        ScaleTransition shrink = new ScaleTransition(Duration.millis(120), n);
        shrink.setToX(1.0); shrink.setToY(1.0);
        n.setOnMouseEntered(e -> { shrink.stop(); grow.playFromStart(); });
        n.setOnMouseExited(e -> { grow.stop(); shrink.playFromStart(); });
    }

    /** Continuous rotation for a "busy" icon (e.g. the job-progress chip). Call {@code stop()} on the
     *  returned transition when the work finishes. */
    static RotateTransition spin(Node n) {
        RotateTransition rt = new RotateTransition(Duration.seconds(1.0), n);
        rt.setByAngle(360);
        rt.setCycleCount(javafx.animation.Animation.INDEFINITE);
        rt.setInterpolator(javafx.animation.Interpolator.LINEAR);
        rt.play();
        return rt;
    }

    /** Fade-in entrance for a newly-shown window/dialog root. Call once the Scene is attached to a
     *  Stage (e.g. right before {@code stage.show()}). Fade-only (no translate) — an earlier version
     *  also slid the root vertically, which read as "the plot resized/moved" to users; see ui-bug-log
     *  "Options-panel sizing follow-up". */
    static void fadeSlideIn(Parent root) {
        root.setOpacity(0);
        FadeTransition fade = new FadeTransition(Duration.millis(180), root);
        fade.setFromValue(0); fade.setToValue(1);
        fade.play();
    }
}
