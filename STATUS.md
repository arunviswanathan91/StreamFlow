# StreamFLOW — Project Status

**Verified against the working tree on 2026-08-26.** Build gate: `mvn test` →
**67 tests, 0 failures, 0 errors, 0 skipped**. Packaged app-image built and its bundled engine
verified end-to-end (see *Packaging* below).

> **Rule for anyone editing this file — including future AI sessions.**
> Every status claim here must cite `file:line` evidence read from the current tree.
> This document replaced three earlier audit docs (`AUDIT.md`, `FULL_APP_AUDIT.md`,
> `GATING_DIMRED_REFACTORS.md`) that were deleted because they marked ~20 already-fixed
> items as "STILL OPEN" and caused fixed bugs to be re-investigated. Do not add a status
> without verifying it. Do not trust a prior document's word — including this one's — over
> a direct read.

---

## Architecture

Native **JavaFX** desktop app (`app/`, Java 21 target, Maven) ⇄ headless **Python** engine
(`engine/streamflow_engine.py`, FlowKit/NumPy/SciPy/scikit-learn) over a JSON line protocol,
with bulk events shipped as little-endian `float32` blobs. 45 engine commands. A legacy R engine
is retained for a subset of tests and for R-based plugins.

Packaging is **jpackage** (`app/package.ps1`) producing a self-contained app-image or `.exe`
installer that bundles the JRE, engine scripts and a portable Python — no user-installed Java or
Python required. `.github/workflows/release.yml` builds and publishes it on a `v*` tag.

---

## Packaging — verified working

A full local app-image build now succeeds and the bundled engine has been exercised end-to-end:
bundled Python 3.11.9 starts `content/engine/streamflow_engine.py`, reports
`flowkit/numpy/sklearn/umap` all available, and loads a real 11-channel FCS file.

```powershell
$env:JAVA_HOME = "C:\Program Files\Java\jdk-21.0.10"     # any JDK 21+ with jpackage
powershell -File engine\stage-python.ps1                 # when Python deps change
powershell -File app\package.ps1 -SkipR -Version 1.2.0   # app-image (no WiX needed)
```

Two ship-blockers were found and fixed during that first real validation run — both would have
produced an installer with a **dead engine**:

| Fixed | Detail |
|---|---|
| `appDir` pointed at a directory that never exists | jpackage places `--app-content` at the **image root**, but `$APPDIR` is `<image>\app`. The launcher passed `-Dstreamflow.appDir=$APPDIR\content`, so the packaged app looked for its engine in `<image>\app\content`. Now `$APPDIR\..\content`, with `RPaths.appDir()` normalising the path (`app/package.ps1`, `RPaths.java:27-34`) |
| `stage-python.ps1` reported success after installing nothing | pip failed on every package (TLS), yet the script printed "Done" and exited 0, staging a package-less Python that jpackage would happily bundle. Verification is now fatal and checks all six required imports (`engine/stage-python.ps1`) |
| The exe launched nothing at all | `StreamFlowApp extends Application`, and jpackage puts JavaFX on the **classpath** — the JVM refuses that combination with *"JavaFX runtime components are missing"*. `StreamFLOW.exe` is a windowed binary with no console, so the error was invisible and double-clicking appeared to do nothing. Fixed by `org.streamflow.Launcher`, which does not extend `Application` (`app/src/main/java/org/streamflow/Launcher.java`) |
| `--app-content` preserves the staged directory's **name** | The staging tree must therefore keep the leaf name `content`; staging it as `streamflow-content-<pid>` produced an image with no `content\` at all and an app that could not find its engine (`app/package.ps1`) |
| Engine errors blamed R when Python was running | The bridge is still called `RBridge`, and its messages hardcoded "R engine" — the ready log literally printed `R engine ready: {"py":"3.11.9"}`, and a failed start raised an *"R analysis engine could not be started"* dialog sending users to install an R they do not need. Messages now name the engine that actually ran (`RBridge.java` `engineKind()`, `StreamFlowApp.java`) |

The ~1 GB `content` staging tree is now built under `$env:TEMP` rather than in `app/target`, and
build-directory deletion retries: background file scanners hold handles on freshly written build
output and break the next build's cleanup. The culprit here was **WinZip Background Tools**
(`WzBGTools64.exe`) — identified via the Restart Manager API, not guesswork; OneDrive, indexers
and AV behave the same way.

Same failure class as the historical `mvn clean`-wipes-the-staged-engine bug. **Never reintroduce
`mvn clean` into `package.ps1`**, and never "simplify" the `..\` out of the `appDir` option.

---

## Built and verified

Re-read on 2026-08-26. The first group was already in the tree (the retired audit docs wrongly
called these open); the second group was fixed in this pass.

### Previously fixed, wrongly reported as open

| Item | Evidence |
|---|---|
| Graph-window tree-listener leak fixed | `WorkspaceModel.java:173` `removeTreeChangeListener`; registered `GraphWindowController.java:201`, removed on hidden `:243` |
| `CytoPlot` render-thread leak fixed | `CytoPlot.java:163` `dispose()` → `renderExec.shutdownNow()` `:170`; called from CompWizard, DimReduction, Export, PositivityDialog, Workstation |
| Ghost embedding across dataset change | `DimReductionController.java:370` `embeddingStale()`, `:379` `clearEmbeddingState()` |
| Auto-threshold honours the real transform | `_positivity_thresholds` (`engine/streamflow_engine.py:843`) takes a `transform` arg and reports `actual`/`requested`/`transform` |
| Naming mode auto-switches to Thresholds | `DimReductionController.java:1205-1206` |
| Impossible multi-positive names capped | `EmbeddingRules.java:107-108` — "Ambiguous (N)" / "Mixed: " |
| Single source of truth for transform | `WorkspaceModel.java:63-64` `activeTransformProperty()` |
| `refreshModules()` no longer swallows | `MainController.java:632-646` — logs and surfaces a status warning |
| `render_plot` honours compensation | `engine/streamflow_engine.py:375` |
| Setup experiment rename wired | `SetupController.java:130` |
| Stale-cache clears on workspace reset | verified in `refreshFromWorkspace()` for Plugins, Compensation, Classifier, Clustering, CrossSample, StatComparison, Statistics, Export |
| Honest clustering backend reporting | `cluster_embedding` returns `actual`/`requested` (`engine/streamflow_engine.py:750`) |
| Soft/probabilistic clustering exists | `_bayesian()` DP-prior `BayesianGaussianMixture` returns per-event `confidence` |

### Fixed in this pass

| Item | Evidence |
|---|---|
| **Unseeded downsampling** — `run_clustering` drew its events with a bare `np.random.choice`, so identical input produced different cluster percentages and medians every run. Now seeded, with the seed threaded through k-means/FlowSOM and echoed in the response. Verified: two separate processes give byte-identical results, and different seeds genuinely differ. | `engine/streamflow_engine.py:943` and `:219` (`get_events`, the subset Java renders **and gates on**) |
| **Wrong-sample answers** — `_sample_by_name` fell through to `idx = 0` for an unknown name, so a typo'd or stale sample name silently analysed the **first** sample and reported it under the requested name. Now raises and lists the loaded samples. An omitted name still means "first sample". | `engine/streamflow_engine.py` `_sample_by_name` |
| **Swallowed compensation failures** — bare `except: pass` left one sample uncompensated while its siblings were compensated, silently mixing the two in every downstream statistic. Now recorded in `STATE["comp_failures"]` and logged. | `engine/streamflow_engine.py:356` |
| **t-SNE/UMAP backend now reported**, and `perplexity` / `n_neighbors` / `min_dist` / `densmap` are exposed instead of hardcoded. Verified live: reports `openTSNE` and `umap-learn` and honours the supplied values. | `engine/streamflow_engine.py:587` and the `backend`/`params` response fields |
| **`run_kinetic` reports dropped samples** instead of silently computing a curve from a subset — `skipped`/`n_skipped`/`n_requested`, mirroring `load_wsp`. | `engine/streamflow_engine.py:2597` |
| **`AppSettings` persists** via `java.util.prefs` — export DPI and fonts survive a restart. | `app/src/main/java/org/streamflow/ui/AppSettings.java` |
| **Generation token on sample load** — a stale `get_events` response can no longer overwrite the plot after the user has navigated elsewhere. | `GraphWindowController.java`, `loadGen` |
| **Undo/Redo buttons follow the stacks** — declared as `@FXML` fields and disabled when empty. | `GraphWindowController.java` `updateUndoButtons()` |
| **Supply chain** — the CPython download is now pinned to a SHA-256 taken from the publisher's `.sha256` sidecar and verified before use; extraction calls the Windows `tar.exe` explicitly (a bare `tar` resolved to Git Bash's GNU tar, which reads `C:\` as a remote host). | `engine/stage-python.ps1` |

| **Graph Windows would not open at all** — neither double-click nor "open in graphing window". `applyAxes()` calls `setView()` (which only invalidates, deferring `recomputeRanges()` to a 50 ms debounce) and then `applyFmoLines() -> setFmo()`, which paints **synchronously**. On the first open of a sample with a Logicle axis nothing had built the logicle LUT yet, so `drawAxes -> pxX -> sxv` dereferenced a null `xLogicle`; the NPE escaped through `selectNode -> useData -> loadFromEngine -> open` and killed the window mid-construction, with no visible error. A latent race present long before this session (`CytoPlot.java` was unmodified when it surfaced). `paint()` now builds the LUTs on demand. | `CytoPlot.java` `paint()`; regression test `CytoPlotLogicleFmoRaceTest` — **verified to fail without the fix** with the exact production NPE |

### Golden invariant — protected, do not regress

**Double-clicking a population at any nesting depth opens it in a Graph Window.** Depth-safe by
construction: `PopNode` is a plain parent/children structure with no depth field;
`selfAndDescendants()` and `chain()` recurse unconditionally; `WorkstationController.buildPop()`
and `GraphWindowController.buildItem()` both recurse with no depth guard. If any of those are ever
rewritten to use indexed or path-based lookup, re-verify against a 4–5 level tree before merging.
No automated test covers this end-to-end yet — worth adding as a pure-Java structural test.

---

## Genuinely open

| Pri | Item | Evidence |
|---|---|---|
| **P0** | **Python engine has almost no test coverage.** 9 bridge test classes point `streamflow.engine` at `streamflow_engine.R`; only `PyEngineTest` targets Python, covering 3 of 45 commands. ~42 shipping commands have zero automated protection — every engine fix above had to be verified by hand. Parametrize the bridge tests over both engines (they already skip gracefully), or add a Python job to `build-java.yml`. | `app/src/test/java/org/streamflow/bridge/*Test.java`; e.g. `AnalysisCommandTest.java:36-37` |
| **P1** | **No dependency lockfile.** `stage-python.ps1` pip-installs unpinned, so two machines can still get different library versions even though the interpreter is now hash-pinned. Commit a `requirements-lock.txt` and install from it. | `engine/stage-python.ps1` |
| **P2** | **FlowSOM grid never scales with N** — neither clustering path passes `xdim`/`ydim`. | `engine/streamflow_engine.py` `_flowsom` |
| **P2** | **`axisPopoverActive` workaround still present.** The new load-generation token covers this controller's sample loads, but not CytoPlot's internal render-busy path, which is what that flag actually guards. Removing it needs the busy-state fix, not the token. | `GraphWindowController.java:326` |
| **P2** | **CA bundle goes stale.** `engine/win-ca-bundle.pem` must be regenerated (`engine/export-win-ca.ps1`) when the TLS-intercepting AV rotates its root, or pip fails with `CERTIFICATE_VERIFY_FAILED`. Hit this on 2026-08-26; the June bundle had 136 certs, the current store 148. | `engine/export-win-ca.ps1` |

---

## Roadmap

### Spectral phase 1 — compensation upgrades (next)

Clean-room from published methods, all additive to `compute_spillover_from_controls`:

1. **Signature QC** — cosine-similarity matrix over spillover rows plus a per-pair collinearity
   index from the inverse Gram matrix; flags near-collinear fluorophores before compensation
   becomes unstable. Surfaced as a second heatmap in `CompMatrixView`.
2. **Saturation exclusion** — drop events at ≥99.9 % of detector range before computing positive
   medians. Currently absent; pinned events bias medians high and over-compensate.
3. **PCA peak-detector picking** — replace `_brightest()` (highest gated median) with the argmax of
   the first principal component, robust to dim controls with high autofluorescence.
4. **kNN scatter-matched AF baseline** — replace the single global unstained median per detector
   with a k-nearest-neighbour match in FSC/SSC space, correcting autofluorescence that varies with
   cell size.

### Spectral phase 2 — full unmixing

New engine commands (`build_spectral_model`, `compute_unmixing_matrix` with OLS/WLS,
`extract_af_library`, `apply_unmixing`, `export_unmixed`), unmixed events as a derived source
alongside `raw`/`comp`, and a Spectral Process tab. The largest available differentiator: it is
what Cytek/Aurora users need and what FlowJo handles poorly.

### Also pending

- **Graph Export Layout Editor** — FlowJo-style drag-and-drop layout + batch PDF/PPT engine.
- **Gating-Strategy toolbar** — context-sensitive PowerPoint-style top toolbar for the GS canvas.
- **Vendor roadmap** — CytoNorm, autospill, InfinityFlow (clustering backends already done:
  phenograph, bayesian, flowgrid).

---

## Method attribution

Compensation follows **Bagwell–Adams** median spillover. Logicle implements
**Parks–Moore–Roederer** (Cytometry A, 2012). The spectral roadmap draws on published methods
only: **Novo, Grégori & Rajwa** (Cytometry A, 2013) for weighted-least-squares unmixing, and the
**AutoSpectral** control-cleaning and per-cell autofluorescence write-ups from Colibri Cytometry.

`honeychrome/`, if present in your working tree, is an **LGPL-3.0 reference checkout** — gitignored
and read-only. Do not copy, transliterate or adapt its source; work from the papers.
