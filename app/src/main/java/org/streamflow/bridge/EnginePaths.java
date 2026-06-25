package org.streamflow.bridge;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.List;

/**
 * Resolves the command used to launch the compute engine — Python (FlowKit) or
 * R — so the bridge is engine-agnostic. Python is selected when
 * {@code -Dstreamflow.python=...} is set or {@code -Dstreamflow.engine.kind=python};
 * otherwise the R engine is used. Migration target: Python by default once the
 * port is complete.
 */
public final class EnginePaths {

    private EnginePaths() {}

    // Selectable via system property (tests, surefire) OR env var (the forked
    // javafx:run app JVM inherits env, not Maven -D — same pattern as R_HOME).
    public static boolean usePython() {
        if (System.getProperty("streamflow.python") != null) return true;
        if (System.getenv("STREAMFLOW_PYTHON") != null) return true;
        String kind = System.getProperty("streamflow.engine.kind",
                System.getenv().getOrDefault("STREAMFLOW_ENGINE", ""));
        return "python".equalsIgnoreCase(kind);
    }

    public static List<String> command(Path controlDir) {
        if (usePython()) {
            return List.of(pythonExe().toString(), pyScript().toString(),
                    "--control-dir", controlDir.toString());
        }
        return List.of(RPaths.rscript().toString(), RPaths.engineScript().toString(),
                "--control-dir", controlDir.toString());
    }

    public static Path pythonExe() {
        String o = System.getProperty("streamflow.python");
        if (o == null || o.isBlank()) o = System.getenv("STREAMFLOW_PYTHON");
        if (o != null && !o.isBlank()) return Paths.get(o);
        // bundled venv (dev) / app python (packaged)
        Path[] cands = {
                RPaths.repoRoot().resolve("engine").resolve("py-env").resolve("Scripts").resolve("python.exe"),
                RPaths.appDir().resolve("python").resolve("python.exe")
        };
        for (Path c : cands) if (Files.exists(c)) return c;
        return Paths.get("python");
    }

    public static Path pyScript() {
        String o = System.getProperty("streamflow.engine.py");
        if (o == null || o.isBlank()) o = System.getenv("STREAMFLOW_ENGINE_PY");
        if (o != null && !o.isBlank()) return Paths.get(o);
        Path[] cands = {
                RPaths.appDir().resolve("engine").resolve("streamflow_engine.py"),
                RPaths.repoRoot().resolve("engine").resolve("streamflow_engine.py")
        };
        for (Path c : cands) if (Files.exists(c)) return c.toAbsolutePath();
        return RPaths.repoRoot().resolve("engine").resolve("streamflow_engine.py");
    }
}
