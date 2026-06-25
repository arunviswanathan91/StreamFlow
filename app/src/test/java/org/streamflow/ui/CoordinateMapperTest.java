package org.streamflow.ui;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Pure-logic test of the gating pixel↔data mapping (no JavaFX/R needed). This is
 * the Java half of the gating calibration: the engine test proves data→gate→count
 * fidelity; this proves canvas-pixel→data fidelity, including the Y-flip.
 */
class CoordinateMapperTest {

    // plot region: x 50..750 px -> data 0..1000 ; y 20(top)..520(bottom) px -> data 500..0
    private final CoordinateMapper m = new CoordinateMapper(
            new double[]{50, 20, 750, 520}, new double[]{0, 1000}, new double[]{0, 500});

    @Test
    void mapsXEdgesAndRoundTrips() {
        assertEquals(0, m.toDataX(50), 1e-9);
        assertEquals(1000, m.toDataX(750), 1e-9);
        assertEquals(50, m.toPxX(0), 1e-9);
        assertEquals(750, m.toPxX(1000), 1e-9);
        // round trip across the range
        for (double px = 50; px <= 750; px += 70) {
            assertEquals(px, m.toPxX(m.toDataX(px)), 1e-6);
        }
    }

    @Test
    void yAxisIsFlipped() {
        // top pixel (20) = max data (500); bottom pixel (520) = min data (0)
        assertEquals(500, m.toDataY(20), 1e-9);
        assertEquals(0, m.toDataY(520), 1e-9);
        assertEquals(20, m.toPxY(500), 1e-9);
        assertEquals(520, m.toPxY(0), 1e-9);
        // midpoint
        assertEquals(250, m.toDataY(270), 1e-6);
    }

    @Test
    void inPlotAndClamp() {
        assertTrue(m.inPlot(400, 270));
        assertFalse(m.inPlot(10, 270));   // left of plot
        assertFalse(m.inPlot(400, 600));  // below plot
        assertEquals(50, m.clampX(10), 1e-9);
        assertEquals(750, m.clampX(900), 1e-9);
        assertEquals(20, m.clampY(5), 1e-9);
        assertEquals(520, m.clampY(900), 1e-9);
    }
}
