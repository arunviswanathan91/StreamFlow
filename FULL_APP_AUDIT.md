# StreamFlow — Full-App Deep Audit (all tabs, plotting, analytics, future directions)

Reviewed as Software Architect + Computational Biologist across all 27 controllers, `CytoPlot`, and the ~46-command Python engine. Builds on the gating/dim-reduction pass (`GATING_DIMRED_REFACTORS.md`) and the top-level audit (`AUDIT.md`).

Priority: 🔴 leak/correctness · 🟠 silent-wrong-result · 🟡 UX · 🟢 future/feature.

**Revision note:** an earlier draft of this document claimed four items were "FIXED THIS ROUND" (CytoPlot render-thread accumulation, Setup experiment name, GraphWindow tree-listener leak, DimReduction ghost embedding). **All four were re-verified against the current code and found to still be open** — see the corrected section immediately below. The expanded Part 2 (other controllers) and Part 3 (engine commands) sections are new in this revision, produced by reading every remaining controller and the full engine file rather than sampling.

---

## CORRECTED: none of the "APPLIED THIS ROUND" items are actually in the tree

- 🔴 **CytoPlot render-thread accumulation — STILL OPEN.** `CytoPlot.java` (2761 lines, read in full) has **no `dispose()`/`shutdown()`/`close()` method at all** (repo-wide grep for `dispose|shutdown` in that file matches nothing; the only `dispose()` call anywhere in `ui/` is an unrelated `ImageWriter.dispose()` in `DimReductionController.java:2284`). Every `CytoPlot` instance owns a private single-thread `renderExec` (`CytoPlot.java:153-155`) that is never stopped by anything — this is broader than "some remaining discard sites," it's **every** discarded `CytoPlot` in the app. See Part 2 for the full list of discard sites found this pass.
- 🟠 **Setup: experiment name — STILL an unlinked field.** `experimentField` (`SetupController.java:44`) is set from the engine in `populate()` (`:150`) and disabled/enabled (`:346`), but has no `setOnAction`/focus listener and never calls `set_experiment_name` (which exists in the engine, `streamflow_engine.py:173`). Renaming an experiment and pressing Enter/clicking away still does nothing.
- 🔴 **GraphWindow tree-listener leak — STILL OPEN.** See AUDIT.md §0/GATING_DIMRED_REFACTORS.md G1 for full detail: no `removeTreeChangeListener` exists on `WorkspaceModel`, and the listener is registered as an unremovable inline method reference.
- 🔴 **DimReduction ghost embedding across dataset change — STILL OPEN.** See GATING_DIMRED_REFACTORS.md D1.

---

## LIFECYCLE — verdict per tab (re-verified)

The ~20 analysis tabs are singletons loaded once by `MainController.loadModule`, so unremoved workspace/settings listeners registered in `init()` are correct (they must live for the app). Real teardown risk exists for **spawned windows**: `GraphWindow` (still leaking, above), and the transient dialogs `CompWizard`/`CopySettings`/`FcsDiagnostics`/`Settings`/`PositivityDialog`/`AddStatisticDialog`. Of these, `CompWizardController` and `PositivityDialog` were newly found this pass to instantiate a throwaway `CytoPlot` per open with no cleanup (Part 2).

- 🟡 **Workstation/AnalysisLog/DevConsole/Setup are `ContextAware` but not `Refreshable`.** Re-verified: Workstation and AnalysisLog/DevConsole correctly self-wire listeners as a substitute. Setup does not fully — see AUDIT.md §2b (stale on "New Workspace," low practical impact since Setup isn't in the nav list).

---

## PART 2 — Expanded controller audit (new this revision)

Every controller not previously covered in depth: `ApoptosisController`, `CellCycleController`, `ClassifierController`, `ClusteringController`, `CompWizardController`, `CompensationController`, `CopySettingsController`, `CrossSampleController`, `DevConsoleController`, `ExportController`, `FcsDiagnosticsController`, `KineticController`, `LongitudinalController`, `PluginsController`, `PositivityDialog`, `ProliferationController`, `Scatter3DController`, `SettingsController`, `SetupController`, `StatComparisonController`, `StatisticsController`, `TransformationController`, `VisualizationController`, `WorkstationController`, `AnalysisLogController`, `CompMatrixView`, `AddStatisticDialog`.

**Foundational fact underlying most 🔴 findings below:** since `CytoPlot` has no `dispose()` at all (confirmed above), *any* `new CytoPlot()` that gets discarded leaks one daemon render thread — not just the sites a prior draft called "remaining."

### 🔴 WorkstationController.java
`refreshOverview()` (`:363`) builds one new `CytoPlot` mini per sample on every Overview-tab visit or manual refresh click (`:125-127`, `:332-333`), discarding the previous batch via `overviewGrid.getChildren().clear()` (`:347`) with no thread cleanup. A user with 20 samples clicking between Overview and another tab 10× over a session leaks 200 render threads.

### 🔴 ExportController.java
- `makeStepCell` (`:565`), called from `renderTiles` (Layout Editor, `:347-368`) and the Gating-Strategy canvas rebuild (`:498-535`) — both rebuild fresh `CytoPlot`s per population tile on every regenerate/Cols-spinner tick (`:265`), discarding the old ones.
- 🟠 `refreshFromWorkspace` (`:206-258`) never clears the Data-Export `rows`/`dataTable` from a prior `onCompute()` run, and `copyTsvButton`/`saveCsvButton` stay enabled — a user can copy/save a CSV describing a workspace that's no longer loaded.

### 🔴 PositivityDialog.java
`plot = new CytoPlot()` (`:99`) inside the static `show(...)` dialog is never disposed when the dialog closes (OK or Cancel both just call `stage.close()`/`dlg` dismissal). Every open of this dialog (from DimReduction's live/dead threshold picker) leaks one render thread.

### 🔴 CompWizardController.java
`scatterPlot` (`@FXML CytoPlot`), instantiated fresh on every `open(...)` call (`:105-126`). `onClose()`/`onUse()` (`:439-446`) only call `stage.close()` — no hook stops the plot's render thread. N wizard opens over a session = N leaked threads.

### 🔴 CompensationController.java
`refreshFromWorkspace()` (`:108`) only calls `refreshSamples()` — never clears `currentChannels`/`rows`/the `CompMatrixView` heatmap. Concrete failure: open a workspace with a *different* panel (different channel set) than the one currently displayed, then click **Apply** without re-running Extract/Compute — `matrixArgs()` (`:145-156`) builds `apply_compensation` args from the stale `currentChannels`/`rows` of the *previous* panel and ships them against the new dataset, silently applying a wrong-channel spillover matrix.

### 🟠 ClassifierController.java
`lastResult` (`:99`, set in `showResult()` `:271-295`) is never cleared by `refreshFromWorkspace()` (`:138-142`, which only calls `rebuildSamples()`/`refreshHints()`). File ▸ New/Open Workspace leaves the old PCA biplot + loadings table rendered, Copy/Export still enabled, for samples no longer in the workspace.

### 🟠 ClusteringController.java
`plotChannels`/`plotMedians`/`plotK` (`:56-58`, set in `showResult()` `:127-151`) never cleared by `refreshFromWorkspace()` (`:87-88`, only `refreshSamples()`+`refreshHints()`). Same stale-render pattern as Classifier.

### 🟠 CrossSampleController.java
`refreshFromWorkspace()` (`:138-142`) only calls `syncSampleSelList()`/`refreshHints()`; `sampleList`/`gateList`/`matrixRows`/`consRows`/`driftRows` and the radar canvas (populated in `computeViews()` `:196-235`) are left stale after a reset — `clearViews()` (`:159-166`) exists but is only called from `compute()`'s own empty-selection paths, never from the refresh hook.

### 🟠 StatComparisonController.java
`refresh()` (`:128-144`) rebuilds `gateCombo` and recomputes per-sample frequencies for the still-selected gate, but never clears `plotNames`/`plotValues`/`plotP`/`plotStars`/`plotPosthoc` (`showResult()` `:212-243`). The box-plot + significance brackets from the last test on the *old* dataset stay on screen after a reload.

### 🟠 StatisticsController.java
- `refreshPopulations()` (`:103-112`, the `refreshFromWorkspace` hook) only repopulates `popList` — never clears `rows`/`channels` from a previous `onCompute()`, and `exportButton` is never disabled. After File ▸ New/Open Workspace, clicking **Export statistics** without re-clicking Compute silently writes a CSV of the *previous* workspace's data.
- `computeNext`/`computeSample` (`:168`) and `ensureData` (`:319`) both `catch (Exception ignored) {}` — a malformed gate chain silently drops that sample's row with no status message; a failed `EventData.read` silently still calls `onReady.run()`, producing a shorter-than-expected table with no visible cause.

### 🟠 PluginsController.java
`refreshFromWorkspace()` (`:73-79`) repopulates `sampleCombo` and re-selects a same-named sample if one exists, but never resets `lastKeptIndices` (`:49`) or disables `addPopButton`. Concrete failure: run a plugin on sample "A", get `kept_indices` + an enabled "Add as population," then load a *different* workspace that also has a sample named "A" (e.g. reload the same folder) — `sampleCombo` reselects "A," and **Add as population** (`:202-209`) applies the *old* run's row indices to the *new* sample's data, producing a population with wrong/out-of-range membership. This is the highest-risk stale-cache finding in this section (silent data corruption, not just a stale plot).

### 🟢 Clean (verified, no findings)
`ApoptosisController`, `CellCycleController`, `ProliferationController` — engine args fully wired; `showResult` always replaces `currentData`/gates together, no stale-cache path. `Scatter3DController` — no persisted cross-run cache. `KineticController` — registers its `WorkspaceModel` listener once in `init()` as a singleton tab (correct, not a leak); all UI controls reach the engine call. `LongitudinalController` — per-row subject/timepoint state is intentionally sticky, recomputes live from the tree each Plot. `TransformationController`/`VisualizationController` — stateless request/response. `DevConsoleController` — trivial diagnostics, no state. `AnalysisLogController` — reads the shared `AuditLog`'s live `ObservableList` directly, nothing to go stale. `FcsDiagnosticsController` — self-contained per-open dialog, no `CytoPlot`, no shared-model listeners. `CopySettingsController`/`SettingsController` — listeners live on local dialog controls writing *into* `AppSettings`, never subscribed *from* it, so no leak on repeated open/close. `CompMatrixView` — embedded once inside singleton parents (not repeatedly instantiated/discarded); `setMatrix()` correctly clears stale `flagged` state. `AddStatisticDialog` — pure static modal on a local working copy, nothing persisted.

**Recommended fix pattern (applies to every 🟠 stale-cache finding above):** the DimReduction-style `clearEmbeddingState()` idiom (once actually implemented per GATING_DIMRED_REFACTORS.md D1-fix) should become the shared pattern for every `Refreshable.refreshFromWorkspace()` — null cached result fields and disable Export/Copy before the next recompute, not just repopulate combos. `PluginsController.lastKeptIndices` should be prioritized given its data-corruption (not just staleness) failure mode.

---

## PART 3 — Expanded Python engine audit (new this revision)

`engine/streamflow_engine.py`, read in full (~2858 lines), ~46 command handlers. Cross-checked reachability against `app/src/main/java` call sites and test coverage against `app/src/test/java/org/streamflow/bridge/*Test.java`.

**Meta-finding (see also AUDIT.md §0):** every bridge test targets the **R** engine (`System.setProperty("streamflow.engine", ...R script...)`), and CI never sets the Python-selection property either. Only `PyEngineTest.java` exercises `streamflow_engine.py`, covering just `capabilities`/`load_fcs`/`get_events`. **Essentially the entire command surface below — ~43 handlers — has zero automated coverage of the code that actually ships.**

### 🔴 Correctness
- `render_plot` (`:375`) and `compute_stats` (`:450`) hardcode `source="raw"`, ignoring `STATE["comp_applied"]` (`compute_stats` has no Java call site — dead code, latent only; `render_plot` is live via `VisualizationController.onRender()` `:89`).
- `apply_compensation` (`:1136-1139`) — per-sample exceptions are logged as a warning string but the command still reports success; `CompensationController.onApply()` (`:135-141`) never checks the `applied` count.
- `compute_spillover_from_controls` (`:1454-1477`) — a failed `curve_fit`/`HuberRegressor` fit for an off-diagonal coefficient is silently replaced with `0.0`, with `ok=True` hardcoded (`:1456`) regardless of fit quality — no per-coefficient failure surfaced in `reports`.

### 🟠 Silent-wrong-science
- `cluster_embedding` (`:744-749`) / `run_clustering` (`:928-932`) — bare `except Exception`, always phrased as `"<method> unavailable (<ExceptionType>) — using FlowSOM/k-means"` regardless of whether the cause was a missing package or a real crash on malformed input (NaN, singular covariance). Opposite of the file's own good pattern in `_threshold_value`.
- FlowSOM grid size never scales with N — `_flowsom()` (`:685-692`) and `run_clustering`'s FlowSOM branch (`:919-927`) pass only `n_clusters`, never `xdim`/`ydim`.
- `positivity_thresholds` (`:854,870`) — same hardcoded-arcsinh(150) bug as GATING_DIMRED_REFACTORS.md D2; confirmed this handler was never touched by any prior patch attempt.
- `run_apoptosis` (`:2313-2343`) — no transform applied to annexin/PI channels before valley-finding on a 256-bin **linear** histogram; NaN events silently drop from the 4 quadrant counts while remaining in the `n` denominator, so percentages don't sum to 100%.
- `run_classifier` (`:2530-2536`) — `RandomForestClassifier(n_estimators=200, random_state=0)` fit on **all** samples, no train/test split or CV, `oob_score` left `False`; only impurity-based `feature_importances_` returned. Trivially overfits with typical few-sample cytometry cohorts; no minimum per-group sample count enforced.
- Logicle/log transform params hardcoded in `_apply_matrix_transform` (`:807-808`) and `apply_transformation` (`:1555-1560`): `param_t=262144, param_w=0.5, param_m=4.5, param_a=0` fixed for every channel/instrument, no per-channel `$PnR`-based scaling or override.

### 🟡 Missing guard / hardcoded hyperparameter with no UI passthrough
- `run_clustering` (`:923,931,935`) hardcodes `seed=42`, unlike `cluster_embedding` which threads a `seed` arg — reproducibility inconsistency.
- `cluster_embedding`'s `_flowgrid` (`:729-730`, `MinDenB=3, MinDenC=40` fixed), `_phenograph` (`:696`, no `k`/`n_neighbors` exposed), `_bayesian` (`:704-707`, `max_iter=200, reg_covar=1e-4` fixed) — all internal hyperparameters with no `a.get(...)` override.
- `run_apoptosis`'s `_valley_threshold` (`:2317`) hardcodes `bins=256` with no override — unlike `run_cell_cycle`/`run_proliferation`, which both expose `bins`.
- `run_dimredux` tSNE (`:603-609`) — `perplexity=30` fixed; the openTSNE→sklearn fallback on `ImportError` is correctly scoped (unlike the bare-`Exception` cases above) but the response never reports which backend actually ran — a real transparency gap since the two TSNE implementations differ numerically.
- `_sample_by_name` (`:346-347`) — bare `except: pass` around lazy compensation caching; a stale/malformed comp matrix silently leaves just that one sample uncompensated with zero diagnostic.
- `import_flowjo_wsp` (`:1725-1727, :1743`) — swallows per-sample `analyze_samples()` and per-gate membership failures via bare `except Exception`, silently returning `[]` gates with no progress message at all.
- `run_kinetic` (`:2439-2440`) — `except Exception: continue` silently drops any sample that fails to load, with no count of skipped samples returned.
- No finite-value guard before UMAP/TSNE/FlowSOM in `run_dimredux`/`cluster_embedding`, in contrast to `run_cell_cycle`/`run_proliferation`, which both explicitly filter non-finite values plus a minimum-count check.
- `run_stats_comparison` — no NaN/Inf filtering of group values before `scipy.stats` calls; a NaN input silently yields `p=nan` and `_stars(nan)` evaluates to `"ns"` rather than surfacing bad input.

### 🟢 Confirmed solid / minor / test-gap
- `run_cell_cycle`'s Watson model genuinely constrains `mu2=2·mu1, sigma2=2·sigma1` **inside** the `curve_fit` objective (`:1962-1968`), not just a post-hoc label.
- `run_proliferation`'s log10-spaced generation delta (`:2081-2107`) is mathematically correct (seeded/bounded around `log10(2)`).
- `run_stats_comparison` genuinely calls `scipy.stats.mannwhitneyu`/`wilcoxon`/`kruskal`; `_dunn_posthoc` (`:2169-2211`) is a real rank-based Dunn's test with tie correction + Bonferroni, not a placeholder.
- `_threshold_value` (ImportError-only fallback) and `positivity_thresholds`'s per-channel try/except (labels `ok=False` + exception type) remain the two clean examples of the "good" error-labeling pattern — worth replicating into `cluster_embedding`/`run_clustering`.
- `run_classifier`'s stale error text ("Need ≥ 2 features and ≥ 3 samples for PCA") over-states the actual guard condition (`n_comp<1`) — cosmetic, low priority.
- **Zero Python-engine test coverage** (confirmed, not just a name-grep artifact) for: `bead_calibration`, `cluster_embedding`, `comp_preview`, `comp_residual`, `compute_spillover_from_controls`, `compute_stats`, `dimredux_channels`, `export_fcs`, `get_metadata`, `get_state`, `import_flowjo_wsp`, `list_channels`, `list_fluor_channels`, `load_wsp`, `positivity_thresholds`, `run_apoptosis`, `run_cell_cycle`, `run_classifier`, `run_clustering`, `run_dimredux`, `run_kinetic`, `run_proliferation`, `run_stats_comparison`, `sample_path`, `save_gatingml`, `save_wsp`, `save_workspace`, `load_workspace`, `set_experiment_name`, `subsample`, `suggest_transform`, `apply_compensation`, `apply_transformation`, `extract_spillover`, `compute_spillover`, `render_plot`, `reset`.

---

## GOLDEN INVARIANT (verified this pass — see AUDIT.md §9 for full detail)

Double-click on any population/gate node, **at any nesting depth**, opens/focuses a Graph Window on that population. Traced end-to-end through `PopNode`'s unconditional `parent`/`children` recursion, `WorkstationController`'s and `GraphWindowController`'s own gate-tree double-click handlers, and `GraphWindowController.openChild`/`navigateTo` — none of it depends on depth. **This already works correctly today and must not be broken by future refactors** (in particular, the G1 fix for the tree-listener leak touches the same window-open path — verify against a 4-5-level-deep gating tree before merging that fix). See AUDIT.md §9 for exact file:line citations and a recommended regression test.

---

## FUTURE DIRECTIONS (beyond bug-fixing) — unchanged, still valid

Spatial/GeoMx bridge, batch-effect diagnostics before pooling, reproducibility manifest (transform/seed/method/hyperparameters/event counts/library versions — the engine already threads `actual`/`transform`/`seed` in some handlers, not all, per Part 3 above), probabilistic gating end-to-end (density-contour + purity score), headless/CLI mode reusing the same 46 engine commands.
