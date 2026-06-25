package org.streamflow.ui;

/**
 * Session-wide user settings (Settings window → Export tab). Held in {@link AppContext}.
 * Screen rendering is ~96 DPI; {@link #exportScale()} scales snapshots up for publication output.
 */
public final class AppSettings {

    private int exportDpi = 300;   // publication default
    private double exportFontSize = 12.0;  // point size for axis labels in export
    private boolean exportGateLabels = true;  // show gate names + stats in export

    public int exportDpi() { return exportDpi; }
    public void setExportDpi(int dpi) { exportDpi = Math.max(72, Math.min(1200, dpi)); }

    public double exportFontSize() { return exportFontSize; }
    public void setExportFontSize(double size) { exportFontSize = Math.max(6, Math.min(36, size)); }

    public boolean exportGateLabels() { return exportGateLabels; }
    public void setExportGateLabels(boolean b) { exportGateLabels = b; }

    /** Snapshot scale factor for the configured DPI (relative to 96-DPI screen space). */
    public double exportScale() { return exportDpi / 96.0; }
}
