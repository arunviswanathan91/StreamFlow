package org.streamflow.ui;

import javafx.application.Platform;
import javafx.scene.Group;
import javafx.scene.Scene;
import javafx.stage.Stage;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Regression test for the crash that stopped Graph Windows opening at all.
 *
 * <p>{@code GraphWindowController.applyAxes()} calls {@code plot.setView(...)} — which only
 * <em>invalidates</em>, deferring {@code recomputeRanges()} to a 50 ms debounce — and then
 * immediately calls {@code applyFmoLines() -> plot.setFmo(...)}, which paints <b>synchronously</b>.
 * On the first open of a sample whose axis scale is Logicle, that painted before any logicle LUT
 * had been built, so {@code drawAxes -> pxX -> sxv} dereferenced a null {@code xLogicle}:
 *
 * <pre>NullPointerException: Cannot invoke "Logicle.scale(double)" because "this.xLogicle" is null</pre>
 *
 * <p>The exception escaped through {@code selectNode -> useData -> loadFromEngine -> open}, so the
 * window died mid-construction and nothing appeared — neither double-click nor the context menu's
 * "open in graphing window" worked, with no visible error.
 *
 * <p>This test drives that exact order in one FX pulse (so the debounce cannot rescue it) and fails
 * if any exception escapes. It guards the golden invariant: double-clicking a population opens a
 * Graph Window.
 */
class CytoPlotLogicleFmoRaceTest {

    private static boolean fxReady;

    @BeforeAll
    static void initToolkit() {
        try {
            CountDownLatch latch = new CountDownLatch(1);
            Platform.startup(latch::countDown);
            latch.await(10, TimeUnit.SECONDS);
            fxReady = true;
        } catch (IllegalStateException already) {
            fxReady = true;
        } catch (Throwable headless) {
            fxReady = false;
        }
        if (fxReady) Platform.setImplicitExit(false);
    }

    @Test
    void settingFmoRightAfterSwitchingToLogicleDoesNotThrow() throws Exception {
        assumeTrue(fxReady, "JavaFX toolkit unavailable (headless) — skipping");

        AtomicReference<Throwable> err = new AtomicReference<>();
        CountDownLatch done = new CountDownLatch(1);

        Platform.runLater(() -> {
            Stage stage = null;
            try {
                int rows = 1000;
                float[] d = new float[rows * 2];
                for (int i = 0; i < rows; i++) {
                    d[i * 2]     = (float) (Math.random() * 50_000);   // CD3
                    d[i * 2 + 1] = (float) (Math.random() * 50_000);   // CD4
                }
                EventData data = new EventData(d, rows, 2, List.of("CD3-A", "CD4-A"));

                CytoPlot plot = new CytoPlot();
                plot.setPrefSize(500, 360);
                plot.resize(500, 360);
                plot.setData(data);

                stage = new Stage();
                stage.setScene(new Scene(new Group(plot)));
                stage.show();
                plot.applyCss();
                plot.layout();

                // The exact applyAxes() order. No time passes between these two calls, so the 50 ms
                // render debounce cannot have run recomputeRanges() — reproducing the real race
                // rather than a slow-machine approximation of it.
                plot.setView("CD3-A", "CD4-A", CytoPlot.Scale.LOGICLE, CytoPlot.Scale.LOGICLE,
                        "pseudocolor");
                plot.setFmo(1234.0, 5678.0);      // painted synchronously - used to NPE here

                // A histogram view takes the same path with only the X LUT in play.
                plot.setView("CD3-A", null, CytoPlot.Scale.LOGICLE, CytoPlot.Scale.LOGICLE,
                        "histogram");
                plot.setFmo(1234.0, Double.NaN);

                plot.dispose();
            } catch (Throwable t) {
                err.set(t);
            } finally {
                if (stage != null) {
                    try { stage.close(); } catch (Throwable ignored) { }
                }
                done.countDown();
            }
        });

        assertTrue(done.await(30, TimeUnit.SECONDS), "timed out waiting for the FX thread");
        if (err.get() != null) {
            throw new AssertionError(
                    "setFmo() immediately after switching to a Logicle view threw — this is the bug "
                            + "that prevented Graph Windows from opening", err.get());
        }
    }
}
