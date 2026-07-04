package org.streamflow.ui;

/**
 * Session-wide user settings (Settings window → Export tab). Held in {@link AppContext}.
 * Screen rendering is ~96 DPI; {@link #exportScale()} scales snapshots up for publication output.
 */
public final class AppSettings {

    private int exportDpi = 300;              // publication default
    private double exportFontSize = 12.0;     // gate-label font size for scatter plots (px at screen res)
    private double exportAxisFontSize = 12.0; // axis title / tick font size for scatter plots (px at screen res)
    private int exportPointSize = 0;          // pseudocolor/dot point radius in pixels (0 = 1px)
    private boolean exportGateLabels = true;  // show gate names + stats in scatter-plot export
    // Analysis-chart (AnalysisChart) specific — applied at export DPI (scaled by dpi/96)
    private double chartTitleFontSize = 14.0; // title line font size (px at screen res)
    private double chartLegendFontSize = 11.0; // legend label + key swatch font size (px at screen res)

    // App-wide format: changing any of these notifies every open plot so Copy is consistent everywhere.
    private final java.util.List<Runnable> listeners = new java.util.ArrayList<>();
    public void addChangeListener(Runnable r) { listeners.add(r); }
    public void removeChangeListener(Runnable r) { listeners.remove(r); }
    private void fireChanged() { for (Runnable r : new java.util.ArrayList<>(listeners)) { try { r.run(); } catch (Exception ignored) {} } }

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
