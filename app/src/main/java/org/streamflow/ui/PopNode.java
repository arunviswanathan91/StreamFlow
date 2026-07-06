package org.streamflow.ui;

import java.util.ArrayList;
import java.util.List;

/**
 * A node in the gating hierarchy (FlowJo-style population tree). The root is the
 * whole sample ("All Events", {@code gate == null}); every other node is defined
 * by one {@link CytoPlot.Gate} applied to its parent population. Gate geometry is
 * in raw data space, so a node's events are the root events filtered by the AND of
 * its ancestor gates.
 */
final class PopNode {

    final CytoPlot.Gate gate;        // null only for the root
    PopNode parent;
    final List<PopNode> children = new ArrayList<>();
    int count = -1;                  // cached event count for the current sample
    double parentPct = Double.NaN;   // % of parent population
    // Plot configuration used while gating THIS population, so reopening it (or its parent)
    // shows the child gates on the axes they were drawn on, and they stay editable.
    String viewX, viewY, viewXScale, viewYScale;
    /** True when this gate was moved/resized in a graph window since it was last applied to all samples —
     *  surfaced in the Workstation tree so the user knows to re-propagate it. */
    boolean edited = false;

    /** Geometry snapshots for gate-history replay (#18): each = {timestampMs, xs[], ys[]}. */
    final List<GateSnapshot> history = new ArrayList<>();

    /** One captured gate geometry + the freq it produced, for the history scrubber. */
    record GateSnapshot(long when, double[] xs, double[] ys, double parentPct) {}

    /** Capture the current gate geometry into the history (called after create + each edit). */
    void snapshot() {
        if (gate == null || gate.xs == null) return;
        history.add(new GateSnapshot(System.currentTimeMillis(),
                gate.xs.clone(), gate.ys == null ? null : gate.ys.clone(), parentPct));
        if (history.size() > 50) history.remove(0);   // cap
    }

    PopNode(CytoPlot.Gate gate, PopNode parent) { this.gate = gate; this.parent = parent; }

    boolean isRoot() { return gate == null; }
    String name() { return isRoot() ? "All Events" : gate.name; }

    /** Gates from just below the root down to this node, in application order. */
    List<CytoPlot.Gate> chain() {
        List<CytoPlot.Gate> c = new ArrayList<>();
        for (PopNode n = this; n != null && !n.isRoot(); n = n.parent) c.add(0, n.gate);
        return c;
    }

    /** This node and all descendants, depth-first. */
    List<PopNode> selfAndDescendants() {
        List<PopNode> out = new ArrayList<>();
        collect(out);
        return out;
    }
    private void collect(List<PopNode> out) {
        out.add(this);
        for (PopNode ch : children) ch.collect(out);
    }

    /**
     * Deep structural clone for "apply gate to all samples". Creates new PopNode instances
     * with the same Gate geometry (reused — gates are in data space, valid across samples of the
     * same panel). Counts start as -1/NaN and will be computed when a Graph Window opens.
     */
    PopNode cloneTree(PopNode newParent) {
        PopNode copy = new PopNode(gate, newParent);   // gate geometry shared (data-space, panel-specific)
        copy.viewX = viewX; copy.viewY = viewY;
        copy.viewXScale = viewXScale; copy.viewYScale = viewYScale;
        for (PopNode ch : children) copy.children.add(ch.cloneTree(copy));
        return copy;
    }
}
