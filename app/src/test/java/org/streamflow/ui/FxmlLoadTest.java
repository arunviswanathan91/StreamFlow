package org.streamflow.ui;

import javafx.application.Platform;
import javafx.fxml.FXMLLoader;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.io.ByteArrayInputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Actually loads every FXML through {@link FXMLLoader} with the JavaFX toolkit running, so the
 * full view graph is built. Controllers and event handlers are stripped first so only the
 * markup is exercised (no AppContext needed). This catches the load-time failures the
 * regex-based {@link FxmlImportTest} cannot:
 * <ul>
 *   <li>unescaped {@code %resource} keys with no ResourceBundle ("No resources specified"),</li>
 *   <li>invalid/read-only properties (e.g. {@code animatedProperty} instead of {@code animated}),</li>
 *   <li>bad Ikonli icon literals (e.g. {@code fas-circle-dot} which is FontAwesome 6, not 5).</li>
 * </ul>
 * Run before launching: {@code mvn -Dtest=FxmlLoadTest test}. Skips automatically when the
 * JavaFX toolkit cannot start (headless CI without Monocle).
 */
class FxmlLoadTest {

    private static boolean fxReady;

    @BeforeAll
    static void initToolkit() {
        try {
            CountDownLatch latch = new CountDownLatch(1);
            Platform.startup(latch::countDown);
            latch.await(10, TimeUnit.SECONDS);
            fxReady = true;
        } catch (IllegalStateException alreadyStarted) {
            fxReady = true;
        } catch (Throwable headless) {
            fxReady = false;
        }
    }

    @Test
    void everyFxmlLoads() throws Exception {
        assumeTrue(fxReady, "JavaFX toolkit unavailable (headless) — skipping FXML load test");

        Path dir = Paths.get("src/main/resources/org/streamflow/ui");
        assertTrue(Files.isDirectory(dir), "FXML dir not found: " + dir.toAbsolutePath());

        List<Path> fxmls = new ArrayList<>();
        try (Stream<Path> s = Files.walk(dir)) {
            s.filter(p -> p.toString().endsWith(".fxml")).forEach(fxmls::add);
        }

        List<String> failures = new ArrayList<>();
        CountDownLatch done = new CountDownLatch(1);
        Platform.runLater(() -> {
            try {
                for (Path fxml : fxmls) {
                    try {
                        // Strip the controller and every on*="#handler" so no controller is
                        // instantiated — we only want to validate the markup itself.
                        String xml = Files.readString(fxml)
                                .replaceAll("\\s+fx:controller=\"[^\"]*\"", "")
                                .replaceAll("\\s+on[A-Z][A-Za-z]*=\"#[^\"]*\"", "");
                        new FXMLLoader().load(new ByteArrayInputStream(xml.getBytes(StandardCharsets.UTF_8)));
                    } catch (Throwable t) {
                        Throwable root = t;
                        while (root.getCause() != null) root = root.getCause();
                        failures.add(fxml.getFileName() + "  ->  "
                                + root.getClass().getSimpleName() + ": " + root.getMessage());
                    }
                }
            } finally {
                done.countDown();
            }
        });
        assertTrue(done.await(60, TimeUnit.SECONDS), "FXML load timed out");
        assertTrue(failures.isEmpty(), "FXML failed to load:\n" + String.join("\n", failures));
    }
}
