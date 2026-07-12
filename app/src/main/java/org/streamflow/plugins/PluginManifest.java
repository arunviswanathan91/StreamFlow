package org.streamflow.plugins;

import java.nio.file.Path;
import java.util.List;

/**
 * A parsed {@code plugin.json}. Plugins are R packages driven through {@code engine/run_plugin.R};
 * this is purely the declarative half — what the plugin is called, what R packages it needs, which
 * entry function to invoke, and what parameters the UI should collect.
 *
 * @param dir      the plugin directory (contains plugin.json + the entry script)
 * @param id       stable identifier, e.g. {@code peacoqc}
 * @param entryScript  script file inside {@code dir}, sourced by run_plugin.R
 * @param entryFunction  function in that script, called as {@code fn(request)}
 * @param rPackages  R packages that must be installed for this plugin to run
 * @param params   parameters the UI collects and passes through in the request JSON
 */
public record PluginManifest(
        Path dir,
        String id,
        String name,
        String version,
        String description,
        String status,
        String entryScript,
        String entryFunction,
        List<String> rPackages,
        List<Param> params,
        List<String> outputs) {

    /** One declared parameter. {@code type} drives which control the Plugins UI renders. */
    public record Param(String id, String label, String type, boolean required,
                        String help, Double min, Double max, Object defaultValue) {}

    /** Convenience: the entry script's absolute path. */
    public Path entryScriptPath() { return dir.resolve(entryScript); }
}
