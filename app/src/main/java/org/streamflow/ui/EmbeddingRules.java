package org.streamflow.ui;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Marker-defined populations over an embedding: "Treg = CD45+ AND CD4+ AND FoxP3+".
 *
 * A rule is evaluated against the marker columns the embedding already carries, so no engine round-trip
 * is needed. The population a rule defines is **the boolean set** — the events satisfying the rule.
 * The outline this class also computes is a *visual footprint* of where those events sit on the map and
 * has no say in membership. That separation is deliberate: t-SNE coordinates are a non-metric layout, so
 * letting a polygon decide membership would make the biology depend on the drawing.
 *
 * Pure functions over arrays: no JavaFX, no engine, fully unit-testable.
 */
public final class EmbeddingRules {

    private EmbeddingRules() {}

    /** One clause: a channel must be above (positive) or below (negative) its threshold. */
    public record Clause(String channel, boolean positive) {}

    /** A named population defined by ANDing its clauses. An empty clause list matches nothing. */
    public record Rule(String name, List<Clause> clauses) {
        public Rule {
            clauses = List.copyOf(clauses);
        }
        @Override public String toString() {
            if (clauses.isEmpty()) return name + " = (no clauses)";
            StringBuilder sb = new StringBuilder(name).append(" = ");
            for (int i = 0; i < clauses.size(); i++) {
                if (i > 0) sb.append(" AND ");
                sb.append(clauses.get(i).channel()).append(clauses.get(i).positive() ? "+" : "-");
            }
            return sb.toString();
        }
    }

    // ---- cluster naming (threshold-free) ------------------------------------

    /** How a cluster's per-marker +/- call is decided when no hard threshold is used. */
    public enum CallMethod { THRESHOLD, ZSCORE, OTSU_EVENTS, GLOBAL_MEDIAN }

    /**
     * Per-marker positive/negative call for one cluster, keyed by channel.
     *
     * @param channels    the marker channels to call (only these are considered)
     * @param clusterMed  this cluster's median per channel (aligned to {@code channels})
     * @param allMed      every cluster's median per channel — {@code allMed[c][j]} (for ZSCORE)
     * @param eventCut    a precomputed cut per channel (for OTSU_EVENTS / GLOBAL_MEDIAN / THRESHOLD)
     */
    public static Map<String, Boolean> relativeCalls(List<String> channels, double[] clusterMed,
                                                      double[][] allMed, double[] eventCut,
                                                      CallMethod method) {
        Map<String, Boolean> calls = new LinkedHashMap<>();
        for (int j = 0; j < channels.size(); j++) {
            boolean pos;
            switch (method) {
                case ZSCORE -> {
                    double mean = 0;
                    for (double[] cm : allMed) mean += cm[j];
                    mean /= allMed.length;
                    double var = 0;
                    for (double[] cm : allMed) var += (cm[j] - mean) * (cm[j] - mean);
                    double sd = Math.sqrt(var / allMed.length);
                    // Above the average cluster. sd==0 (all clusters equal) -> never positive.
                    pos = sd > 0 && (clusterMed[j] - mean) / sd > 0;
                }
                case THRESHOLD, OTSU_EVENTS, GLOBAL_MEDIAN ->
                        pos = eventCut != null && clusterMed[j] > eventCut[j];
                default -> pos = false;
            }
            calls.put(channels.get(j), pos);
        }
        return calls;
    }

    /**
     * Name a cluster from its per-marker calls. First rule whose clauses all match wins (rules ordered,
     * as in {@link #labels}); otherwise the positive markers are joined via {@code tagOf}
     * (e.g. "CD45+ CD4+ FOXP3+"), falling back to {@code fallback} when nothing is positive.
     */
    public static String nameCluster(Map<String, Boolean> calls, List<Rule> rules,
                                     java.util.function.Function<String, String> tagOf, String fallback) {
        for (Rule r : rules) {
            if (r.clauses().isEmpty()) continue;
            boolean all = true;
            for (Clause c : r.clauses()) {
                Boolean call = calls.get(c.channel());
                if (call == null || call != c.positive()) { all = false; break; }
            }
            if (all) return r.name();
        }
        List<String> pos = new ArrayList<>();
        for (Map.Entry<String, Boolean> e : calls.entrySet())
            if (Boolean.TRUE.equals(e.getValue())) {
                String tag = tagOf == null ? e.getKey() : tagOf.apply(e.getKey());
                pos.add((tag == null ? e.getKey() : tag) + "+");
            }
        return pos.isEmpty() ? fallback : String.join(" ", pos);
    }

    /**
     * Rows of {@code data} satisfying every clause of {@code rule}.
     * A clause whose channel is missing from the data matches nothing, so an incomplete embedding
     * yields an empty population rather than a silently wrong one.
     */
    public static boolean[] evaluate(EventData data, Rule rule, Map<String, Double> thresholds) {
        boolean[] out = new boolean[data.rows()];
        if (rule.clauses().isEmpty()) return out;

        int[] cols = new int[rule.clauses().size()];
        double[] thr = new double[cols.length];
        boolean[] wantPositive = new boolean[cols.length];
        for (int c = 0; c < cols.length; c++) {
            Clause cl = rule.clauses().get(c);
            cols[c] = data.indexOf(cl.channel());
            Double t = thresholds.get(cl.channel());
            if (cols[c] < 0 || t == null) return out;   // unknown channel or unset threshold -> empty
            thr[c] = t;
            wantPositive[c] = cl.positive();
        }

        Arrays.fill(out, true);
        for (int r = 0; r < out.length; r++) {
            for (int c = 0; c < cols.length; c++) {
                boolean pos = data.get(r, cols[c]) > thr[c];
                if (pos != wantPositive[c]) { out[r] = false; break; }
            }
        }
        return out;
    }

    /**
     * Assign each row to the FIRST rule it matches, or -1. Rules therefore have priority order, which
     * is what makes the colouring deterministic when a cell is both CD4+ and CD4+FoxP3+. Membership
     * counts per rule come from {@link #evaluate}, not from here — an event can belong to several
     * populations at once even though it can only be painted one colour.
     */
    public static int[] labels(EventData data, List<Rule> rules, Map<String, Double> thresholds) {
        int[] lab = new int[data.rows()];
        Arrays.fill(lab, -1);
        for (int i = rules.size() - 1; i >= 0; i--) {          // paint backwards so rule 0 wins
            boolean[] m = evaluate(data, rules.get(i), thresholds);
            for (int r = 0; r < lab.length; r++) if (m[r]) lab[r] = i;
        }
        return lab;
    }

    /** Per-rule event counts (independent of {@link #labels}' first-match-wins colouring). */
    public static Map<String, Integer> counts(EventData data, List<Rule> rules, Map<String, Double> thresholds) {
        Map<String, Integer> out = new LinkedHashMap<>();
        for (Rule r : rules) {
            int n = 0;
            for (boolean b : evaluate(data, r, thresholds)) if (b) n++;
            out.put(r.name(), n);
        }
        return out;
    }

    // ---- outline ------------------------------------------------------------

    /**
     * A closed outline of the selected events in (x, y) space, as {@code [xs, ys]}.
     *
     * This is a <b>concave</b> hull: a convex hull would swallow the empty middle of a crescent-shaped
     * island and imply the outline contains events it does not. We use the standard "chi-shape" trim —
     * build the convex hull, then repeatedly bite into the longest boundary edge if a nearby interior
     * point lets us replace it with two shorter ones. {@code concavity} is the multiple of the median
     * edge length above which an edge is considered too long to keep; larger = closer to convex.
     *
     * Returns null for fewer than 3 points, which cannot bound an area.
     */
    public static double[][] outline(double[] xs, double[] ys, boolean[] keep, double concavity) {
        List<double[]> pts = new ArrayList<>();
        for (int i = 0; i < keep.length; i++) if (keep[i]) pts.add(new double[]{xs[i], ys[i]});
        if (pts.size() < 3) return null;

        List<double[]> hull = convexHull(pts);
        if (hull.size() < 3) return null;
        List<double[]> shape = concaveTrim(hull, pts, concavity);

        double[] ox = new double[shape.size()], oy = new double[shape.size()];
        for (int i = 0; i < shape.size(); i++) { ox[i] = shape.get(i)[0]; oy[i] = shape.get(i)[1]; }
        return new double[][]{ox, oy};
    }

    /** Andrew's monotone chain. Counter-clockwise, no repeated endpoint. */
    static List<double[]> convexHull(List<double[]> input) {
        List<double[]> p = new ArrayList<>(input);
        p.sort((a, b) -> a[0] != b[0] ? Double.compare(a[0], b[0]) : Double.compare(a[1], b[1]));
        if (p.size() < 3) return p;

        List<double[]> hull = new ArrayList<>();
        for (double[] pt : p) {                                   // lower
            while (hull.size() >= 2 && cross(hull.get(hull.size() - 2), hull.get(hull.size() - 1), pt) <= 0)
                hull.remove(hull.size() - 1);
            hull.add(pt);
        }
        int lower = hull.size() + 1;
        for (int i = p.size() - 2; i >= 0; i--) {                 // upper
            double[] pt = p.get(i);
            while (hull.size() >= lower && cross(hull.get(hull.size() - 2), hull.get(hull.size() - 1), pt) <= 0)
                hull.remove(hull.size() - 1);
            hull.add(pt);
        }
        hull.remove(hull.size() - 1);                             // last == first
        return hull;
    }

    private static double cross(double[] o, double[] a, double[] b) {
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    }

    private static double dist(double[] a, double[] b) {
        return Math.hypot(a[0] - b[0], a[1] - b[1]);
    }

    /**
     * Bite into over-long hull edges using the nearest interior point, until every edge is within
     * {@code concavity} × the median original edge length. Bounded by a hard iteration cap so a
     * pathological cloud can never spin here.
     */
    private static List<double[]> concaveTrim(List<double[]> hull, List<double[]> all, double concavity) {
        if (concavity <= 0) return hull;

        double[] lens = new double[hull.size()];
        for (int i = 0; i < hull.size(); i++) lens[i] = dist(hull.get(i), hull.get((i + 1) % hull.size()));
        double[] sorted = lens.clone();
        Arrays.sort(sorted);
        double limit = concavity * sorted[sorted.length / 2];
        if (limit <= 0) return hull;

        List<double[]> shape = new ArrayList<>(hull);
        List<double[]> interior = new ArrayList<>(all);
        interior.removeIf(shape::contains);

        int cap = 4 * all.size() + 64;
        boolean changed = true;
        while (changed && cap-- > 0) {
            changed = false;
            for (int i = 0; i < shape.size(); i++) {
                double[] a = shape.get(i), b = shape.get((i + 1) % shape.size());
                double edge = dist(a, b);
                if (edge <= limit) continue;

                double[] best = null;
                double bestCost = Double.MAX_VALUE;
                for (double[] c : interior) {
                    double cost = dist(a, c) + dist(c, b);
                    // Only accept a point that genuinely shortens the path, or the outline degenerates.
                    if (cost < bestCost && cost < edge * 1.8 && dist(a, c) < edge && dist(c, b) < edge) {
                        bestCost = cost; best = c;
                    }
                }
                if (best != null) {
                    shape.add(i + 1, best);
                    interior.remove(best);
                    changed = true;
                    break;
                }
            }
        }
        return shape;
    }
}
