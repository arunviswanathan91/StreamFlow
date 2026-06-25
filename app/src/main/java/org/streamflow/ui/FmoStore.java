package org.streamflow.ui;

import java.util.HashMap;
import java.util.Map;

/**
 * Session-wide FMO (Fluorescence-Minus-One) reference levels per channel (differentiator #5).
 * When a sample is designated the FMO control for a channel, its 95th-percentile value is stored
 * here; every plot of that channel then draws a dashed reference line at that level so positive
 * gates can be placed consistently. One panel per session → applies across all samples.
 */
public final class FmoStore {

    private final Map<String, Double> levels = new HashMap<>();   // channel -> p95 value
    private final Map<String, String> source = new HashMap<>();   // channel -> sample it came from

    public void set(String channel, double p95, String sampleName) {
        levels.put(channel, p95);
        source.put(channel, sampleName);
    }

    public void clear(String channel) { levels.remove(channel); source.remove(channel); }

    /** Drop every FMO reference (e.g. when a new experiment is loaded). */
    public void clearAll() { levels.clear(); source.clear(); }

    /** FMO level for a channel, or NaN if none set. */
    public double level(String channel) {
        Double v = levels.get(channel);
        return v == null ? Double.NaN : v;
    }

    public String source(String channel) { return source.get(channel); }
    public boolean has(String channel) { return levels.containsKey(channel); }
}
