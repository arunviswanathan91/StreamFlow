package org.streamflow.ui;

import java.util.List;

/**
 * The statistic-key vocabulary shared by the gate label, the Add Statistic dialog, the Statistics
 * table and Export. A key is a colon-separated string persisted in the workspace, so its grammar is
 * a compatibility surface — extend it, never re-spell it.
 *
 * <pre>
 *   parent | grandparent | total | count      no argument
 *   freq:&lt;population&gt;                          frequency of an arbitrary population
 *   &lt;stat&gt;:&lt;channel&gt;                           median mean geomean mode sd rsd cv rcv mad min max
 *   pct:&lt;p&gt;:&lt;channel&gt;                          arbitrary percentile
 *   correlation:&lt;channelA&gt;:&lt;channelB&gt;          Pearson r
 * </pre>
 *
 * {@code mfi:&lt;channel&gt;} is a legacy alias: it always computed the MEDIAN despite the name, and old
 * workspaces contain it. It parses as {@code median} and is never written again.
 */
public final class StatKeys {

    private StatKeys() {}

    public static final String MEDIAN = "median", MEAN = "mean", GEOMEAN = "geomean", MODE = "mode",
            SD = "sd", RSD = "rsd", CV = "cv", RCV = "rcv", MAD = "mad", MIN = "min", MAX = "max",
            PCT = "pct", CORRELATION = "correlation",
            PARENT = "parent", GRANDPARENT = "grandparent", TOTAL = "total", COUNT = "count", FREQ = "freq";

    /** Statistics computed from one channel's values. */
    private static final List<String> CHANNEL_STATS =
            List.of(MEDIAN, MEAN, GEOMEAN, MODE, SD, RSD, CV, RCV, MAD, MIN, MAX);

    public record Parsed(String stat, String chanA, String chanB, double percentile, String popRef) {}

    /** Never returns null; an unrecognised key parses as {@code stat} with no arguments. */
    public static Parsed parse(String key) {
        if (key == null || key.isBlank()) return new Parsed("", null, null, 0, null);
        int i = key.indexOf(':');
        if (i < 0) return new Parsed(key, null, null, 0, null);
        String stat = key.substring(0, i), rest = key.substring(i + 1);

        if ("mfi".equals(stat)) return new Parsed(MEDIAN, rest, null, 0, null);   // legacy: always the median
        if (FREQ.equals(stat)) return new Parsed(FREQ, null, null, 0, rest);
        if (PCT.equals(stat)) {
            int j = rest.indexOf(':');
            if (j < 0) return new Parsed(PCT, rest, null, 50, null);
            double p = 50;
            try { p = Double.parseDouble(rest.substring(0, j)); } catch (NumberFormatException ignored) {}
            return new Parsed(PCT, rest.substring(j + 1), null, p, null);
        }
        if (CORRELATION.equals(stat)) {
            int j = rest.indexOf(':');
            return j < 0 ? new Parsed(CORRELATION, rest, rest, 0, null)
                         : new Parsed(CORRELATION, rest.substring(0, j), rest.substring(j + 1), 0, null);
        }
        return new Parsed(stat, rest, null, 0, null);
    }

    public static boolean isChannelStat(String stat) { return CHANNEL_STATS.contains(stat); }

    /** Does this statistic read event values (so the node's EventData must be materialised)? */
    public static boolean needsData(String stat) {
        return isChannelStat(stat) || PCT.equals(stat) || CORRELATION.equals(stat);
    }

    /** Short label as it appears next to the number, e.g. "gMean", "rCV". */
    public static String shortLabel(String stat) {
        return switch (stat) {
            case MEDIAN -> "Median";
            case MEAN -> "Mean";
            case GEOMEAN -> "gMean";
            case MODE -> "Mode";
            case SD -> "SD";
            case RSD -> "rSD";
            case CV -> "CV";
            case RCV -> "rCV";
            case MAD -> "MAD";
            case MIN -> "Min";
            case MAX -> "Max";
            case PCT -> "Percentile";
            case CORRELATION -> "r";
            default -> stat;
        };
    }

    /** Compute a channel statistic. Percentiles and correlation are handled here too. */
    public static double compute(Parsed p, EventData nd) {
        double[] a = Stats.values(nd, p.chanA());
        return switch (p.stat()) {
            case MEDIAN -> Stats.median(a);
            case MEAN -> Stats.mean(a);
            case GEOMEAN -> Stats.geoMean(a).value();
            case MODE -> Stats.mode(a);
            case SD -> Stats.sd(a);
            case RSD -> Stats.rsd(a);
            case CV -> Stats.cv(a);
            case RCV -> Stats.rcv(a);
            case MAD -> Stats.mad(a);
            case MIN -> Stats.min(a);
            case MAX -> Stats.max(a);
            case PCT -> Stats.percentile(a, p.percentile());
            case CORRELATION -> Stats.correlation(a, Stats.values(nd, p.chanB()));
            default -> 0;
        };
    }

    /** Percentages and dimensionless ratios must not be formatted like fluorescence intensities. */
    public static boolean isPercentLike(String stat) { return CV.equals(stat) || RCV.equals(stat); }
    public static boolean isRatio(String stat) { return CORRELATION.equals(stat); }

    /** Intensity formatting: scientific for the extremes, thousands-separated otherwise. */
    public static String fmt(double v) {
        double a = Math.abs(v);
        return (a >= 1e5 || (a > 0 && a < 0.01)) ? String.format("%.2e", v) : String.format("%,.0f", v);
    }

    /** Human-readable form of a whole key, for the dialog's "currently shown" list. */
    public static String describe(String key, ChannelAliases aliases) {
        Parsed p = parse(key);
        java.util.function.Function<String, String> lbl =
                c -> aliases == null || c == null ? String.valueOf(c) : aliases.label(c);
        return switch (p.stat()) {
            case PARENT -> "Freq. of Parent";
            case GRANDPARENT -> "Freq. of Grandparent";
            case TOTAL -> "Freq. of Total";
            case COUNT -> "Count";
            case FREQ -> "Freq. of " + p.popRef();
            case PCT -> String.format("%s : P%s", lbl.apply(p.chanA()), trimPct(p.percentile()));
            case CORRELATION -> "Correlation : " + lbl.apply(p.chanA()) + " vs " + lbl.apply(p.chanB());
            default -> lbl.apply(p.chanA()) + " : " + shortLabel(p.stat());
        };
    }

    public static String trimPct(double p) {
        return p == Math.rint(p) ? String.valueOf((long) p) : String.valueOf(p);
    }
}
