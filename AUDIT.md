# StreamFlow — Software-Level Audit

Scope: full `main` (commit e74de6f). 21k lines Java across 27 controllers + 2.9k-line Python engine (`engine/streamflow_engine.py`). Focus per request: lifecycle abruption, unlinked values/fields, ghost fields, tab interlinking, dim-reduction modernisation, workspace linkage, usability/UX.

**Revision note:** this document supersedes an earlier draft that marked several items below as "FIXED." Every claim in that draft was re-verified against the current working tree by direct file reads (not by trusting the prior document), and **7 of 8 previously-claimed fixes were found to not actually be in this repo** — the patches were evidently made in a different local working copy and never landed here. Statuses below reflect the actual code as of this revision. Treat any future "FIXED" label with the same skepticism: verify against file:line, not against a prior audit's word.

---

## 0. CI / test suite validates the wrong engine (new, highest priority)

`EnginePaths.usePython()` (`app/src/main/java/org/streamflow/bridge/EnginePaths.java:21-27`) defaults to the **R** engine unless `-Dstreamflow.python`, `-Dstreamflow.engine.kind=python`, or the `STREAMFLOW_PYTHON`/`STREAMFLOW_ENGINE` env vars are set. Both `app/run-dev.ps1:46-48` and `app/package.ps1:104` set these explicitly, so **the app as actually run (dev and packaged) always uses `engine/streamflow_engine.py`** — consistent with the Python migration being largely complete.

But every Java bridge test — `AnalysisCommandTest`, `CompensationCommandTest`, `GatingCommandTest`, `StatisticsCommandTest`, `TransformationCommandTest`, `VisualizationCommandTest`, `SetupCommandTest`, `PackageSmokeTest`, `ProtocolConformanceTest` — calls `System.setProperty("streamflow.engine", ...)` pointing at the **R** script, and CI (`.github/workflows/build-java.yml:85-86`) runs `mvn -B clean test -Dstreamflow.rscript=%RSCRIPT_PATH%` with no Python property set anywhere. Only `PyEngineTest.java` targets `streamflow_engine.py`, and it exercises exactly 3 of ~46 command handlers (`capabilities`, `load_fcs`, `get_events`).

**Consequence:** ~43 Python command handlers — `run_dimredux`, `cluster_embedding`, `positivity_thresholds`, `apply_compensation`, `run_classifier`, `run_cell_cycle`, `run_apoptosis`, `run_stats_comparison`, and every other analysis command — have **zero CI protection** for the code path that actually ships to users. Any future edit to `streamflow_engine.py` can silently regress with a fully green build.

**Fix:** parametrize the bridge tests to run against both engines (skip gracefully per engine, mirroring `ProtocolConformanceTest`'s existing R-availability skip), or add a Python-specific test class per command family. At minimum, add a Python-engine job to `build-java.yml`.

---

## 1. Ghost fields — result: clean

Cross-checked every `@FXML private <field>` in all 27 controllers against `fx:id` in the paired FXML. **No true ghost fields** — every injected field has a backing `fx:id`. Still true on re-check.

The reverse (an `fx:id` with no controller field) is clean except four buttons in `graph-window.fxml` — see §3.

---

## 2. Lifecycle wiring — the real weak points

### 2a. Silent refresh failures — STILL OPEN (correction: this document previously claimed it was fixed; that was wrong)
`refreshModules()` (`MainController.java:632-638`) is a **bare** `catch (Exception ignored) {}` around every `Refreshable.refreshFromWorkspace()` call, with zero logging or toast:
```java
private void refreshModules() {
    for (Object c : controllers.values()) {
        if (c instanceof Refreshable r) {
            try { r.refreshFromWorkspace(); } catch (Exception ignored) {}
        }
    }
}
```
Any module that throws during refresh goes permanently stale with no visible symptom. This matters more than a routine silent-catch: it's the exact method that P1/P3's `refreshFromWorkspace()` fixes elsewhere in this codebase route through, so a bug introduced by any of those fixes (e.g. a null-pointer in a new `clearEmbeddingState()`) would silently no-op forever during manual QA. Recommend fixing this alongside/before the P1/P3 work: add an slf4j `Logger`, log the failing controller class + exception, and surface a toast.

### 2b. ContextAware-but-not-Refreshable tabs
`AnalysisLog`, `DevConsole`, `Setup`, `Workstation` receive `ctx` but never get `refreshFromWorkspace`. Verified each wires its own listener as a substitute:
- `WorkstationController.init()` (`:116-122`) wires `sampleNames()`/tree/data listeners directly — correct substitute.
- `SetupController` relies on `MainController` calling `populate()` directly on load/import/open (`MainController.java:491,542,609`) — **but `onNewWorkspace`'s `purgeWorkspaceState()` (`MainController.java:619-629`) never calls `setupController.populate(...)`**, so Setup's `samples`/`channels` go stale on "New Workspace." Low practical impact since Setup is loaded but not in the visible nav list (`MainController.java:117-122`).

### 2c. Dialog controllers (legitimately not ContextAware)
`CompWizard`, `CopySettings`, `FcsDiagnostics`, `GraphWindow`, `Settings` get context by constructor/opener. `GraphWindow` is the one that matters — it's the main plotting surface, not a small dialog, and its undo/redo buttons can't be disabled as a direct consequence (§3).

---

## 3. graph-window.fxml — undo/redo/remove/svg buttons half-wired

**Confirmed still open** (never claimed fixed). `undoButton`, `redoButton`, `removeGateButton`, `svgButton` are wired by `onAction` in FXML but have **no `@FXML` field** in `GraphWindowController.java` (its `@FXML private Button` declarations, lines 71-74, list only `xAxisOptsButton, yAxisOptsButton, prevSampleButton, nextSampleButton, copyButton, copySettingsButton, channelsButton, fmoButton, compareButton`). `doUndo()`/`doRedo()` (lines 1941-1957) never touch a button's `disableProperty`. Undo stays clickable with an empty stack.

---

## 4. THE BIG ONE — transform linkage between tabs is broken by design

**Confirmed still broken**, and worse than previously described: Dim Reduction doesn't just disagree with the Transformation tab, it actively **overwrites** it.

1. **`apply_transformation` still never transforms the stored events** — it records the transform into `STATE["transforms"]`; every analysis re-derives from raw each time via `_default_transform()` (`streamflow_engine.py:776-789`).
2. **`DimReductionController.onRun()` always sends its own local transform** (`transformCombo`/`cofactorField`, lines 516-519) into `run_dimredux`'s args. On the Python side, `_run_dimredux` does `xform = a.get("transform") or _default_transform()` (`streamflow_engine.py:533`) — since Java always sends a non-empty object, `_default_transform()` is dead code whenever Dim Reduction runs.
3. **Dim Reduction's local combo then clobbers the global setting**: `applyGlobalTransform()` (`DimReductionController.java:1150-1159`) pushes that same local (never re-synced) combo value back into `STATE["transforms"]` via `apply_transformation`. `transformCombo`'s default ("logicle") is set once in `initialize()` (`:204-205`) and never re-read from the current global state. So: pick logicle-cofactor-5 in the Transformation tab, open Dim Reduction (still showing its own stale default), click Run — the global transform is silently overwritten back to Dim Reduction's default, with no warning either direction.
4. **Transform is still not in `WorkspaceModel`** — confirmed no `activeTransform()`/`setActiveTransform()` exists (repo-wide grep, zero matches).

### Fix (structural)
`WorkspaceModel.activeTransform()` / `setActiveTransform(type, cofactor)` as the single observable source. Transformation tab writes it; DimReduction/Clustering/Scatter3D **read** it (drop local combos, or bind read-only labeled "inherited from Transformation tab"). Engine keeps `_default_transform()` as the CLI/headless fallback only.

---

## 5. Dim-reduction: modernisation + probabilistic gating (roadmap unchanged, still valid)

Current: t-SNE (openTSNE/sklearn) + UMAP; clustering FlowSOM/k-means/phenograph/bayesian/flowgrid; naming by threshold/z-score/otsu/median.

- **densMAP** — a `densmap=True` UMAP flag, near-free.
- **PaCMAP / TriMap** — better global structure for lineage trees.
- **Expose UMAP `min_dist`/`n_neighbors`, t-SNE `perplexity`** — confirmed still fully hardcoded (`streamflow_engine.py:606,609,613`: `perplexity=30` in both openTSNE and sklearn branches; UMAP gets only `n_components`/`random_state`, no `n_neighbors`/`min_dist` at all — falls to library defaults 15/0.1). No UI control exists anywhere.
- **GMM soft clustering** — confirmed still absent. `cluster_embedding`'s backend dict (`streamflow_engine.py:738`) has `flowsom/kmeans/phenograph/bayesian/flowgrid`, no `gmm`. The closest, `_bayesian()` (lines 702-710), is a Dirichlet-process BGM but still returns **hard** labels via `.predict()` plus a max-probability confidence — not true soft/multi-membership output.
- **Seed-stability (ARI across N seeds)** — still not present.
- **FlowSOM grid scaled to N** — confirmed absent. Neither `_flowsom()` in `cluster_embedding` (:685-692) nor the FlowSOM branch of `run_clustering` (:919-927) pass `xdim`/`ydim`; no events-per-node check exists anywhere in the file.

---

## 6. Tab interlinking suggestions (currently siloed) — unchanged from prior pass, still valid

DimReduction → Workstation/Gating population creation, DimReduction → Statistics cluster-frequency export, Transformation → everything (§4, highest value), Compensation → DimReduction status visibility, Clustering ↔ DimReduction shared k/method/seed — none of these links exist yet; recommendations stand.

---

## 7. Silent-failure sweep (engine + Java) — corrected

Two items in the original draft claimed engine fixes that are **not actually present**:

- ❌ *"Fixed earlier: `cluster_embedding` over-broad `except Exception`..."* — **still open.** `cluster_embedding` (:744-749) and `run_clustering` (:928-932) both still catch bare `except Exception` and always report `"<method> unavailable (<ExceptionType>) — using FlowSOM/k-means"`, regardless of whether the cause was a genuine `ImportError` or a real crash on malformed input (NaN, singular covariance). This mislabels real bugs as missing packages.
- ❌ *"Fixed earlier: `positivity_thresholds` hard-coded arcsinh"* — **still open**, identical to the original bug: `_positivity_thresholds` (:842-879) hardcodes `cofactor=150.0` (:854) and always does arcsinh/sinh (:870), never calling `_default_transform()`. The Java "Auto" button (`DimReductionController.java:445-463`, and the inline Auto button in `onMarkerThresholds()` :1090-1099) still sends only `sample`+`channels`, no transform. A patch for this was written and applied in a separate debugging session's working copy but never merged into this repo.
- ❌ *"Fixed earlier: missing NaN/Inf guard before UMAP/t-SNE"* — **still open.** No finite-value filtering exists before `run_dimredux`'s UMAP/t-SNE calls or `cluster_embedding`'s clustering calls, in contrast to `run_cell_cycle`/`run_proliferation`, which both correctly do `vals[np.isfinite(vals)]` plus a minimum-count check. The "good pattern" exists in the file but isn't applied uniformly.
- ❌ *"Fixed earlier: silent openTSNE↔sklearn swap"* — **partially true, re-verified:** the openTSNE→sklearn fallback (`streamflow_engine.py:603-609`) is correctly scoped to `ImportError` only (not a bare except) and IS the "good" pattern — but the response never reports *which* backend actually ran (no `actual`-style field, unlike `cluster_embedding`'s clustering-method `actual`), so a user can't tell openTSNE from sklearn TSNE in the result even though they render slightly differently. This is a transparency gap, not a mislabeling bug — downgrading from "silent swap" to "un-reported swap."

Newly found in this pass (not in the original draft at all):
- 🔴 `render_plot` (:375) and `compute_stats` (:450) both hardcode `source="raw"`, ignoring `STATE["comp_applied"]` — Visualization tab silently shows uncompensated data after Apply Compensation. (`compute_stats` has no Java call site — dead code, so latent only.)
- 🔴 `apply_compensation` (:1136-1139) — per-sample failures are logged as a warning string but the command still returns success; `CompensationController.onApply()` never checks the `applied` count, so a partially-failed compensation run reports "Compensation applied to N channels" (channel count, not success count) even when samples silently failed.
- 🔴 `compute_spillover_from_controls` (:1454-1477) — a failed regression fit for an off-diagonal coefficient is silently replaced with `0.0` with `ok=True` hardcoded; the matrix looks complete but under-compensates that pair with nothing surfaced in `reports`.
- 🟠 `run_apoptosis` (:2313-2343) never transforms annexin/PI channels before valley-finding on a 256-bin linear histogram, and NaN events silently drop from quadrant counts while remaining in the denominator (percentages won't sum to 100%).
- 25 `catch (Exception ignored)` in Java — reconfirmed mostly benign temp-file cleanup; the two with real blast radius (`refreshModules`, `StatisticsController` per-sample) were the ones already addressed, but see FULL_APP_AUDIT.md §Part 2 for newly-found stale-cache issues in Classifier/Clustering/CrossSample/StatComparison/Statistics/Plugins that are a related but distinct problem (never cleared, not silently caught).

---

## 8. General usability / UX — corrected

- **Busy state on long embeds** — still just a job chip; engine sends `send_progress` fractions but the Java side doesn't render a determinate bar.
- **Perplexity/min_dist/k hidden defaults** — confirmed unchanged, see §5.
- **Seed fixed at 42 in `run_clustering`** but threaded as an arg in `cluster_embedding` — confirmed still inconsistent (`streamflow_engine.py:923,931,935` vs. the `seed` param `cluster_embedding` accepts).
- **Transform disagreement is invisible** — confirmed, and now understood to be an active clobber, not passive disagreement (§4).
- ❌ *"Undo/redo not disabled when empty"* — was never claimed fixed; still open, see §3.
- **"Auto" threshold vs naming-mode coupling** — still open; naming mode is hardcoded to Z-score at `DimReductionController.java:184` and never switches to Thresholds even after the user sets `markerThresholds` (see GATING_DIMRED_REFACTORS.md D3).
- **FlowSOM grid not scaled to N** — confirmed still absent, see §5.

---

## 9. Golden invariant — verified correct, document as protected

**Double-click on a population/gate node, at ANY nesting depth (root's direct child through the nth-level descendant), must open that population in a Graph Window.** This was checked end-to-end by reading the actual recursive data structures and both UI entry points — it already works correctly today, with no depth limit anywhere in the code path. This section exists so future edits don't accidentally introduce one (e.g. by replacing recursion with a fixed-depth loop, or a path-based lookup capped at N levels).

**Why it's depth-safe by construction:**
- `PopNode` (`PopNode.java:13-76`) is a plain `parent`/`children` linked structure with no depth field or depth cap anywhere.
- `PopNode.selfAndDescendants()` (`:54-62`) and `PopNode.chain()` (`:47-51`) both recurse/walk unconditionally through `children`/`parent` — arbitrary depth by definition.
- `WorkstationController.buildPop()` (`:186-191`) and `GraphWindowController.buildItem()` (`:971-974`) both build their respective `TreeItem<...>` hierarchies by recursing through `n.children` with no depth guard — every level renders and is clickable.

**The two entry points, both generic over depth:**
1. **Workstation main tree** → `tree.setOnMouseClicked` (`WorkstationController.java:94-101`) treats any `PopNode` uniformly regardless of depth (`it.getValue() instanceof PopNode n`) → `openSample(sampleOf(it), n.isRoot() ? null : n)` (`:316-328`) → for non-root, `GraphWindowController.openChild(ctx, sample, focus)`.
2. **A Graph Window's own gate tree** (showing the descendant hierarchy of the currently open population) → `gateTree.setOnMouseClicked` (`GraphWindowController.java:465-470`) → `openChildForNode(it.getValue())` (`:1015-1020`) → same `openChild(...)`, which either reuses an already-open window via `navigateTo(focus)` (`:631-635`, itself depth-agnostic via `isInTree()` → `selfAndDescendants().contains(n)`) or opens a fresh one with `initialFocus = focus`.

**Regression guard for future changes:** if any of the four methods above (`selfAndDescendants`, `buildPop`/`buildItem`, `openSample`/`openChildForNode`, `openChild`/`navigateTo`) are ever rewritten to use indexed/path-based lookup instead of direct object references, re-verify this invariant against a gating tree at least 4-5 levels deep (e.g. Lymphocytes → CD3+ → CD4+ → CD25+ → FoxP3+) before merging. No automated test currently covers this end-to-end — consider adding one alongside the existing `EmbeddingRulesTest`/`CoordinateMapperTest` style unit tests, since it's a pure-Java structural property (build a synthetic multi-level `PopNode` tree, assert `selfAndDescendants()` returns all levels and `openChild` is reachable for the deepest node) that doesn't require a JavaFX toolkit or the engine.
