package org.streamflow.bridge;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

/**
 * Resolves the Rscript executable and the engine script, working both in
 * development (running from the repo) and in a packaged jpackage app image
 * (R-Portable bundled next to the app). Mirrors the {@code resourcesPath}
 * logic in the legacy {@code electron/main.js}.
 *
 * <p>Resolution order for each path is: explicit system property, then bundled
 * R-Portable relative to the app, then the repo layout, then the system PATH.
 * Overridable so tests can point at any R install.
 */
public final class RPaths {

    private RPaths() {}

    private static final boolean IS_WINDOWS =
            System.getProperty("os.name", "").toLowerCase().contains("win");

    private static final String RSCRIPT = IS_WINDOWS ? "Rscript.exe" : "Rscript";

    /** Directory the app is running from (jpackage app dir, or the working dir in dev). */
    public static Path appDir() {
        String override = System.getProperty("streamflow.appDir");
        if (override != null && !override.isBlank()) return Paths.get(override);
        return Paths.get("").toAbsolutePath();
    }

    /** R_HOME of the bundled runtime, if present. */
    public static Path bundledRHome() {
        // packaged: <appDir>/R-Portable ; dev: <repo>/R-Portable (gitignored, built at package time)
        Path[] candidates = {
                appDir().resolve("R-Portable"),
                appDir().resolve("app").resolve("R-Portable"),
                repoRoot().resolve("R-Portable")
        };
        for (Path c : candidates) {
            if (Files.isDirectory(c)) return c;
        }
        return null;
    }

    /** Best guess at the repo root when running in dev (…/StreamFlow). */
    public static Path repoRoot() {
        Path here = appDir();
        // when launched via Maven the working dir is <repo>/app; step up one if so
        if (here.getFileName() != null && here.getFileName().toString().equals("app")
                && Files.exists(here.getParent().resolve("engine"))) {
            return here.getParent();
        }
        return here;
    }

    /** Absolute path to Rscript(.exe). */
    public static Path rscript() {
        String override = System.getProperty("streamflow.rscript");
        if (override != null && !override.isBlank()) return Paths.get(override);

        Path rhome = bundledRHome();
        if (rhome != null) {
            Path exe = rhome.resolve("bin").resolve(RSCRIPT);
            if (Files.exists(exe)) return exe;
        }
        String envHome = System.getenv("R_HOME");
        if (envHome != null && !envHome.isBlank()) {
            Path exe = Paths.get(envHome, "bin", RSCRIPT);
            if (Files.exists(exe)) return exe;
        }
        // A normal Windows R install ("C:\Program Files\R\R-4.4.2") puts Rscript in bin\ but does NOT
        // add it to PATH, so without this the R plugins would be invisible on a machine that has R.
        // Prefer the highest version found.
        Path sys = newestSystemR();
        if (sys != null) return sys;
        return Paths.get(RSCRIPT); // fall back to PATH lookup
    }

    /** Newest {@code <ProgramFiles>\R\R-x.y.z\bin\Rscript.exe} on Windows, or null. */
    private static Path newestSystemR() {
        if (!IS_WINDOWS) return null;
        Path best = null; String bestVer = null;
        for (String base : new String[]{System.getenv("ProgramFiles"), System.getenv("ProgramW6432"), "C:\\R"}) {
            if (base == null || base.isBlank()) continue;
            Path rRoot = Paths.get(base, "R");
            if (!Files.isDirectory(rRoot)) continue;
            try (java.util.stream.Stream<Path> s = Files.list(rRoot)) {
                for (Path dir : s.toList()) {
                    Path exe = dir.resolve("bin").resolve(RSCRIPT);
                    if (!Files.exists(exe)) continue;
                    String ver = dir.getFileName().toString();   // e.g. "R-4.4.2"
                    if (bestVer == null || ver.compareTo(bestVer) > 0) { bestVer = ver; best = exe; }
                }
            } catch (java.io.IOException ignored) { }
        }
        return best;
    }

    /** Absolute path to streamflow_engine.R. */
    public static Path engineScript() {
        String override = System.getProperty("streamflow.engine");
        if (override != null && !override.isBlank()) return Paths.get(override);

        Path[] candidates = {
                appDir().resolve("engine").resolve("streamflow_engine.R"),
                repoRoot().resolve("engine").resolve("streamflow_engine.R")
        };
        for (Path c : candidates) {
            if (Files.exists(c)) return c.toAbsolutePath();
        }
        // last resort: assume it sits beside the app dir
        return appDir().resolve("engine").resolve("streamflow_engine.R");
    }

    /** R library path of the bundled runtime, or null to use the default. */
    public static Path bundledRLibrary() {
        Path rhome = bundledRHome();
        if (rhome == null) return null;
        Path lib = rhome.resolve("library");
        return Files.isDirectory(lib) ? lib : null;
    }
}
