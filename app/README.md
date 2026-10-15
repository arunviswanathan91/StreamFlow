# StreamFLOW — JavaFX desktop app

Native JavaFX front-end for StreamFLOW. Plotting and gating run **natively in Java**
(`CytoPlot` renders off the FX thread); analysis runs in a headless **Python engine**
(`../engine/streamflow_engine.py`, FlowKit/NumPy/SciPy/scikit-learn) spawned as a child process
and driven over a JSON line protocol, with bulk event data shipped as little-endian `float32`
blobs.

A legacy **R** engine (`../engine/streamflow_engine.R`) is retained for R-based plugins and for a
subset of the test suite. The Python engine is the default everywhere the app actually runs —
`run-dev.ps1` and `package.ps1` both select it explicitly.

See [../STATUS.md](../STATUS.md) for what is built vs. open.

## Prerequisites

The repo bundles a JDK and Maven, so no system install is strictly required:

- **JDK**: bundled Microsoft Build of OpenJDK at `../jdk-25.0.3+9`. The pom targets
  `release 21`, so any JDK 21+ works — `build.ps1`/`package.ps1` fall back to `JAVA_HOME`.
  `jpackage` is required for packaging.
- **Maven**: bundled Apache Maven 3.9.16 at `../apache-maven-3.9.16`, with fallback to `mvn`
  on PATH.
- `build.ps1` / `run-dev.ps1` pin `JAVA_HOME` and set `MAVEN_OPTS` to trust the **Windows
  certificate store**, so Maven Central works behind a TLS-intercepting proxy or AV (otherwise
  you get a PKIX "unable to find valid certification path" error).
- **Python**: a staged environment at `../engine/py-env` for development
  (`../engine/stage-python.ps1`), or the relocatable `target/python` that packaging bundles.
- **R** is optional — needed only for R-based plugins and the R-engine tests.

> ⚠️ **OneDrive.** This repo syncs through OneDrive, which can leave files as placeholders that
> native processes cannot read. Symptoms look like code bugs but are not: the bundled JDK fails
> with `Error loading java.security file`, and Python tests fail with `OSError: [Errno 22]` on
> FCS fixtures. Mark `jdk-*`, `test-assests` and `engine/py-env` as *Always keep on this device*,
> or move the repo out of OneDrive. OneDrive can also transiently lock `target/` — if
> `mvn clean` fails to delete it, retry.

## Run (dev)

```powershell
# from app\
./run-dev.ps1          # builds, then launches via the JavaFX Maven plugin
./build.ps1            # build only
```

## Test

```powershell
$env:JAVA_HOME = "C:\Program Files\Java\jdk-21.0.10"   # or the bundled JDK
mvn -B clean test
```

The suite covers the FXML markup (`FxmlLoadTest` loads every FXML through a real `FXMLLoader`),
pure-Java maths (`StatsTest`, `LogicleTest`, `CoordinateMapperTest`, `EmbeddingRulesTest`),
plotting (`CytoPlot*Test`), the plugin registry, and the engine bridge.

> **Coverage caveat.** Most `bridge/` tests point `streamflow.engine` at the **R** script;
> only `PyEngineTest` targets Python, and it covers 3 of 45 commands. A green build proves
> less about the shipping engine than it appears to — see [../STATUS.md](../STATUS.md) (P0).

Tests needing R or the large FCS fixtures self-skip via JUnit assumptions; a skipped test in CI
is expected, not a failure.

## Package a Windows app (jpackage)

Produces a self-contained app-image (StreamFLOW.exe + private JRE + engine + portable Python),
so end users need neither Java nor Python:

```powershell
# 1) when Python deps change — stage the relocatable runtime into app\target\python:
powershell -ExecutionPolicy Bypass -File ..\engine\stage-python.ps1

# 2) optional: stage R too, only if you want the R plugin runway bundled:
powershell -ExecutionPolicy Bypass -File ..\engine\stage-r-portable.ps1

# 3) build — app-image (no WiX needed):
powershell -ExecutionPolicy Bypass -File .\package.ps1 -SkipR -Version 1.2.0
#    or a real installer (needs WiX 3.x on PATH):
powershell -ExecutionPolicy Bypass -File .\package.ps1 -Installer -Version 1.2.0

# output: app\target\dist\
```

`package.ps1` deliberately does **not** run `mvn clean` — `target/python` and `target/R-Portable`
are staged into `target` before it runs, and cleaning would delete them, producing an installer
with no engine. It removes only the Maven build outputs.

The packaged app resolves its bundled engine via `-Dstreamflow.appDir` and selects the Python
engine via `-Dstreamflow.engine.kind=python`, both baked in by `package.ps1`.

CI: `.github/workflows/release.yml` installs WiX and publishes the installer as a GitHub Release
on a `v*` tag.

## Layout

```
app/
  pom.xml                      Maven build (OpenJFX 21, ControlsFX, Ikonli, Jackson, PDFBox, JUnit 5)
  run-dev.ps1 / build.ps1      JDK-pinned launchers
  package.ps1                  jpackage packaging (app-image or .exe installer)
  src/main/java/org/streamflow/
    StreamFlowApp.java         JavaFX entry point
    bridge/                    engine process + stdio JSON protocol + async job model
                               (RBridge, EnginePaths, RJob, BridgeService, RProgress)
    plugins/                   plugin discovery and registry
    ui/                        27 controllers + CytoPlot (native plot/gating canvas),
                               WorkspaceModel, GraphWindowController, Stats, AppSettings
  src/main/resources/org/streamflow/ui/
    *.fxml, streamflow-dark.css
  src/test/java/org/streamflow/
    bridge/                    protocol conformance + per-command-family tests
    ui/                        maths, plotting and FXML-integrity tests
```
