package org.streamflow;

/**
 * Entry point for the <em>packaged</em> app.
 *
 * <p>jpackage puts the OpenJFX jars on the <b>classpath</b>, not the module path. When the class
 * named as the main class is itself a subclass of {@code javafx.application.Application}, the JVM's
 * launcher refuses to start it in that configuration and prints:
 *
 * <pre>Error: JavaFX runtime components are missing, and are required to run this application</pre>
 *
 * <p>Because the packaged {@code StreamFLOW.exe} is a windowed binary with no console, that message
 * goes nowhere and the app simply appears to do nothing when double-clicked.
 *
 * <p>This class does not extend {@code Application}, so the launcher check does not apply; it just
 * hands off to {@link StreamFlowApp#main(String[])}, which calls {@code launch(...)} as before. Dev
 * runs via {@code javafx:run} are unaffected — that plugin uses the module path, where
 * {@link StreamFlowApp} works as its own entry point.
 *
 * <p><b>Do not</b> point {@code app/package.ps1}'s {@code --main-class} back at
 * {@code StreamFlowApp}: it builds and packages cleanly and then fails to launch.
 */
public final class Launcher {

    private Launcher() {}

    public static void main(String[] args) {
        StreamFlowApp.main(args);
    }
}
