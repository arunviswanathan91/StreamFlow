package org.streamflow.ui;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/** Every value here is hand-computed from the definitions documented on {@link Stats}. */
class StatsTest {

    /** 1..10 — small enough that every statistic can be worked out by hand. */
    private static final double[] V = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10};
    private static final double EPS = 1e-6;

    @Test void meanAndMedian() {
        assertEquals(5.5, Stats.mean(V), EPS);
        assertEquals(5.5, Stats.median(V), EPS);
        assertEquals(6.0, Stats.median(new double[]{2, 10, 6}), EPS);   // odd n
    }

    @Test void otsuSplitsTwoGaussiansAtTheValley() {
        // Two well-separated clouds around 10 and 90; the split must land in the empty gap between.
        java.util.Random rng = new java.util.Random(7);
        double[] v = new double[2000];
        for (int i = 0; i < 1000; i++) v[i] = 10 + rng.nextGaussian() * 3;
        for (int i = 1000; i < 2000; i++) v[i] = 90 + rng.nextGaussian() * 3;
        // Otsu returns the argmax of between-class variance, which is a plateau across the empty gap;
        // the standard first-max lands just past the low cloud. Any value in the gap separates them.
        double t = Stats.otsu(v);
        assertTrue(t > 19 && t < 82, "Otsu threshold should separate the two clouds, was " + t);
    }

    @Test void otsuDegenerateInputs() {
        assertEquals(0, Stats.otsu(new double[]{}), EPS);
        assertEquals(5, Stats.otsu(new double[]{5, 5, 5}), EPS);   // single value -> that value
    }

    @Test void populationSdAndCv() {
        // sum((x-5.5)^2) = 82.5 ; /10 = 8.25 ; sqrt = 2.8722813...
        assertEquals(Math.sqrt(8.25), Stats.sd(V), EPS);
        assertEquals(100 * Math.sqrt(8.25) / 5.5, Stats.cv(V), EPS);
        assertEquals(0, Stats.cv(new double[]{-1, 1}), EPS);            // mean 0 -> undefined -> 0
    }

    @Test void type7Percentiles() {
        assertEquals(5.5, Stats.percentile(V, 50), EPS);
        assertEquals(1.0, Stats.percentile(V, 0), EPS);
        assertEquals(10.0, Stats.percentile(V, 100), EPS);
        // h = (n-1)p/100 = 9*0.1587 = 1.4283 -> s[1] + 0.4283*(s[2]-s[1]) = 2.4283
        assertEquals(2.4283, Stats.percentile(V, Stats.P_LOW), 1e-4);
        // h = 9*0.8413 = 7.5717 -> s[7] + 0.5717*(s[8]-s[7]) = 8.5717
        assertEquals(8.5717, Stats.percentile(V, Stats.P_HIGH), 1e-4);
    }

    @Test void robustStatisticsUseTheConfirmedFormulas() {
        double expectedRsd = (8.5717 - 2.4283) / 2.0;                   // (P84.13 - P15.87)/2
        assertEquals(expectedRsd, Stats.rsd(V), 1e-4);
        assertEquals(100 * expectedRsd / 5.5, Stats.rcv(V), 1e-3);      // denominator is the MEDIAN
    }

    @Test void medianAbsoluteDeviation() {
        // |x-5.5| sorted = .5 .5 1.5 1.5 2.5 2.5 3.5 3.5 4.5 4.5 -> median 2.5
        assertEquals(2.5, Stats.mad(V), EPS);
    }

    @Test void geometricMeanReportsTheEventsItCouldNotUse() {
        Stats.GeoMean g = Stats.geoMean(new double[]{-1, 1, 4});
        assertEquals(2.0, g.value(), EPS);                              // sqrt(1*4), computed on 2 of 3
        assertEquals(1, g.excluded());
        assertEquals(3, g.total());
        assertTrue(g.hasExcluded());

        Stats.GeoMean all = Stats.geoMean(V);
        assertEquals(Math.pow(3628800, 0.1), all.value(), 1e-6);        // 10!^(1/10)
        assertEquals(0, all.excluded());

        Stats.GeoMean none = Stats.geoMean(new double[]{-3, -1, 0});
        assertEquals(0, none.value(), EPS);                             // no positive events at all
        assertEquals(3, none.excluded());
    }

    @Test void pearsonCorrelation() {
        assertEquals(1.0, Stats.correlation(new double[]{1, 2, 3}, new double[]{2, 4, 6}), EPS);
        assertEquals(-1.0, Stats.correlation(new double[]{1, 2, 3}, new double[]{3, 2, 1}), EPS);
        assertEquals(0.0, Stats.correlation(new double[]{1, 2, 3}, new double[]{5, 5, 5}), EPS); // constant
    }

    @Test void modeFindsThePeakOfTheSmoothedDistribution() {
        // 1000 events packed at 7.0 plus a thin uniform 0..10 background: the raw values are all
        // distinct, so only a smoothed histogram can recover 7.
        double[] v = new double[1100];
        for (int i = 0; i < 1000; i++) v[i] = 7.0 + (i % 20) * 1e-3;
        for (int i = 0; i < 100; i++) v[1000 + i] = i / 10.0;
        assertEquals(7.0, Stats.mode(v), 0.15);
    }

    @Test void emptyInputNeverThrows() {
        double[] e = {};
        assertEquals(0, Stats.mean(e), EPS);
        assertEquals(0, Stats.median(e), EPS);
        assertEquals(0, Stats.sd(e), EPS);
        assertEquals(0, Stats.rsd(e), EPS);
        assertEquals(0, Stats.rcv(e), EPS);
        assertEquals(0, Stats.mad(e), EPS);
        assertEquals(0, Stats.mode(e), EPS);
        assertEquals(0, Stats.min(e), EPS);
        assertEquals(0, Stats.max(e), EPS);
        assertEquals(0, Stats.geoMean(e).value(), EPS);
        assertEquals(0, Stats.correlation(e, e), EPS);
    }
}
