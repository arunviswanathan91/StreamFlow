package org.streamflow.ui;

import javafx.application.Platform;
import javafx.beans.property.BooleanProperty;
import javafx.beans.property.SimpleBooleanProperty;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Session-wide store so gating survives closing a graph window. The workspace owns, per sample
 * (keyed by FCS file name): the gating tree (root {@link PopNode}) and the cached event data.
 * Graph windows — top-level and drill-down children — are just views that read/write this shared
 * tree, so closing one and reopening the sample restores the gates and stats (FlowJo's model).
 * Held in {@link AppContext}.
 *
 * <p>The workstation module binds to {@link #sampleNames()} (updated after every {@code load_fcs})
 * and registers a tree-change listener so it refreshes when gates are drawn or removed in any
 * Graph Window.
 */
public final class WorkspaceModel {

    private final Map<String, PopNode> trees = new HashMap<>();
    private final Map<String, EventData> events = new HashMap<>();
    private final Map<String, Integer> gateSeq = new HashMap<>();
    private final Map<String, Integer> eventCounts = new HashMap<>();   // total events per sample (from load_fcs), for QC
    private PopNode gateClipboard;                                       // copy/paste a gate subtree across samples

    // True once Apply Compensation has succeeded in this session.
    private final BooleanProperty compApplied = new SimpleBooleanProperty(false);

    // Observable sample list — populated by SetupController after load_fcs completes.
    private final ObservableList<String> allSamples = FXCollections.observableArrayList();
    // Channel names for the loaded experiment (all samples share one panel).
    private final List<String> channelNames = new ArrayList<>();
    // Listeners notified (on the FX thread) whenever any gating tree changes.
    private final List<Runnable> treeListeners = new ArrayList<>();
    // Listeners notified (on the FX thread, with the sample name) when event data is cached.
    private final List<java.util.function.Consumer<String>> dataListeners = new ArrayList<>();

    // ---- compensation state --------------------------------------------------

    /** Observable flag — true once Apply Compensation succeeds. Bind UI badges to this. */
    public BooleanProperty compApplied() { return compApplied; }

    // ---- sample + channel registry (from SetupController) --------------------

    /** Observable list of all loaded sample file names; updated after every {@code load_fcs}. */
    public ObservableList<String> sampleNames() { return allSamples; }

    /** Replace the sample list. Must be called on the FX thread (or Platform.runLater). */
    public void setSamples(List<String> names) {
        if (Platform.isFxApplicationThread()) {
            allSamples.setAll(names);
        } else {
            Platform.runLater(() -> allSamples.setAll(names));
        }
    }

    /** Store the channel list for the current experiment panel. */
    public void setChannelNames(List<String> names) {
        channelNames.clear();
        channelNames.addAll(names);
    }

    /** The channel list for the current experiment (empty until data is loaded). */
    public List<String> channelNames() { return Collections.unmodifiableList(channelNames); }

    // ---- gating tree ---------------------------------------------------------

    /** The sample's gating tree root ("All Events"), created empty on first access. */
    public PopNode treeFor(String sample) {
        return trees.computeIfAbsent(sample, s -> new PopNode(null, null));
    }
    public boolean hasTree(String sample) { return trees.containsKey(sample); }

    /** Replace the gating tree root for a sample (used by "Apply gates to all samples"). */
    public void replaceTree(String sample, PopNode newRoot) { trees.put(sample, newRoot); }

    /** Cached full events for a sample, or null if not yet fetched from the engine. */
    public EventData data(String sample) { return events.get(sample); }
    public void putData(String sample, EventData d) {
        events.put(sample, d);
        notifyDataChanged(sample);
    }

    /** Register a listener called (on the FX thread) when event data is cached for a sample. */
    public void addDataChangeListener(java.util.function.Consumer<String> listener) {
        dataListeners.add(listener);
    }

    private void notifyDataChanged(String sample) {
        if (Platform.isFxApplicationThread()) dataListeners.forEach(l -> l.accept(sample));
        else Platform.runLater(() -> dataListeners.forEach(l -> l.accept(sample)));
    }

    /** Total event count per sample (from the load_fcs summary), used for QC before a sample is opened. */
    public void setEventCount(String sample, int events) { eventCounts.put(sample, events); }
    public int eventCount(String sample) {
        Integer n = eventCounts.get(sample);
        if (n != null) return n;
        EventData d = events.get(sample);
        return d != null ? d.rows() : -1;
    }

    /** Shared gate clipboard for copy/paste of a gate subtree across samples (Workstation). */
    public void setGateClipboard(PopNode subtree) { this.gateClipboard = subtree; }
    public PopNode gateClipboard() { return gateClipboard; }

    public int nextSeq(String sample) {
        int n = gateSeq.getOrDefault(sample, 1);
        gateSeq.put(sample, n + 1);
        return n;
    }

    public java.util.Set<String> samples() { return trees.keySet(); }

    /** Wipe ALL experiment data (trees, events, event counts, channel names, dirty flag, sample list).
     *  Called before loading a new FCS folder or opening a workspace over an existing session so that
     *  no stale data from the previous session lingers in the UI. */
    public void clearAll() {
        trees.clear();
        events.clear();
        gateSeq.clear();
        eventCounts.clear();
        gateClipboard = null;
        channelNames.clear();
        dirty = false;
        if (Platform.isFxApplicationThread()) {
            allSamples.clear();
            treeListeners.forEach(Runnable::run);
        } else {
            Platform.runLater(() -> {
                allSamples.clear();
                treeListeners.forEach(Runnable::run);
            });
        }
    }

    // ---- tree-change notifications (for the Workstation) ----------------------

    /** Register a listener called (on the FX thread) when any gating tree changes. */
    public void addTreeChangeListener(Runnable listener) { treeListeners.add(listener); }

    /** Notify all registered listeners that a gating tree has changed. */
    public void notifyTreeChanged() {
        dirty = true;   // a gate was drawn/edited/removed → there are unsaved changes
        if (Platform.isFxApplicationThread()) {
            treeListeners.forEach(Runnable::run);
        } else {
            Platform.runLater(() -> treeListeners.forEach(Runnable::run));
        }
    }

    // ---- unsaved-changes tracking (autosave + close prompts) ------------------

    private boolean dirty = false;

    /** True if there are gating/analysis changes not yet written to a workspace file. */
    public boolean isDirty() { return dirty; }
    /** Mark unsaved changes (e.g. an analysis was run). */
    public void markDirty() { dirty = true; }
    /** Clear the dirty flag (after a save, or after loading a fresh experiment/workspace). */
    public void markClean() { dirty = false; }

    // ---- §14 open-window registry (focus-existing instead of opening duplicate) -
    private final Map<String, javafx.stage.Stage> openWindows = new HashMap<>();

    public void registerWindow(String sample, javafx.stage.Stage stage) { openWindows.put(sample, stage); }
    public void unregisterWindow(String sample) { openWindows.remove(sample); }
    /** Returns the open Stage for this sample, or null if none. */
    public javafx.stage.Stage openWindowFor(String sample) {
        javafx.stage.Stage s = openWindows.get(sample);
        return (s != null && s.isShowing()) ? s : null;
    }
}
