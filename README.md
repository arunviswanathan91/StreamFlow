<div align="center">

<img src="streamflow-icon.png" alt="StreamFLOW" width="180"/>

# StreamFLOW

**Flow-cytometry analysis as one native Windows program.**
FlowJo-class gating, compensation, spectral unmixing and analysis — open, fast and reproducible.

</div>

---

## What it is

StreamFLOW is a desktop flow-cytometry analysis application written in **Rust**. It ships as a
single executable: no installer, no Java, no Python, no Visual C++ redistributable. The reader,
the statistics, the transforms, compensation, gating, clustering and embeddings are all native
code in this repository, and each is pinned by tests to an established reference implementation
(FlowKit, FlowUtils, SciPy, scikit-learn, umap-learn, openTSNE, PhenoGraph, FlowSOM).

The goal is to match FlowJo's workflow where it counts and improve on it where it doesn't: undo
that works, live gate statistics, a real before/after compensation preview, and analysis modules
that are interactive rather than static images.

> The original JavaFX + Python application is still in the repository (`app/`, `engine/`) as the
> legacy version. See [Legacy JavaFX / Python app](#legacy-javafx--python-app).

## Download and run

1. Get `StreamFLOW-<version>-win-x64.exe` from the repository's
   [Releases](https://github.com/arunviswanathan91/StreamFlow/releases) page.
2. Double-click it. Nothing else needs to be installed.

Notes:

- **Windows 10 / 11, 64-bit.**
- The file is not code-signed, so SmartScreen may show "Windows protected your PC". Choose
  **More info**, then **Run anyway**.
- The window is drawn with OpenGL, so the PC needs a working graphics driver (OpenGL 2.0 or
  newer). On a machine without one — some virtual machines and remote-desktop sessions — the
  program exits at start without showing a window.
- `SHA256SUMS.txt` next to the download holds its checksum.

## Highlights

- **Gating** — Graph Windows with pseudocolour, dot, contour, density, zebra and histogram plots;
  rectangle, ellipse, polygon, interval and quadrant gates; Boolean (And / Or / Not) gates; move
  and resize with handles; a gating tree of any depth; detachable windows; undo and redo.
- **Groups** — apply a gate to every sample in a group, keep them in sync, or release one sample
  to edit it on its own.
- **Axis scales** — Linear, Log, Logicle, ArcSinh and FlowJo Biex.
- **Compensation** — use the matrix embedded in the file, import one from CSV, or compute one
  from single-stain controls in a guided wizard (medians, slope or refined regression) with a
  per-control scatter gate, a before/after view on any channel pair, spreading and similarity
  tables.
- **Spectral unmixing** — signatures from controls, OLS / WLS / NNLS unmixing, per-cell
  autofluorescence and signature quality checks.
- **Discovery** — UMAP and t-SNE embeddings; FlowSOM, PhenoGraph and k-means clustering; cluster
  naming by marker; approve a cluster and it becomes a population in the gating tree.
- **Analysis modules** — Statistics, Cell Cycle (Watson / Dean-Jett-Fox), Proliferation,
  Apoptosis, Stats Comparison (Mann-Whitney, Wilcoxon, Kruskal-Wallis, Dunn), Kinetic,
  Cross-Sample and Classifier (PCA + random forest).
- **FMO and comparison** — FMO reference lines per channel and a compare strip showing the same
  population across every sample on shared axes.
- **Export** — plots as PDF, SVG or PNG at a chosen DPI; a Layout editor with drag-and-drop
  tiles, overlays and batch PDF; plot pages as PowerPoint; statistics as CSV or Excel; gated
  events as FCS.
- **Interoperability** — reads FCS files, imports FlowJo `.wsp` workspaces, exports GatingML 2.0,
  and saves its own workspaces as `.sfw`.

**Not built yet:** the Longitudinal, 3D Scatter and Plugins screens are placeholders.

## Repository layout

```
rust/                     the application
  crates/sf-fcs           FCS reader and writer
  crates/sf-core          statistics, scales, compensation, spectral unmixing, clustering,
                          t-SNE, UMAP, hypothesis tests, curve fitting (no third-party crates)
  crates/sf-gating        gates, gating tree, Boolean gates, .sfw workspaces, FlowJo .wsp
                          import, GatingML export
  crates/sf-app           the desktop program (egui); builds streamflow.exe
  tools/                  Python scripts that regenerate the reference numbers the tests pin to
  check-binary.ps1        checks the built exe is self-contained and carries its icon
  smoke-test.ps1          starts the built exe and checks that it draws a frame
app/, engine/             legacy JavaFX app and Python engine
.github/workflows/        CI and release
```

## Build from source

Requires the stable [Rust toolchain](https://rustup.rs) with the MSVC target on Windows.

```powershell
cd rust
cargo build --release -p sf-app          # produces target\release\streamflow.exe
cargo run   --release -p sf-app          # build and start
```

You can pass FCS files on the command line: `streamflow.exe sample1.fcs sample2.fcs`.

## Tests

```powershell
cd rust
cargo test --workspace --release
```

Run the suite in release mode: the t-SNE, UMAP and clustering reference tests are slow
unoptimised.

The numerical tests compare the Rust code with numbers produced by the reference Python
libraries; those numbers are stored as text files next to the tests and regenerated with the
scripts in `rust/tools/`. Tests that need the large FCS fixtures (`test-assests/`, not in the
repository) skip themselves when the files are absent, so a clone passes without them.

After building, check the binary and start it:

```powershell
powershell -File rust\check-binary.ps1 -Exe rust\target\release\streamflow.exe
powershell -File rust\smoke-test.ps1   -Exe rust\target\release\streamflow.exe
```

## CI and releases

| Workflow | When | What it does |
|---|---|---|
| `build-rust.yml` | push or pull request touching `rust/` | tests the workspace, builds the exe, checks it, uploads it as a run artifact |
| `release.yml` | a `v*` tag, or a manual run | tests, builds and checks the exe, then publishes `StreamFLOW-<version>-win-x64.exe` as a GitHub Release |
| `build-java.yml` | changes to the legacy app | tests the JavaFX app and its engines |

To release: set the version in `[workspace.package]` in `rust/Cargo.toml`, then push a matching
tag (`v0.1.0` for version `0.1.0`). The workflow refuses to build if the two disagree. A manual
run of `release.yml` is a dry run unless **publish** is ticked: the exe is attached to the
workflow run and nothing public is created.

GitHub's hosted Windows runners have no usable OpenGL driver, so the "start the application" step
is informational there. Run `rust\smoke-test.ps1` on a real PC before tagging.

## Legacy JavaFX / Python app

`app/` is the original JavaFX desktop app (Java 21, Maven) and `engine/` its headless Python
engine (FlowKit, NumPy, SciPy, scikit-learn), talking over a JSON line protocol. It is kept for
reference and as the source the Rust port was verified against; releases no longer include it.

```powershell
$env:JAVA_HOME = "C:\Program Files\Java\jdk-21.0.10"   # any JDK 21+
mvn -f app/pom.xml test          # test suite
mvn -f app/pom.xml javafx:run    # run the app
```

## Acknowledgements

StreamFLOW stands on open-source work. Thank you to every maintainer and contributor.

### Built with

| Project | What it gives StreamFLOW | Link |
|---|---|---|
| **egui / eframe** | The whole user interface and window | <https://github.com/emilk/egui> |
| **rfd** | Native open / save dialogs | <https://github.com/PolyMeilex/rfd> |
| **arboard** | Copying images to the clipboard | <https://github.com/1Password/arboard> |
| **ab_glyph** | Text in exported PNG images | <https://github.com/alexheretic/ab-glyph> |
| **png** | PNG encoding | <https://github.com/image-rs/image-png> |
| **miniz_oxide** | Compression for PDF, Excel and PowerPoint output | <https://github.com/Frommi/miniz_oxide> |
| **winresource** | Icon and version details in the exe | <https://github.com/BenjaminRi/winresource> |

### Verified against

The Rust implementations are checked number for number, or by agreement metrics where exact
equality is not possible, against these projects.

| Project | Used to verify | Link |
|---|---|---|
| **FlowKit** | Gate membership, compensation, GatingML, FlowJo workspaces | <https://github.com/whitews/FlowKit> |
| **FlowUtils** | Logicle and other transforms | <https://github.com/whitews/FlowUtils> |
| **FlowIO** | FCS parsing | <https://github.com/whitews/FlowIO> |
| **NumPy / SciPy** | Linear algebra, curve fitting, peak finding, hypothesis tests | <https://github.com/scipy/scipy> |
| **scikit-learn** | PCA, random forest, trustworthiness | <https://github.com/scikit-learn/scikit-learn> |
| **umap-learn** | UMAP | <https://github.com/lmcinnes/umap> |
| **openTSNE** | t-SNE | <https://github.com/pavlin-policar/openTSNE> |
| **PhenoGraph** | Graph-based clustering | <https://github.com/dpeerlab/PhenoGraph> |
| **FlowSOM (Python)** | Self-organising-map clustering | <https://github.com/saeyslab/FlowSOM_Python> |

### Methods

Compensation follows the **Bagwell–Adams** median method. The Logicle scale implements
**Parks–Moore–Roederer** (Cytometry A, 2012). Spectral unmixing follows **Novo, Grégori &
Rajwa** (Cytometry A, 2013). FlowSOM follows **Van Gassen et al.** (Cytometry A, 2015) and
consensus clustering **Monti et al.** (2003). FlowJo and FCS Express set the bar we aim to clear.

## License

StreamFLOW is licensed under the **Apache License 2.0** — see [LICENSE](LICENSE).

Third-party dependencies remain under their own licenses; see [NOTICE](NOTICE),
[THIRD-PARTY-NOTICES.txt](THIRD-PARTY-NOTICES.txt) and the acknowledgements above.
