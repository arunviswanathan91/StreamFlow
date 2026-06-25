# StreamFLOW — Java desktop UI (Phase 0)

Native JavaFX desktop front-end for the StreamFLOW flow-cytometry suite. The
heavy compute stays in R (Bioconductor / CytoExploreR) running as a background
process; Java owns the window, file dialogs, and async orchestration. See
`../engine/streamflow_engine.R` for the R side and the project plan for the full
architecture.

This is **Phase 0**: the JavaFX ⇄ R bridge, the async job model, the app shell,
and a developer console that proves the architecture end-to-end. The real
analysis modules land in later phases.

## Prerequisites

The repo is **self-contained for building** — no system JDK or Maven needed:

- **JDK**: bundled **Microsoft Build of OpenJDK 25 (LTS)** at `../jdk-25.0.3+9`.
- **Maven**: bundled **Apache Maven 3.9.16** at `../apache-maven-3.9.16`.
- The `build.ps1` / `run-dev.ps1` scripts pin `JAVA_HOME`, use the bundled Maven,
  and set `MAVEN_OPTS` to trust the **Windows certificate store** (so Maven Central
  works behind a TLS-intercepting proxy/AV — otherwise you'd get a PKIX
  "unable to find valid certification path" error).
- **R** is the only external requirement: `jsonlite` for Phase 0, and from Phase 1
  on the full Bioconductor/CytoExploreR stack. In a packaged build this is the
  bundled `R-Portable`; in dev, point the scripts at any Rscript with `-Rscript`.

> The repo lives in a OneDrive-synced folder, which can transiently lock
> `target/`; if `mvn clean` fails to delete it, just `rm -rf target` and retry.

## Run the app (dev)

```powershell
# from app\
./run-dev.ps1                                   # Rscript from PATH / bundled R-Portable
./run-dev.ps1 -Rscript "C:\Program Files\R\R-4.4.2\bin\Rscript.exe"
```

Then in the **Developer / Engine Console**:
- **Ping / R Version** — basic round-trip over the stdio JSON protocol.
- **Sleep (5s, cancellable)** — start it, then click the nav items and drag the
  window: the UI stays fully responsive while R works (the freeze fix). Hit
  **Cancel** to prove cooperative cancellation.
- **Trigger Error** — shows the real R error message surfaced to the UI.
- **Noisy stdout** — R does `cat()`/`print()`; the JSON stream stays intact
  (proves the `sink(stderr())` guard).

## Verify (Phase 0 gate)

```powershell
# from app\  — runs the protocol conformance test against a real R engine
./build.ps1 -Rscript "C:\Program Files\R\R-4.4.2\bin\Rscript.exe"
```

`ProtocolConformanceTest` asserts: ready handshake, echo/version round-trip,
progress frames, cooperative cancel, error+traceback replies, malformed-request
resilience, and that stray R stdout never corrupts the stream. (The test is
skipped automatically if no Rscript is available.)

Orphan-safety check (manual): launch the app, find the child `Rscript.exe` in
Task Manager, kill the `java`/JavaFX process — the engine should exit on its own
within seconds (stdin EOF), leaving no lingering R process.

## Build a Windows app (jpackage)

Produces a self-contained app-image (StreamFLOW.exe + bundled JRE + R), so end
users need neither Java nor R:

```powershell
# 1) one-time / when R deps change — stage a minimal portable R (slow, ~1 GB):
powershell -ExecutionPolicy Bypass -File engine\stage-r-portable.ps1
# 2) build the app image (or add -Installer for an .exe, needs WiX):
powershell -ExecutionPolicy Bypass -File app\package.ps1
# output: app\target\dist\StreamFLOW\
```

The app resolves the bundled engine + R-Portable via `-Dstreamflow.appDir`
(set by `package.ps1`). *Status: scripts are ready; the first packaged build still
needs a validation run (jpackage classpath + runtime path).*

## Layout

```
app/
  pom.xml                         Maven build (OpenJFX, controlsfx, Jackson, JUnit)
  run-dev.ps1 / build.ps1         JDK-pinned launchers
  src/main/java/org/streamflow/
    StreamFlowApp.java            JavaFX entry point
    bridge/                       R process + stdio JSON protocol + async jobs
    ui/MainController.java         shell controller (Phase 0 dev console)
  src/main/resources/org/streamflow/ui/
    main.fxml, streamflow-dark.css
  src/test/java/.../ProtocolConformanceTest.java
```
