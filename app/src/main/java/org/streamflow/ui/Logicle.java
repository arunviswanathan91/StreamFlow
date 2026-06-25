package org.streamflow.ui;

/**
 * Logicle (biexponential) data scale — Parks–Moore–Roederer, the FlowJo v10
 * default for fluorescence: linear near zero (shows negative compensated values)
 * and logarithmic at high values. Maps raw data → a scaled coordinate where 0
 * maps to W/(M+A) and T maps to 1.0; the curve is antisymmetric about the zero
 * point so negative values mirror positive ones.
 *
 * <p>The reverse map (scaled → data) is the closed-form biexponential; the forward
 * map (data → scaled) is served from a monotone lookup table so binning 500k
 * events per redraw stays cheap. Validated numerically against FlowKit/flowutils
 * ({@code LogicleTest}).
 *
 * @see "Moore WA, Parks DR. Update for the logicle data scale… Cytometry A 2012."
 */
public final class Logicle {

    private static final double LN10 = Math.log(10);
    private static final int N = 16384;            // LUT resolution
    private static final double S_LO = -0.8, S_HI = 1.2; // scaled-coordinate span of the LUT

    private final double a, b, c, d, f, x1;
    private final double[] sGrid = new double[N + 1]; // scaled coords (ascending)
    private final double[] dGrid = new double[N + 1]; // matching data values (ascending)

    /**
     * @param T top of scale (max data value)
     * @param W width of the linear region, in decades (typical 0.5)
     * @param M total number of decades (typical 4.5)
     * @param A additional negative decades (typical 0)
     */
    public Logicle(double T, double W, double M, double A) {
        double w = W / (M + A);
        double x2 = A / (M + A);
        x1 = x2 + w;
        double x0 = x2 + 2 * w;
        b = (M + A) * LN10;
        d = solveD(b, w);
        double ca = Math.exp(x0 * (b + d));
        double fa = Math.exp(b * x1) - ca / Math.exp(d * x1);
        a = T / ((Math.exp(b) - fa) - ca / Math.exp(d));
        c = ca * a;
        f = fa * a;                                  // makes inverse(x1) == 0
        for (int i = 0; i <= N; i++) {
            double s = S_LO + (S_HI - S_LO) * i / N;
            sGrid[i] = s;
            dGrid[i] = invRaw(s);
        }
    }

    /** Root of g(d) = 2·ln(d/b) + w·(b+d); monotonic increasing on (0, b). */
    private static double solveD(double b, double w) {
        if (w == 0) return b;
        double lo = 1e-300, hi = b;
        for (int i = 0; i < 200; i++) {
            double mid = 0.5 * (lo + hi);
            double g = 2 * Math.log(mid / b) + w * (b + mid);
            if (g > 0) hi = mid; else lo = mid;
        }
        return 0.5 * (lo + hi);
    }

    /** Closed-form biexponential, antisymmetric about the zero point x1. */
    private double invRaw(double x) {
        boolean neg = x < x1;
        double s = neg ? 2 * x1 - x : x;
        double v = a * Math.exp(b * s) - c * Math.exp(-d * s) - f;
        return neg ? -v : v;
    }

    /** Scaled coordinate → raw data. */
    public double inverse(double x) { return invRaw(x); }

    /** Raw data → scaled coordinate (LUT + linear interpolation; clamps off-scale). */
    public double scale(double value) {
        if (value <= dGrid[0]) return sGrid[0];
        if (value >= dGrid[N]) return sGrid[N];
        int lo = 0, hi = N;
        while (hi - lo > 1) {
            int mid = (lo + hi) >>> 1;
            if (dGrid[mid] <= value) lo = mid; else hi = mid;
        }
        double t = (value - dGrid[lo]) / (dGrid[hi] - dGrid[lo]);
        return sGrid[lo] + t * (sGrid[hi] - sGrid[lo]);
    }
}
