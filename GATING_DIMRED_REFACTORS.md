# StreamFlow — Gating Window & Dim Reduction: Bug Checklist + Refactors

Reviewed as: Software Architect + Computational Biologist. Scope: `GraphWindowController` (+ `graph-window.fxml`, `CytoPlot`, `WorkspaceModel`) and `DimReductionController` (+ `dimreduction.fxml`, `EmbeddingRules`).

**Revision note:** an earlier draft of this document marked D1, D2, D3, D4, and G1 as "FIXED" in the working tree, with an "APPLIED PATCHES" section describing the changes. **All five of those claims were re-checked against the actual current code and found to be false** — none of the described patches exist in this repo. The D2 patch in particular was written and applied during a separate debugging chat session, against a different local working copy, and never merged here. Every status below is re-derived from file:line evidence in the current tree, not from the prior document's claims.

Legend: 🔴 critical (leak/correctness) · 🟠 high (silent wrong result) · 🟡 medium (UX/flow) · 🟢 feature/spec, not yet built.

---

## PRIORITIZED CHECKLIST

### Gating window (GraphWindowController)
- 🔴 **G1 — Tree-change listener leak. STILL OPEN.** `WorkspaceModel.java` (319 lines, read in full) has `addTreeChangeListener` (`:151`) but **no `removeTreeChangeListener` method at all** — zero matches repo-wide. `GraphWindowController.build()` (`:201`) registers `ctx.workspace().addTreeChangeListener(c::onExternalTreeChange)` as an inline method reference, not held in any field. `stage.setOnHidden(...)` (`:240-243`) removes the export-format listener and unregisters the window from `WorkspaceModel`'s open-window map, but does nothing for the tree listener — and *couldn't*, since there's no field holding it and no removal API exists. **Every graph window opened and closed permanently leaks its controller into `WorkspaceModel`'s listener list.** This is the single most impactful leak in the app (every double-click-to-open, including the golden-invariant path documented in AUDIT.md §9, creates one).
- 🟠 **G2 — Undo/Redo/Remove/SVG buttons have no field. STILL OPEN.** `graph-window.fxml` declares `fx:id="undoButton"`/`redoButton`/`removeGateButton`/`svgButton`, each wired only via `onAction` in FXML. `GraphWindowController.java`'s `@FXML private Button` declarations (lines 71-74) don't include any of the four. `doUndo()`/`doRedo()` (lines 1941-1957) never call `setDisable` on anything. Undo/Redo stay clickable with an empty stack (prints "Nothing to undo" instead of greying out).
- 🟡 **G3 — Axis-control freeze guard is a workaround, not a fix. STILL OPEN.** `setOnBusy` (`:319-330`) skips disabling combos when `axisPopoverActive`, to avoid a cancelled render stranding the disabled state — a documented workaround, not a generation-token fix. Confirmed no `renderGen`/`AtomicLong` staleness guard exists anywhere in `GraphWindowController.java` for its own async work: `loadFromEngine()` (`:559-596`) fires a `Task<JsonNode>` for `get_events` with no check that `sampleFile` still matches the sample requested when the task launched, and `gotoSample()` (`:537-543`) can be invoked repeatedly (rapid Next/Prev) with no generation guard — a slow, stale `get_events` response can land and overwrite the plot after the user has already navigated elsewhere. Note this is **distinct** from `CytoPlot`'s own `gen == renderGen` pattern (`CytoPlot.java:806-882`), which is real and correctly guards CytoPlot's *internal* raster jobs — it just doesn't cover GraphWindowController's own async sample-load path.

### Dim reduction (DimReductionController)
- 🔴 **D1 — Ghost embedding across dataset change. STILL OPEN** (previously claimed FIXED). `refreshFromWorkspace()` (`DimReductionController.java:360-392`) repopulates sample/channel combos but never clears `embedding`, gates, clusters, suggestions, or `markerThresholds`. No `clearEmbeddingState()` or `embeddingStale()` method exists anywhere in the file (or the repo — zero grep matches). Because nothing gates `restoreEmbedding` on the new dataset actually containing the embedded samples, a map from the *previous* dataset survives with gate row-indices pointing into events that no longer exist. `MainController.refreshModules()` (`:632-638`) does call `refreshFromWorkspace()` on every `Refreshable` including DimReduction after a workspace reset — but since that method does nothing to `embedding`, the stale plot/gates remain visible.
- 🔴 **D2 — Auto-threshold ignores the real transform. STILL OPEN** (previously claimed FIXED — this is the exact bug documented in the earlier "Debugging dimension reduction implementation" chat, whose patch never made it into this repo). `_positivity_thresholds` (`streamflow_engine.py:842-879`) hardcodes `cofactor = float(a.get("cofactor", 150.0))` (`:854`) and always computes `np.arcsinh(v / cofactor)` / inverts with `np.sinh(...) * cofactor` (`:870`) — it never calls `_default_transform()` or `_apply_matrix_transform()`, so it ignores logicle/log/none entirely. `onAutoThreshold()` (`DimReductionController.java:445-463`) and the inline Auto button inside `onMarkerThresholds()` (`:1090-1099`) both still send only `sample`+`channels` — no `transform`/`cofactor` args. Under logicle or a non-default cofactor, the Otsu cut lands on the wrong curve, so intensity-based ("Thresholds") cluster naming can disagree with z-score/median-based naming for the same cluster — this is very likely the "scientifically wrong" symptom originally reported.
- 🟠 **D3 — Naming defaults to Z-score regardless of thresholds. STILL OPEN** (previously claimed FIXED). `namingCombo.getSelectionModel().select(NAME_ZSCORE)` is set once, statically, in `initialize()` (`DimReductionController.java:184`). No other code path re-selects `NAME_THRESHOLD` after the user populates `markerThresholds` via the Auto/manual threshold UI — a user who sets thresholds still gets Z-score-based cluster naming with no indication their thresholds are unused.
- 🟠 **D4 — Fallback cluster names concatenate every positive marker. STILL OPEN** (previously claimed FIXED). `EmbeddingRules.nameCluster()` (`EmbeddingRules.java:87-105`) has no "Mixed:" prefix or "Ambiguous (N)" cap — the fallback path joins every positive marker verbatim (`String.join(" ", pos)`), so a cluster positive on 5 mutually-exclusive lineage markers is named e.g. `"CD3+ CD4+ CD8+ CD19+ CD56+"`, an impossible phenotype presented as if it were a confident call.
- 🟡 **D5 — UMAP `min_dist`/`n_neighbors` and t-SNE `perplexity` are hardcoded, no UI control. Confirmed still open**, unchanged from original: `perplexity=30` fixed in both the openTSNE (`streamflow_engine.py:606`) and sklearn (`:609`) branches; UMAP (`:613`) gets only `n_components`/`random_state`, so `n_neighbors`/`min_dist` fall to library defaults (15/0.1). Spec below (D5-fix) unchanged and still recommended.
- 🟢 **D6 — No probabilistic clustering / soft assignment. Confirmed still open.** `cluster_embedding`'s backend dict (`streamflow_engine.py:738`) has `flowsom/kmeans/phenograph/bayesian/flowgrid`, no `gmm`. `_bayesian()` (`:702-710`) is the closest thing — a Dirichlet-process BayesianGaussianMixture — but still returns hard labels via `bg.predict(Xc)` (`:708`), not soft/multi-membership output. Root cause of impossible multi-positives (D4) at the source remains unaddressed.
- 🟢 **D7 — densMAP / PaCMAP not offered. Confirmed still open.** One-line UMAP flag + near-drop-in library addition, spec unchanged.

### Cross-cutting
- 🟠 **X1 — Transform has two sources of truth. Confirmed still open, and actively harmful, not just inconsistent** — see AUDIT.md §4 for the exact clobbering mechanism (Dim Reduction's local `transformCombo` silently overwrites `STATE["transforms"]` via `applyGlobalTransform()` whenever Run is clicked). Consolidate into `WorkspaceModel.activeTransform()`.

---

## STATUS OF PRIOR "APPLIED PATCHES" SECTION — retracted

The prior document's "APPLIED PATCHES (already in tree)" section described a `WorkspaceModel.removeTreeChangeListener` addition (G1) and a `DimReductionController.clearEmbeddingState()`/`embeddingStale()` addition (D1). **Neither exists in this repository.** These were evidently real patches made during a debugging session against a separate local checkout that was never pushed/merged into `main`. Treat G1 and D1 as fully open work items — see the checklist above for exact evidence.

---

## SPEC PATCHES (recommended next — specs below remain valid, none implemented yet)

### G1-fix — add the missing removal API (blocks everything else in this section)
```java
// WorkspaceModel
private final List<Runnable> treeListeners = new ArrayList<>();
public void addTreeChangeListener(Runnable r) { treeListeners.add(r); }
public void removeTreeChangeListener(Runnable r) { treeListeners.remove(r); }
```
`GraphWindowController` must hold the listener as a field (a method-ref passed inline can't be removed later):
```java
private final Runnable treeChangeListener = this::onExternalTreeChange;
// in build(): ctx.workspace().addTreeChangeListener(c.treeChangeListener);
// in setOnHidden(...): ctx.workspace().removeTreeChangeListener(c.treeChangeListener);
```

### G2-fix — expose the four buttons, bind disable
```java
// GraphWindowController
@FXML private Button undoButton, redoButton, removeGateButton, svgButton;

// after gateTree wiring, and inside pushUndo/doUndo/doRedo:
private void updateUndoButtons() {
    undoButton.setDisable(undoStack.isEmpty());
    redoButton.setDisable(redoStack.isEmpty());
}
```
`ArrayDeque` isn't observable — cheapest fix is calling `updateUndoButtons()` at the end of every push/pop rather than building a synthetic property.

### G3-fix — render generation token (kills the freeze class, not one case)
```java
private final AtomicLong loadGen = new AtomicLong();
// gotoSample()/loadFromEngine(): long myGen = loadGen.incrementAndGet();
// in the Task's setOnSucceeded: if (myGen != loadGen.get()) return;  // stale response, drop it
```
Then `setOnBusy` can disable controls unconditionally and the `axisPopoverActive` special-case goes away — same idiom as `CytoPlot`'s existing (correct) `gen == renderGen`, just applied to `GraphWindowController`'s own sample-load path instead of CytoPlot's internal raster jobs.

### D1-fix — clear embedding state on dataset change
```java
// DimReductionController
private void clearEmbeddingState() {
    embedding = null;
    plot.clearGates();
    clusterList.getItems().clear();
    suggestionList.getItems().clear();
    markerThresholds.clear();
    // ... comparePanel/previewGrid/colour-by items, snapshot
}
private boolean embeddingStale() {
    if (embeddedSamples == null) return false;
    return !ctx.workspace().sampleNames().containsAll(embeddedSamples);
}
// in refreshFromWorkspace(): if (ctx.workspace().sampleNames().isEmpty() || embeddingStale()) clearEmbeddingState();
```

### D2-fix — make `positivity_thresholds` respect the real transform
```python
# _positivity_thresholds, streamflow_engine.py
xform = a.get("transform") or _default_transform()
t = _apply_matrix_transform(v, xform)          # instead of hardcoded np.arcsinh(v/150)
thr_t, actual = _threshold_value(t, method)
thr = _invert_matrix_transform(thr_t, xform)   # numeric inversion for logicle/log; closed-form for arcsinh/none
```
```java
// DimReductionController — onAutoThreshold() and the inline Auto button in onMarkerThresholds()
ObjectNode xf = JSON.createObjectNode();
xf.put("type", transformCombo.getValue());
xf.put("cofactor", parseOrDefault(cofactorField.getText(), 150.0));
args.set("transform", xf);
```

### D3-fix — auto-switch naming mode when thresholds are set
```java
// wherever markerThresholds is populated (Auto button success handler, manual entry commit):
if (!markerThresholds.isEmpty() && !NAME_THRESHOLD.equals(namingCombo.getValue()))
    namingCombo.getSelectionModel().select(NAME_THRESHOLD);
```

### D4-fix — cap impossible multi-positive names
```java
// EmbeddingRules.nameCluster(), fallback branch
if (pos.size() > 2) return "Ambiguous (" + pos.size() + ")";
if (pos.size() == 2) return "Mixed: " + String.join(" ", pos);
return pos.isEmpty() ? fallback : pos.get(0);
```

### D5-fix — link the hyperparameters (UI + engine) — spec unchanged from prior draft
FXML (shown only for the relevant method):
```xml
<Label text="n_neighbors:"/><Spinner fx:id="nNeighborsSpinner"/>
<Label text="min_dist:"/><TextField fx:id="minDistField" text="0.1"/>
<Label text="perplexity:"/><Spinner fx:id="perplexitySpinner"/>
```
```java
if ("umap".equals(methodCombo.getValue())) {
    args.put("n_neighbors", nNeighborsSpinner.getValue());
    args.put("min_dist", parseOrDefault(minDistField.getText(), 0.1));
} else {
    args.put("perplexity", perplexitySpinner.getValue());
}
```
```python
if method == "tsne":
    perp = min(int(a.get("perplexity", 30)), max(2, (X.shape[0]-1)//3))
    ...TSNE(perplexity=perp, ...)
else:
    umap.UMAP(n_neighbors=int(a.get("n_neighbors", 15)),
              min_dist=float(a.get("min_dist", 0.1)),
              densmap=bool(a.get("densmap", False)), random_state=seed)
```

### D6 — probabilistic clustering (soft assignment)
```python
elif method == "gmm":
    from sklearn.mixture import GaussianMixture
    g = GaussianMixture(n_components=k, covariance_type="full", random_state=seed).fit(Xc)
    proba = g.predict_proba(Xc)
    labels = proba.argmax(1)
    probs = proba.max(1)
```
Wire `probs` into the suggestion list confidence column; in `EmbeddingRules.nameCluster`, tag "low-confidence" when mean posterior < 0.6 instead of asserting a phenotype.

### D7 — densMAP / PaCMAP
densMAP: `densmap=True` UMAP flag exposed as a checkbox. PaCMAP: `pip install pacmap`, branch on `method == "pacmap"`.

---

## UI/UX & PUBLICATION PLOTTING (criterion 4) — unchanged, still valid recommendations

Plot defaults remain journal-grade (pseudocolor default, dot/contour/density/zebra/histogram, no stacked bars anywhere). Toolbar density on Dim Reduction is still high — group into `TitledPane`s (Run/Display/Analytics). Padding still missing on the root VBox. Cluster confidence still not visualized (ties to D6). Status-bar effective transform still needs to extend from Dim Reduction into the gating window title/status.

---

## COMPBIO NOTES — unchanged, still accurate

Pooled-equal-N embedding is the correct call. Naming/rules evaluate against raw medians vs raw thresholds — correct space, keep it (this part was never broken). FlowSOM SOM grid isn't scaled to N (D5/roadmap item, confirmed via direct code read this pass — no `xdim`/`ydim` passed anywhere). CD4/CD8 double-positive as a stable cluster remains a doublet/compensation flag worth auto-suggesting a gate for once D4 is fixed and such clusters stop being silently mislabeled instead of flagged.

---

## Golden invariant, cross-referenced from AUDIT.md §9

The double-click-anywhere-in-the-tree → open-in-Graph-Window behavior (`WorkstationController.java:94-101` and `GraphWindowController.java:465-470`, both routing through `openChild`/`navigateTo`) was verified in this pass to already work correctly at **any** nesting depth, with no hardcoded limit — see AUDIT.md §9 for the full trace. This is unrelated to G1's listener leak (which leaks a thread/listener per open, but doesn't affect *which* nodes are openable) — both are true simultaneously: every depth opens correctly, and every open leaks a listener until G1 is fixed. Fixing G1 must not regress the depth-agnostic behavior; the recommended regression test in AUDIT.md §9 should be added alongside the G1 fix's own verification.
