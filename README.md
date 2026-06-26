<div align="center">

<img src="streamflow-icon.png" alt="StreamFLOW" width="180"/>

# StreamFLOW

**Native JavaFX flow-cytometry analysis with a Python / FlowKit engine.**
FlowJo-class gating, compensation and analysis — open, fast, and reproducible.

</div>

---

## What it is

StreamFLOW is a desktop flow-cytometry analysis application. The user interface is a native
**JavaFX** app (instant, GPU-accelerated plotting and gating — no web stack), backed by a headless
**Python** analysis engine built on [FlowKit](https://github.com/whitews/FlowKit) / FlowUtils /
NumPy / SciPy / scikit-learn. The two communicate over a simple JSON line protocol, with bulk event
data shipped as little-endian `float32` blobs for native rendering.

The goal is to match FlowJo's workflow where it counts and improve on it where it doesn't: an
always-on audit trail, undo/redo that actually works, live gate statistics, a real before/after
compensation preview, and analysis modules (cell cycle, proliferation, apoptosis, classifier,
cross-sample) that are interactive rather than static images.

## Highlights

- **FlowJo-style gating** — Graph Windows with pseudocolor / dot / contour / density / histogram
  plots, polygon / rectangle / ellipse / interval / quadrant gates, a live gating tree, drill-down
  child windows, and gate sync across every open window.
- **Axis transforms** — Linear, Log, Logicle (biexponential) and ArcSinh, with per-axis tuning.
- **Compensation** — extract an embedded `$SPILL` matrix or **compute one from single-stain
  controls** (Bagwell-Adams) via a guided wizard; interactive heatmap + table editing; a true
  **before/after preview** on any channel pair; and a residual-correlation diagnostic that flags
  over/under-compensated pairs straight on the heatmap.
- **Analysis modules** — Cell Cycle (Watson / Dean-Jett-Fox), Proliferation (dye-dilution
  generations), Apoptosis (Annexin/PI quadrants), Statistics, Cross-Sample (gate×sample matrix,
  consistency, radar, MFI drift), Sample Classifier (PCA + Random Forest), Kinetic, Longitudinal,
  Dimensionality Reduction and Clustering.
- **Reproducible** — a session audit log, a Methods-paragraph generator from the gating tree,
  workspace save/load (`.sfw`) and GatingML export.
- **Publication export** — high-DPI clipboard images and SVG vector output, with configurable DPI,
  font size and gate-label options.

## Screenshots

The app icon (above) and module screens use the StreamFLOW dark theme. See **Help → User Guide**
inside the application for a guided tour.

## Architecture

```
┌──────────────────────────┐        JSON line protocol         ┌───────────────────────────┐
│  JavaFX desktop app       │  ───────────────────────────────▶ │  Python engine            │
│  (app/)                   │     commands + args (stdin)        │  (engine/)                │
│                           │ ◀───────────────────────────────  │  streamflow_engine.py     │
│  • CytoPlot native render │     results / progress (stdout)    │                           │
│  • Graph Windows + gating │                                    │  • FlowKit / FlowUtils    │
│  • Compensation wizard    │     event data as float32 blobs    │  • NumPy / SciPy          │
│  • Analysis modules       │ ◀───────────────────────────────  │  • scikit-learn, etc.     │
└──────────────────────────┘                                    └───────────────────────────┘
```

- **`app/`** — the JavaFX application (Java 25, Maven). Plotting/gating run natively in
  `CytoPlot`; heavy event binning is computed off the FX thread.
- **`engine/`** — the Python analysis engine (`streamflow_engine.py`) plus a portable virtual
  environment under `engine/py-env`. A legacy R engine (`streamflow_engine.R`) is retained for
  reference and a subset of tests.

## Requirements

- Windows 10/11 (primary target; the stack is cross-platform).
- **Microsoft Build of OpenJDK 25** (bundled under `jdk-25.0.3+9` in dev setups).
- **Maven** (a copy is bundled under `apache-maven-3.9.16`).
- A staged Python environment under `engine/py-env` (see `engine/stage-python.ps1`).

## Build & run

From the `app/` directory:

```powershell
# Run in development (builds, then launches via the JavaFX Maven plugin)
./run-dev.ps1

# Build only
./build.ps1

# Package a distributable
./package.ps1
```

Or directly with the bundled Maven:

```powershell
$env:JAVA_HOME = "<repo>/jdk-25.0.3+9"
./apache-maven-3.9.16/bin/mvn -f app/pom.xml javafx:run
```

## Tests

```powershell
$env:JAVA_HOME = "<repo>/jdk-25.0.3+9"
./apache-maven-3.9.16/bin/mvn -f app/pom.xml test
```

The suite includes `FxmlLoadTest` (loads every FXML through a real `FXMLLoader` to catch markup
regressions) and `PyEngineTest` (exercises the live Python engine). Engine-dependent bridge tests
skip automatically when their data or runtime is absent.

## Help

Inside the app: **Help → User Guide** for the end-to-end workflow, and **Help → Compensation Help**
for the compensation method (controls, gating, Bagwell-Adams, residual diagnostic).

## Acknowledgements — standing on the shoulders of giants

StreamFLOW is built entirely on open-source software. **Without these projects and the people behind
them, it simply would not exist.** Our deepest thanks to every maintainer and contributor. 🙏

### Analysis engine (Python)

| Project | What it gives StreamFLOW | Link |
|---|---|---|
| **FlowKit** | FCS handling, GatingML/FlowJo interop, compensation application | <https://github.com/whitews/FlowKit> |
| **FlowUtils** | Low-level compensation & transform math (incl. OLS spectral) | <https://github.com/whitews/FlowUtils> |
| **FlowIO** | Pure-Python FCS file reader | <https://github.com/whitews/FlowIO> |
| **NumPy** | Array math underpinning every computation | <https://github.com/numpy/numpy> |
| **SciPy** | Curve fitting, signal processing, statistics | <https://github.com/scipy/scipy> |
| **pandas** | Event tables / dataframes | <https://github.com/pandas-dev/pandas> |
| **scikit-learn** | PCA, Random Forest, clustering | <https://github.com/scikit-learn/scikit-learn> |
| **matplotlib** | Server-side plot rendering | <https://github.com/matplotlib/matplotlib> |
| **UMAP (umap-learn)** | Dimensionality reduction | <https://github.com/lmcinnes/umap> |
| **openTSNE** | Fast t-SNE embeddings | <https://github.com/pavlin-policar/openTSNE> |
| **PhenoGraph** | Graph-based clustering | <https://github.com/dpeerlab/PhenoGraph> |
| **FlowSOM (Python)** | Self-organizing-map clustering | <https://github.com/saeyslab/FlowSOM_Python> |
| **AnnData** | Annotated single-cell data structures | <https://github.com/scverse/anndata> |

### Desktop app (Java / JavaFX)

| Project | What it gives StreamFLOW | Link |
|---|---|---|
| **OpenJFX (JavaFX)** | The entire native UI, canvas rendering & 3D | <https://github.com/openjdk/jfx> |
| **ControlsFX** | PopOver and richer JavaFX controls | <https://github.com/controlsfx/controlsfx> |
| **Ikonli** | Icon fonts (FontAwesome 5, Material Design 2) | <https://github.com/kordamp/ikonli> |
| **Jackson** | JSON protocol between the app and engine | <https://github.com/FasterXML/jackson> |
| **SLF4J** | Logging API | <https://github.com/qos-ch/slf4j> |
| **Logback** | Logging implementation | <https://github.com/qos-ch/logback> |
| **JUnit 5** | Test framework | <https://github.com/junit-team/junit5> |
| **TestFX** | JavaFX UI testing | <https://github.com/TestFX/TestFX> |

### Build & runtime

| Project | Role | Link |
|---|---|---|
| **Microsoft Build of OpenJDK** | Java 21+ runtime / toolchain | <https://github.com/microsoft/openjdk> |
| **Apache Maven** | Build & dependency management | <https://github.com/apache/maven> |
| **javafx-maven-plugin** | Run/package the JavaFX app | <https://github.com/openjfx/javafx-maven-plugin> |

### Methods & inspiration

The compensation workflow follows the **Bagwell-Adams** median method and is informed by the
approaches in [flowStats](https://github.com/RGLab/flowStats) and
[CytoExploreR](https://github.com/DillonHammill/CytoExploreR); the **Logicle** scale implements
Parks–Moore–Roederer (Cytometry A, 2012). [FlowKit's](https://github.com/whitews/FlowKit) design and
the [scverse](https://github.com/scverse) ecosystem (incl. [pytometry](https://github.com/scverse/pytometry))
shaped how StreamFLOW thinks about reproducible cytometry. FlowJo and FCS Express set the bar we aim to clear.

## License

See repository for licensing details.
