package org.streamflow.ui;

import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

class EmbeddingRulesTest {

    /** Columns: tSNE 1, tSNE 2, CD4, FoxP3. Thresholds are 0, so sign decides positivity. */
    private static EventData data(double[][] rows) {
        float[] flat = new float[rows.length * 4];
        for (int r = 0; r < rows.length; r++)
            for (int c = 0; c < 4; c++) flat[r * 4 + c] = (float) rows[r][c];
        return new EventData(flat, rows.length, 4, List.of("tSNE 1", "tSNE 2", "CD4", "FoxP3"));
    }

    private static final Map<String, Double> THR = Map.of("CD4", 0.0, "FoxP3", 0.0);

    private static EmbeddingRules.Rule rule(String name, Object... pairs) {
        List<EmbeddingRules.Clause> cl = new java.util.ArrayList<>();
        for (int i = 0; i < pairs.length; i += 2)
            cl.add(new EmbeddingRules.Clause((String) pairs[i], (Boolean) pairs[i + 1]));
        return new EmbeddingRules.Rule(name, cl);
    }

    @Test void zscoreCallsClustersAboveTheAverageClusterPositive() {
        List<String> ch = List.of("CD4", "FoxP3");
        // three clusters; CD4 medians {100, 10, 12}, FoxP3 {5, 200, 6}
        double[][] all = {{100, 5}, {10, 200}, {12, 6}};
        // cluster 0 is high CD4, low FoxP3
        Map<String, Boolean> c0 = EmbeddingRules.relativeCalls(ch, all[0], all, null, EmbeddingRules.CallMethod.ZSCORE);
        assertEquals(true, c0.get("CD4"));
        assertEquals(false, c0.get("FoxP3"));
        // cluster 1 is high FoxP3, low CD4
        Map<String, Boolean> c1 = EmbeddingRules.relativeCalls(ch, all[1], all, null, EmbeddingRules.CallMethod.ZSCORE);
        assertEquals(false, c1.get("CD4"));
        assertEquals(true, c1.get("FoxP3"));
    }

    @Test void cutBasedCallsUseTheProvidedThreshold() {
        List<String> ch = List.of("CD4", "FoxP3");
        double[] cut = {50, 50};
        Map<String, Boolean> c = EmbeddingRules.relativeCalls(ch, new double[]{100, 10}, null, cut,
                EmbeddingRules.CallMethod.OTSU_EVENTS);
        assertEquals(true, c.get("CD4"));    // 100 > 50
        assertEquals(false, c.get("FoxP3")); // 10 < 50
    }

    @Test void nameClusterMatchesTheFirstRuleThenFallsBackToTags() {
        Map<String, Boolean> calls = new java.util.LinkedHashMap<>();
        calls.put("CD4", true);
        calls.put("FoxP3", true);
        List<EmbeddingRules.Rule> rules = List.of(
                rule("Treg", "CD4", true, "FoxP3", true),
                rule("Tconv", "CD4", true, "FoxP3", false));
        assertEquals("Treg", EmbeddingRules.nameCluster(calls, rules, t -> t, "Cluster"));

        // no rule matches CD4-only -> fall back to positive tags, using the tag mapper
        Map<String, Boolean> cd4only = new java.util.LinkedHashMap<>();
        cd4only.put("CD4", true);
        cd4only.put("FoxP3", false);
        assertEquals("CD4+", EmbeddingRules.nameCluster(cd4only, List.of(), c -> c, "Cluster"));

        // nothing positive -> fallback string
        Map<String, Boolean> none = Map.of("CD4", false, "FoxP3", false);
        assertEquals("Cluster", EmbeddingRules.nameCluster(none, List.of(), c -> c, "Cluster"));
    }

    @Test void andsItsClauses() {
        EventData d = data(new double[][]{
                {0, 0,  1,  1},   // CD4+ FoxP3+
                {0, 0,  1, -1},   // CD4+ FoxP3-
                {0, 0, -1,  1},   // CD4- FoxP3+
                {0, 0, -1, -1},   // double negative
        });
        boolean[] treg = EmbeddingRules.evaluate(d, rule("Treg", "CD4", true, "FoxP3", true), THR);
        assertArrayEquals(new boolean[]{true, false, false, false}, treg);

        boolean[] cd4Only = EmbeddingRules.evaluate(d, rule("Tconv", "CD4", true, "FoxP3", false), THR);
        assertArrayEquals(new boolean[]{false, true, false, false}, cd4Only);
    }

    @Test void anUnsetThresholdOrMissingChannelYieldsAnEmptyPopulationNotAWrongOne() {
        EventData d = data(new double[][]{{0, 0, 1, 1}});
        assertArrayEquals(new boolean[]{false},
                EmbeddingRules.evaluate(d, rule("x", "CD8", true), THR));              // no such channel
        assertArrayEquals(new boolean[]{false},
                EmbeddingRules.evaluate(d, rule("x", "CD4", true), Map.of()));         // no threshold
        assertArrayEquals(new boolean[]{false},
                EmbeddingRules.evaluate(d, new EmbeddingRules.Rule("x", List.of()), THR)); // no clauses
    }

    @Test void labelsGiveTheFirstMatchingRule() {
        EventData d = data(new double[][]{
                {0, 0, 1,  1},    // matches Treg (rule 0) and CD4+ (rule 1)
                {0, 0, 1, -1},    // matches only CD4+
                {0, 0, -1, -1},   // matches nothing
        });
        List<EmbeddingRules.Rule> rules = List.of(
                rule("Treg", "CD4", true, "FoxP3", true),
                rule("CD4+", "CD4", true));
        assertArrayEquals(new int[]{0, 1, -1}, EmbeddingRules.labels(d, rules, THR));

        // …but the COUNTS are per-rule membership, so the double-positive is in both populations.
        Map<String, Integer> counts = EmbeddingRules.counts(d, rules, THR);
        assertEquals(1, counts.get("Treg"));
        assertEquals(2, counts.get("CD4+"), "an event may belong to two populations even if painted once");
    }

    @Test void outlineNeedsThreePoints() {
        double[] xs = {0, 1}, ys = {0, 1};
        assertNull(EmbeddingRules.outline(xs, ys, new boolean[]{true, true}, 2.0));
        assertNull(EmbeddingRules.outline(xs, ys, new boolean[]{true, false}, 2.0));
    }

    @Test void outlineOfASquareIsItsCorners() {
        double[] xs = {0, 10, 10, 0, 5}, ys = {0, 0, 10, 10, 5};   // 4 corners + 1 interior point
        boolean[] all = {true, true, true, true, true};
        double[][] o = EmbeddingRules.outline(xs, ys, all, 0);      // concavity 0 -> pure convex hull
        assertNotNull(o);
        assertEquals(4, o[0].length, "the interior point must not be on the hull");
        for (int i = 0; i < 4; i++) {
            assertTrue(o[0][i] == 0 || o[0][i] == 10);
            assertTrue(o[1][i] == 0 || o[1][i] == 10);
        }
    }

    /**
     * The load-bearing property: a C-shaped island's convex hull spans the empty bay, claiming
     * territory that holds no events. The concave outline must hug the bay instead. Built as a THICK
     * arc (an annulus sector) so the inner arc supplies the interior points the trim bites with — a
     * zero-thickness arc is already its own convex hull and nothing can be trimmed.
     */
    @Test void concaveOutlineHugsACrescentThatAConvexHullWouldSpan() {
        int steps = 40;
        double[] xs = new double[steps * 2], ys = new double[steps * 2];
        boolean[] keep = new boolean[steps * 2];
        for (int i = 0; i < steps; i++) {
            double a = Math.toRadians(60 + i * 240.0 / (steps - 1));
            xs[i] = Math.cos(a) * 10;  ys[i] = Math.sin(a) * 10;             // outer arc
            xs[steps + i] = Math.cos(a) * 7;  ys[steps + i] = Math.sin(a) * 7;  // inner arc
            keep[i] = keep[steps + i] = true;
        }
        double[][] convex  = EmbeddingRules.outline(xs, ys, keep, 0);
        double[][] concave = EmbeddingRules.outline(xs, ys, keep, 1.2);
        assertNotNull(convex);
        assertNotNull(concave);
        assertTrue(concave[0].length > convex[0].length,
                "concave outline should add vertices to follow the crescent");
        assertTrue(area(concave) < area(convex),
                "concave outline should enclose less area than the convex hull");
    }

    private static double area(double[][] poly) {
        double[] x = poly[0], y = poly[1];
        double a = 0;
        for (int i = 0, n = x.length; i < n; i++) {
            int j = (i + 1) % n;
            a += x[i] * y[j] - x[j] * y[i];
        }
        return Math.abs(a) / 2;
    }
}
