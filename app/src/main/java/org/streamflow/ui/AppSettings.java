package org.streamflow.ui;

import java.util.prefs.Preferences;

/**
 * Session-wide user settings (Settings window → Export tab). Held in {@link AppContext}.
 * Screen rendering is ~96 DPI; {@link #exportScale()} scales snapshots up for publication output.
 *
 * <p>Values persist per-user via {@link Preferences}: an export DPI or font size chosen once should
 * still be in effect after a restart, rather than silently reverting to the defaults mid-project.
 */
public final class AppSettings {

    /** Per-user store. Keyed by this class's package, so it survives app upgrades. */
    private static final Preferences PREFS = Preferences.userNodeForPackage(AppSettings.class);

    private int exportDpi = 300;              // publication default
    private double exportFontSize = 12.0;     // gate-label font size for scatter plots (px at screen res)
    private double exportAxisFontSize = 12.0; // axis title / tick font size for scatter plots (px at screen res)
    private int exportPointSize = 0;          // pseudocolor/dot point radius in pixels (0 = 1px)
    private boolean exportGateLabels = true;  // show gate names + stats in scatter-plot export
    // Analysis-chart (AnalysisChart) specific — applied at export DPI (scaled by dpi/96)
    private double chartTitleFontSize = 14.0; // title line font size (px at screen res)
    private double chartLegendFontSize = 11.0; // legend label + key swatch font size (px at screen res)

    // Settings window "General" tab — defaults applied at startup / New Workspace / new Graph Window.
    // Changing one of these does not retroactively touch already-open windows; it seeds new ones.
    private boolean defaultAutoSave = true;
    private boolean defaultLightBackground = false;
    private boolean defaultInterceptLines = false;

    public AppSettings() {
        load();
    }

    /**
     * Restore persisted values over the defaults above. A missing key keeps its default, so a fresh
     * install and a corrupt store both degrade to the same known-good configuration.
     */
    private void load() {
        try {
            exportDpi = PREFS.getInt("exportDpi", exportDpi);
            exportFontSize = PREFS.getDouble("exportFontSize", exportFontSize);
            exportAxisFontSize = PREFS.getDouble("exportAxisFontSize", exportAxisFontSize);
            exportPointSize = PREFS.getInt("exportPointSize", exportPointSize);
            exportGateLabels = PREFS.getBoolean("exportGateLabels", exportGateLabels);
            chartTitleFontSize = PREFS.getDouble("chartTitleFontSize", chartTitleFontSize);
            chartLegendFontSize = PREFS.getDouble("chartLegendFontSize", chartLegendFontSize);
            defaultAutoSave = PREFS.getBoolean("defaultAutoSave", defaultAutoSave);
            defaultLightBackground = PREFS.getBoolean("defaultLightBackground", defaultLightBackground);
            defaultInterceptLines = PREFS.getBoolean("defaultInterceptLines", defaultInterceptLines);
        } catch (Exception ex) {
            // A locked or unavailable preferences store must never stop the app from starting —
            // the in-code defaults above are always a valid configuration.
        }
    }

    /** Write every value back. Called from {@link #fireChanged()}, so all setters persist. */
    private void save() {
        try {
            PREFS.putInt("exportDpi", exportDpi);
            PREFS.putDouble("exportFontSize", exportFontSize);
            PREFS.putDouble("exportAxisFontSize", exportAxisFontSize);
            PREFS.putInt("exportPointSize", exportPointSize);
            PREFS.putBoolean("exportGateLabels", exportGateLabels);
            PREFS.putDouble("chartTitleFontSize", chartTitleFontSize);
            PREFS.putDouble("chartLegendFontSize", chartLegendFontSize);
            PREFS.putBoolean("defaultAutoSave", defaultAutoSave);
            PREFS.putBoolean("defaultLightBackground", defaultLightBackground);
            PREFS.putBoolean("defaultInterceptLines", defaultInterceptLines);
        } catch (Exception ex) {
            // Failing to persist is not worth interrupting the user's work over.
        }
    }

    /** Reset every setting to its built-in default and clear the persisted store. */
    public void resetToDefaults() {
        exportDpi = 300;
        exportFontSize = 12.0;
        exportAxisFontSize = 12.0;
        exportPointSize = 0;
        exportGateLabels = true;
        chartTitleFontSize = 14.0;
        chartLegendFontSize = 11.0;
        defaultAutoSave = true;
        defaultLightBackground = false;
        defaultInterceptLines = false;
        try { PREFS.clear(); } catch (Exception ignored) { }
        fireChanged();
    }

    public boolean defaultAutoSave() { return defaultAutoSave; }
    public void setDefaultAutoSave(boolean b) { defaultAutoSave = b; fireChanged(); }
    public boolean defaultLightBackground() { return defaultLightBackground; }
    public void setDefaultLightBackground(boolean b) { defaultLightBackground = b; fireChanged(); }
    public boolean defaultInterceptLines() { return defaultInterceptLines; }
    public void setDefaultInterceptLines(boolean b) { defaultInterceptLines = b; fireChanged(); }

    // App-wide format: changing any of these notifies every open plot so Copy is consistent everywhere.
    private final java.util.List<Runnable> listeners = new java.util.ArrayList<>();
    public void addChangeListener(Runnable r) { listeners.add(r); }
    public void removeChangeListener(Runnable r) { listeners.remove(r); }
    private void fireChanged() {
        save();
        for (Runnable r : new java.util.ArrayList<>(listeners)) { try { r.run(); } catch (Exception ignored) {} }
    }

    public int exportDpi() { return exportDpi; }
    public void setExportDpi(int dpi) { exportDpi = Math.max(72, Math.min(1200, dpi)); fireChanged(); }

    public double exportFontSize() { return exportFontSize; }
    public void setExportFontSize(double size) { exportFontSize = Math.max(6, Math.min(36, size)); fireChanged(); }

    public double exportAxisFontSize() { return exportAxisFontSize; }
    public void setExportAxisFontSize(double size) { exportAxisFontSize = Math.max(6, Math.min(36, size)); fireChanged(); }

    public int exportPointSize() { return exportPointSize; }
    public void setExportPointSize(int r) { exportPointSize = Math.max(0, Math.min(4, r)); fireChanged(); }

    public boolean exportGateLabels() { return exportGateLabels; }
    public void setExportGateLabels(boolean b) { exportGateLabels = b; fireChanged(); }

    public double chartTitleFontSize() { return chartTitleFontSize; }
    public void setChartTitleFontSize(double s) { chartTitleFontSize = Math.max(8, Math.min(48, s)); fireChanged(); }

    public double chartLegendFontSize() { return chartLegendFontSize; }
    public void setChartLegendFontSize(double s) { chartLegendFontSize = Math.max(6, Math.min(36, s)); fireChanged(); }

    /** Snapshot scale factor for the configured DPI (relative to 96-DPI screen space). */
    public double exportScale() { return exportDpi / 96.0; }
}
