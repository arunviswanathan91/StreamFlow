package org.streamflow.ui;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.HashSet;
import java.util.Set;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.util.stream.Stream;

import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * Catches missing {@code <?import?>} statements in FXML without needing a display
 * — the failure mode that broke the Graph Window ("ImageView is not a valid
 * type"). For every FXML, every capitalised element tag must have a matching
 * import (explicit or wildcard).
 */
class FxmlImportTest {

    private static final Pattern TAG = Pattern.compile("<([A-Z][A-Za-z0-9]*)[\\s/>]");
    private static final Pattern IMPORT = Pattern.compile("<\\?import\\s+([\\w.]+)\\s*\\?>");

    @Test
    void everyFxmlElementHasAnImport() throws IOException {
        Path dir = Paths.get("src/main/resources/org/streamflow/ui");
        assertTrue(Files.isDirectory(dir), "FXML resource dir not found: " + dir.toAbsolutePath());

        StringBuilder problems = new StringBuilder();
        try (Stream<Path> files = Files.walk(dir)) {
            for (Path fxml : (Iterable<Path>) files.filter(p -> p.toString().endsWith(".fxml"))::iterator) {
                String text = Files.readString(fxml);

                Set<String> importedClasses = new HashSet<>();
                boolean wildcard = false;
                Matcher im = IMPORT.matcher(text);
                while (im.find()) {
                    String imp = im.group(1);
                    if (imp.endsWith(".*")) wildcard = true;
                    else importedClasses.add(imp.substring(imp.lastIndexOf('.') + 1));
                }

                Set<String> tags = new HashSet<>();
                Matcher tm = TAG.matcher(text);
                while (tm.find()) tags.add(tm.group(1));

                for (String tag : tags) {
                    if (!wildcard && !importedClasses.contains(tag)) {
                        problems.append(fxml.getFileName()).append(": <").append(tag)
                                .append("> has no matching <?import?>\n");
                    }
                }
            }
        }
        assertTrue(problems.isEmpty(), "FXML missing imports:\n" + problems);
    }
}
