package org.streamflow.ui;

import javafx.application.Platform;
import javafx.scene.Group;
import javafx.scene.Scene;
import javafx.scene.canvas.Canvas;
import javafx.scene.input.MouseButton;
import javafx.scene.input.MouseEvent;
import javafx.stage.Stage;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.lang.reflect.Field;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Does dragging an interval on a HISTOGRAM CytoPlot fire onGateDrawn? This is exactly the
 * PositivityDialog "Draw threshold" path, which a user reported as doing nothing. We drive the
 * canvas's own mouse handlers so the test exercises the real press/drag/release chain.
 */
class CytoPlotIntervalDrawTest {

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
    void draggingAnIntervalOnAHistogramFiresTheCallback() throws Exception {
        assumeTrue(fxReady, "JavaFX toolkit unavailable (headless) — skipping");

        AtomicReference<CytoPlot.Gate> drawn = new AtomicReference<>();
        AtomicReference<Throwable> err = new AtomicReference<>();
        CountDownLatch done = new CountDownLatch(1);

        Platform.runLater(() -> {
            try {
                int rows = 2000;
                float[] d = new float[rows];
                for (int i = 0; i < rows; i++) d[i] = (float) (Math.random() * 1000);
                EventData data = new EventData(d, rows, 1, List.of("Alexa Fluor 700-A"));

                CytoPlot plot = new CytoPlot();
                plot.setPrefSize(500, 360);
                plot.resize(500, 360);
                plot.setPlotType("histogram");
                plot.setData(data);
                plot.setAxes("Alexa Fluor 700-A", null);
                plot.setOnGateDrawn(drawn::set);
                plot.setTool("Interval");

                Stage stage = new Stage();
                stage.setScene(new Scene(new Group(plot)));
                stage.show();
                plot.applyCss();
                plot.layout();

                Canvas canvas = canvasOf(plot);
                double y = 180;
                fire(canvas, MouseEvent.MOUSE_PRESSED, 150, y);
                fire(canvas, MouseEvent.MOUSE_DRAGGED, 260, y);
                fire(canvas, MouseEvent.MOUSE_RELEASED, 300, y);

                stage.close();
            } catch (Throwable t) {
                err.set(t);
            } finally {
                done.countDown();
            }
        });

        assertTrue(done.await(30, TimeUnit.SECONDS), "timed out");
        if (err.get() != null) throw new AssertionError("draw threw", err.get());
        assertNotNull(drawn.get(), "an interval drag on a histogram should fire onGateDrawn");
        assertTrue(drawn.get().xs != null && drawn.get().xs.length >= 2,
                "interval gate should carry its two x bounds");
    }

    private static Canvas canvasOf(CytoPlot plot) throws Exception {
        Field f = CytoPlot.class.getDeclaredField("canvas");
        f.setAccessible(true);
        return (Canvas) f.get(plot);
    }

    private static void fire(Canvas c, javafx.event.EventType<MouseEvent> type, double x, double y) {
        MouseEvent e = new MouseEvent(type, x, y, x, y, MouseButton.PRIMARY, 1,
                false, false, false, false, true, false, false, true, false, false, null);
        c.fireEvent(e);
    }
}
