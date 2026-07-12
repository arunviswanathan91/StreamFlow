package org.streamflow.ui;

import javafx.application.Platform;
import javafx.scene.Group;
import javafx.scene.Scene;
import javafx.scene.image.WritableImage;
import javafx.scene.paint.Color;
import javafx.stage.Stage;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Regression: an exported plot that carries a colour bar must be white behind the bar.
 *
 * {@code exportImage} widens its crop by {@code colorBarW()} to keep the bar, but {@code paint()}
 * used to white only the plot+axis box. The bar therefore landed on the dark {@code #0D1B2A}
 * surround, and {@code drawColorBar}'s light-mode DARK text became invisible — a dark band with an
 * unreadable channel label down the right edge of every copied figure.
 */
class CytoPlotColorBarExportTest {

    private static boolean fxReady;

    @BeforeAll
    static void initToolkit() {
        try {
            CountDownLatch latch = new CountDownLatch(1);
            Platform.startup(latch::countDown);
            latch.await(10, TimeUnit.SECONDS);
            fxReady = true;
        } catch (IllegalStateException alreadyStarted) {
            fxReady = true;
        } catch (Throwable headless) {
            fxReady = false;
        }
        // This test opens and closes a Stage. With implicit exit on, closing the LAST stage shuts the
        // toolkit down for the whole JVM, and every later FX test (FxmlLoadTest) hangs on runLater.
        if (fxReady) Platform.setImplicitExit(false);
    }

    @Test
    void exportedColourBarSitsOnWhite() throws Exception {
        assumeTrue(fxReady, "JavaFX toolkit unavailable (headless) — skipping");

        AtomicReference<WritableImage> img = new AtomicReference<>();
        AtomicReference<Throwable> err = new AtomicReference<>();
        CountDownLatch done = new CountDownLatch(1);

        Platform.runLater(() -> {
            try {
                // Three channels so there is something to colour by; values spread so the bar has a range.
                int rows = 400;
                float[] d = new float[rows * 3];
                for (int i = 0; i < rows; i++) {
                    d[i * 3]     = (float) Math.cos(i * 0.31) * 50;   // X
                    d[i * 3 + 1] = (float) Math.sin(i * 0.31) * 50;   // Y
                    d[i * 3 + 2] = i;                                  // colour-by channel
                }
                EventData data = new EventData(d, rows, 3, List.of("tSNE 1", "tSNE 2", "BV421-A"));

                CytoPlot plot = new CytoPlot();
                plot.setPrefSize(500, 460);
                plot.resize(500, 460);
                plot.setData(data);
                plot.setAxes("tSNE 1", "tSNE 2");
                plot.setPlotType("dot");
                plot.setColorByChannel("BV421-A", false);   // continuous -> colour bar is drawn

                Stage stage = new Stage();
                stage.setScene(new Scene(new Group(plot)));
                stage.show();
                plot.applyCss();
                plot.layout();

                img.set(plot.exportImage(1.0));
                stage.close();
            } catch (Throwable t) {
                err.set(t);
            } finally {
                done.countDown();
            }
        });

        assertTrue(done.await(30, TimeUnit.SECONDS), "export timed out");
        if (err.get() != null) throw new AssertionError("export threw", err.get());

        WritableImage image = img.get();
        assertNotNull(image, "no image produced");

        int w = (int) image.getWidth(), h = (int) image.getHeight();
        assertTrue(w > 0 && h > 0, "empty image");

        // The last column of the crop is the far edge of the colour-bar strip: background, never ink.
        Color topRight = image.getPixelReader().getColor(w - 1, 0);
        assertEquals(Color.WHITE, topRight,
                "top-right pixel should be white background, not the dark surround");

        // And the strip as a whole must contain no #0D1B2A. Sample the rightmost column.
        Color navy = Color.web("#0D1B2A");
        for (int y = 0; y < h; y++) {
            Color c = image.getPixelReader().getColor(w - 1, y);
            assertTrue(distance(c, navy) > 0.05,
                    "dark surround found at the right edge (y=" + y + "): " + c);
        }
    }

    /**
     * The population legend is drawn INSIDE the plot box, which is filled white in both light and dark
     * mode. Drawing it in the dark-mode axis colour (#E0E0E0) made the key invisible on screen.
     */
    @Test
    void populationLegendIsDarkEnoughToReadOnTheWhitePlot() throws Exception {
        assumeTrue(fxReady, "JavaFX toolkit unavailable (headless) — skipping");

        AtomicReference<WritableImage> img = new AtomicReference<>();
        AtomicReference<Throwable> err = new AtomicReference<>();
        CountDownLatch done = new CountDownLatch(1);

        Platform.runLater(() -> {
            try {
                int rows = 200;
                float[] d = new float[rows * 2];
                for (int i = 0; i < rows; i++) { d[i * 2] = i; d[i * 2 + 1] = i; }
                EventData data = new EventData(d, rows, 2, List.of("tSNE 1", "tSNE 2"));

                CytoPlot plot = new CytoPlot();
                plot.setPrefSize(480, 440);
                plot.resize(480, 440);
                plot.setData(data);
                plot.setAxes("tSNE 1", "tSNE 2");
                plot.setPlotType("dot");
                int[] labels = new int[rows];
                plot.setColorByPopulation(labels,
                        new Color[]{Color.web("#E4572E")},
                        new String[]{"WWWWWWWWWWWW"});   // wide glyphs, so the text certainly renders

                Stage stage = new Stage();
                stage.setScene(new Scene(new Group(plot)));
                stage.show();
                plot.applyCss();
                plot.layout();
                // Snapshot the LIVE canvas, not exportImage(): export forces light mode, so it could
                // never have caught this. The broken case is the on-screen dark theme, whose plot box
                // is nevertheless filled white.
                plot.setLightMode(false);
                plot.refresh();
                img.set(plot.snapshot(new javafx.scene.SnapshotParameters(), null));
                stage.close();
            } catch (Throwable t) {
                err.set(t);
            } finally {
                done.countDown();
            }
        });

        assertTrue(done.await(30, TimeUnit.SECONDS), "render timed out");
        if (err.get() != null) throw new AssertionError("render threw", err.get());

        // Scan the legend band INSIDE the white plot box (x > left margin, y below the top margin), so
        // the dark navy surround cannot be mistaken for legend ink. Light-grey text leaves nothing here.
        WritableImage image = img.get();
        boolean darkInk = false;
        for (int y = 22; y < Math.min(64, (int) image.getHeight()) && !darkInk; y++)
            for (int x = 90; x < Math.min(300, (int) image.getWidth()); x++) {
                Color c = image.getPixelReader().getColor(x, y);
                double lum = 0.2126 * c.getRed() + 0.7152 * c.getGreen() + 0.0722 * c.getBlue();
                if (lum < 0.35 && distance(c, Color.web("#E4572E")) > 0.25) { darkInk = true; break; }
            }
        assertTrue(darkInk, "population legend text should be dark on the white plot, not light grey");
    }

    private static double distance(Color a, Color b) {
        double dr = a.getRed() - b.getRed(), dg = a.getGreen() - b.getGreen(), db = a.getBlue() - b.getBlue();
        return Math.sqrt(dr * dr + dg * dg + db * db);
    }
}
