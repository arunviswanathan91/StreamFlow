package org.streamflow.ui;

import java.util.Arrays;

/**
 * Every population statistic StreamFLOW reports, in one place, matching FlowJo's definitions.
 *
 * Conventions that a reader cannot infer from the code and that change the numbers:
 *
 * <ul>
 *   <li><b>SD is the population SD</b> (divide by n, not n-1). A gated population is the whole
 *       population of interest, not a sample drawn from one. FlowJo does the same. At flow-cytometry
 *       event counts the difference is below the reported precision anyway.</li>
 *   <li><b>Percentiles use linear interpolation</b> between order statistics (h = (n-1)p/100), i.e.
 *       R's default type-7. The robust statistics depend on P15.87/P84.13, which sit between order
 *       statistics, so a nearest-rank percentile would visibly quantise rSD on small populations.</li>
 *   <li><b>Geometric mean silently dropping events is not acceptable.</b> Compensated data contains
 *       negatives, and {@code exp(mean(ln x))} is undefined for them. We exclude x &le; 0 and
 *       <i>report how many</i> ({@link #geoMean}) so the caller can surface it. Never call the
 *       double-returning overload without showing that count somewhere.</li>
 *   <li><b>Mode is the peak of the smoothed distribution</b>, not the most frequent raw value —
 *       on continuous fluorescence data every value is unique, so a raw mode is meaningless.</li>
 * </ul>
 */
public final class Stats {

    private Stats() {}

    /** Percentiles that define the robust statistics: ±1 SD of a Gaussian. */
    public static final double P_LOW = 15.87, P_HIGH = 84.13;

    private static final int MODE_BINS = 256;
    private static final int MODE_SMOOTH_PASSES = 3;
    private static final int MODE_SMOOTH_WIDTH = 5;

    /** Geometric mean plus the events it could not use. */
    public record GeoMean(double value, int excluded, int total) {
        public boolean hasExcluded() { return excluded > 0; }
    }

    public static double[] values(EventData d, String channel) {
        int c = d == null ? -1 : d.indexOf(channel);
        if (c < 0) return new double[0];
        double[] v = new double[d.rows()];
        for (int r = 0; r < v.length; r++) v[r] = d.get(r, c);
        return v;
    }

    public static double mean(double[] v) {
        if (v.length == 0) return 0;
        double s = 0;
        for (double x : v) s += x;
        return s / v.length;
    }

    public static double median(double[] v) {
        return v.length == 0 ? 0 : medianSorted(sorted(v));
    }

    /** Population standard deviation (÷n). See the class note. */
    public static double sd(double[] v) {
        if (v.length == 0) return 0;
        double m = mean(v), s = 0;
        for (double x : v) s += (x - m) * (x - m);
        return Math.sqrt(s / v.length);
    }

    /** CV = 100 · SD / Mean. Undefined at mean 0. */
    public static double cv(double[] v) {
        double m = mean(v);
        return m == 0 ? 0 : 100 * sd(v) / m;
    }

    /** rSD = (P84.13 − P15.87) / 2 — an outlier-insensitive SD. */
    public static double rsd(double[] v) {
        if (v.length == 0) return 0;
        double[] s = sorted(v);
        return (percentileSorted(s, P_HIGH) - percentileSorted(s, P_LOW)) / 2.0;
    }

    /** rCV = 100 · rSD / Median. Note the denominator is the MEDIAN, not the mean. */
    public static double rcv(double[] v) {
        if (v.length == 0) return 0;
        double[] s = sorted(v);
        double med = medianSorted(s);
        if (med == 0) return 0;
        double r = (percentileSorted(s, P_HIGH) - percentileSorted(s, P_LOW)) / 2.0;
        return 100 * r / med;
    }

    /** Median absolute deviation: median(|x − median(x)|). Not scaled by 1.4826. */
    public static double mad(double[] v) {
        if (v.length == 0) return 0;
        double med = median(v);
        double[] dev = new double[v.length];
        for (int i = 0; i < v.length; i++) dev[i] = Math.abs(v[i] - med);
        return median(dev);
    }

    public static double percentile(double[] v, double p) {
        return v.length == 0 ? 0 : percentileSorted(sorted(v), p);
    }

    public static double min(double[] v) {
        double m = Double.MAX_VALUE;
        for (double x : v) m = Math.min(m, x);
        return v.length == 0 ? 0 : m;
    }

    /**
     * Otsu's threshold: the value that maximises between-class variance over a 256-bin histogram of the
     * data. Used to split a marker's events into negative/positive without a control — the between-class
     * criterion finds the valley of a bimodal distribution. Returns the midpoint of the chosen bin;
     * a degenerate (single-value) input returns that value.
     */
    public static double otsu(double[] v) {
        if (v.length == 0) return 0;
        double lo = min(v), hi = max(v);
        if (hi <= lo) return lo;

        int bins = 256;
        long[] hist = new long[bins];
        double scale = bins / (hi - lo);
        for (double x : v) {
            int b = (int) ((x - lo) * scale);
            if (b >= bins) b = bins - 1;
            if (b < 0) b = 0;
            hist[b]++;
        }

        long total = v.length;
        double sum = 0;
        for (int b = 0; b < bins; b++) sum += (double) b * hist[b];

        double sumB = 0, wB = 0, maxVar = -1;
        int threshBin = 0;
        for (int b = 0; b < bins; b++) {
            wB += hist[b];
            if (wB == 0) continue;
            double wF = total - wB;
            if (wF == 0) break;
            sumB += (double) b * hist[b];
            double mB = sumB / wB;
            double mF = (sum - sumB) / wF;
            double between = wB * wF * (mB - mF) * (mB - mF);
            if (between > maxVar) { maxVar = between; threshBin = b; }
        }
        return lo + (threshBin + 0.5) / scale;
    }

    public static double max(double[] v) {
        double m = -Double.MAX_VALUE;
        for (double x : v) m = Math.max(m, x);
        return v.length == 0 ? 0 : m;
    }

    /**
     * Geometric mean over the strictly positive events, reporting how many were excluded.
     * Callers must show {@link GeoMean#excluded} — a gMean computed from 40% of a population is a
     * different quantity from one computed from all of it, and silently reporting it as the same
     * number is how compensated data gets misinterpreted.
     */
    public static GeoMean geoMean(double[] v) {
        double sum = 0;
        int n = 0;
        for (double x : v) if (x > 0) { sum += Math.log(x); n++; }
        return new GeoMean(n == 0 ? 0 : Math.exp(sum / n), v.length - n, v.length);
    }

    /** Pearson correlation between two equal-length channels; 0 if either is constant. */
    public static double correlation(double[] a, double[] b) {
        int n = Math.min(a.length, b.length);
        if (n == 0) return 0;
        double ma = 0, mb = 0;
        for (int i = 0; i < n; i++) { ma += a[i]; mb += b[i]; }
        ma /= n; mb /= n;
        double sab = 0, saa = 0, sbb = 0;
        for (int i = 0; i < n; i++) {
            double da = a[i] - ma, db = b[i] - mb;
            sab += da * db; saa += da * da; sbb += db * db;
        }
        double denom = Math.sqrt(saa * sbb);
        return denom == 0 ? 0 : sab / denom;
    }

    /**
     * The peak of the smoothed distribution: bin into {@value #MODE_BINS} bins over [min, max], run a
     * few box-blur passes (a cheap Gaussian), and return the centre of the tallest bin.
     */
    public static double mode(double[] v) {
        if (v.length == 0) return 0;
        double lo = min(v), hi = max(v);
        if (hi <= lo) return lo;
        double[] h = new double[MODE_BINS];
        double scale = MODE_BINS / (hi - lo);
        for (double x : v) {
            int b = (int) ((x - lo) * scale);
            if (b >= MODE_BINS) b = MODE_BINS - 1;
            if (b < 0) b = 0;
            h[b]++;
        }
        for (int p = 0; p < MODE_SMOOTH_PASSES; p++) h = boxBlur(h, MODE_SMOOTH_WIDTH);
        int peak = 0;
        for (int i = 1; i < h.length; i++) if (h[i] > h[peak]) peak = i;
        return lo + (peak + 0.5) / scale;
    }

    // ---- internals ----------------------------------------------------------

    private static double[] sorted(double[] v) {
        double[] s = v.clone();
        Arrays.sort(s);
        return s;
    }

    private static double medianSorted(double[] s) {
        int n = s.length;
        if (n == 0) return 0;
        return n % 2 == 1 ? s[n / 2] : (s[n / 2 - 1] + s[n / 2]) / 2.0;
    }

    /** Type-7 (linear interpolation between order statistics). */
    private static double percentileSorted(double[] s, double p) {
        int n = s.length;
        if (n == 0) return 0;
        if (n == 1) return s[0];
        double h = (n - 1) * Math.max(0, Math.min(100, p)) / 100.0;
        int lo = (int) Math.floor(h);
        int hi = Math.min(lo + 1, n - 1);
        return s[lo] + (h - lo) * (s[hi] - s[lo]);
    }

    private static double[] boxBlur(double[] h, int width) {
        int r = width / 2;
        double[] out = new double[h.length];
        for (int i = 0; i < h.length; i++) {
            double s = 0;
            int c = 0;
            for (int k = -r; k <= r; k++) {
                int j = i + k;
                if (j >= 0 && j < h.length) { s += h[j]; c++; }
            }
            out[i] = s / c;
        }
        return out;
    }
}
