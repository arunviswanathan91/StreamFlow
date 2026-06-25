package org.streamflow.ui;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;

/**
 * Pins the Java {@link Logicle} to FlowKit/flowutils so the native axis matches
 * FlowJo numerically. Reference values were produced by
 * {@code flowutils.transforms.logicle(x, t=262144, m=4.5, w=0.5, a=0)}.
 */
class LogicleTest {

    private static final double T = 262144, W = 0.5, M = 4.5, A = 0;

    @Test
    void matchesFlowUtilsReference() {
        Logicle lg = new Logicle(T, W, M, A);
        double[][] ref = {
                {-1000, -0.232115},
                {0,      0.111111},
                {100,    0.213181},
                {1000,   0.454338},
                {10000,  0.683833},
                {100000, 0.906928},
                {262144, 1.0},
        };
        for (double[] rc : ref) {
            assertEquals(rc[1], lg.scale(rc[0]), 1e-4, "logicle(" + rc[0] + ")");
        }
    }

    @Test
    void inverseRoundTrips() {
        Logicle lg = new Logicle(T, W, M, A);
        for (double v : new double[]{-500, 0, 50, 5000, 200000, 262144}) {
            double back = lg.inverse(lg.scale(v));
            assertEquals(v, back, 1e-3 * Math.max(1, Math.abs(v)), "round-trip " + v);
        }
    }

    @Test
    void isMonotonic() {
        Logicle lg = new Logicle(T, W, M, A);
        double prev = lg.scale(-2000);
        for (double v = -2000; v <= T; v += 1000) {
            double s = lg.scale(v);
            org.junit.jupiter.api.Assertions.assertTrue(s >= prev, "monotonic at " + v);
            prev = s;
        }
    }
}
