package org.streamflow.ui;

import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.layout.Region;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;
import javafx.scene.text.FontWeight;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * A lightweight, native, white-background analysis chart for the auto-analysis modules
 * (cell cycle, proliferation, …). Renders a shared X grid with any number of overlaid
 * series — histogram bars, filled areas, or lines — and is INTERACTIVE (toggle series,
 * set the X range) and EXPORTABLE (high-DPI PNG snapshot + SVG). It deliberately stays
 * simple: data is a common {@code x[]} plus per-series {@code y[]}, so every module can
 * reuse it and the user gets a restylable, publication-ready plot instead of a static PNG.
 */
public final class AnalysisChart extends Region {

    public enum Kind { BARS, AREA, LINE, BAND }

    public static final class Series {
        final String name; final double[] y; double[] yHi; Color color; final Kind kind;
        boolean dashed; boolean visible = true;
        Series(String name, double[] y, Color color, Kind kind) {
            this.name = name; this.y = y; this.color = color; this.kind = kind;
        }
    }

    private final Canvas canvas = new Canvas();
    private double[] x = new double[0];
    private final Map<String, Series> series = new LinkedHashMap<>();
    private String title = "", xLabel = "", yLabel = "Count";
    private double xMin = 0, xMax = 1;           // current (zoomable) X view
    private double xDataMin = 0, xDataMax = 1;   // full data extent

    // optional draggable threshold (e.g. a compensation positive/negative split)
    private double thresholdX = Double.NaN;
    private boolean thresholdEnabled = false;
    private boolean draggingThreshold = false;
    private java.util.function.Consumer<Double> onThresholdChange;
    private double lastLeft = 60, lastPw = 1;    // captured each redraw for pixel<->data mapping
    private boolean legendVisible = true;        // hide on compact charts where it overlaps bars

    // Draggable G1/G2 peak anchors (cell cycle) — dashed vertical lines the user can nudge
    private double peakG1 = Double.NaN, peakG2 = Double.NaN;
    private boolean peaksEnabled = false;
    private boolean draggingPeak1 = false, draggingPeak2 = false;
    private java.util.function.Consumer<double[]> onPeaksChanged;

    // Drag-to-zoom selection box + double-click to reset
    private boolean zoomBoxEnabled = false;
    private double zoomBoxStartX = Double.NaN, zoomBoxCurrentX = Double.NaN;
    private boolean inZoomDrag = false;

    private static final double ML = 60, MR = 16, MT = 30, MB = 46;

    public AnalysisChart() {
        getChildren().add(canvas);
        widthProperty().addListener((o, a, b) -> redraw());
        heightProperty().addListener((o, a, b) -> redraw());
        setMinSize(200, 160);
        canvas.setOnMousePressed(e -> {
            double mx = e.getX();
            if (thresholdEnabled && Math.abs(mx - sx(thresholdX, lastLeft, lastPw)) < 8) {
                draggingThreshold = true; return;
            }
            if (peaksEnabled) {
                if (Math.abs(mx - sx(peakG1, lastLeft, lastPw)) < 8) { draggingPeak1 = true; return; }
                if (Math.abs(mx - sx(peakG2, lastLeft, lastPw)) < 8) { draggingPeak2 = true; return; }
            }
            if (zoomBoxEnabled) {
                if (e.getClickCount() == 2) { resetZoom(); return; }
                zoomBoxStartX = mx; inZoomDrag = false;
            }
        });
        canvas.setOnMouseDragged(e -> {
            if (draggingThreshold) { thresholdX = dataXAt(e.getX()); redraw(); return; }
            if (draggingPeak1) {
                peakG1 = dataXAt(e.getX());
                peakG2 = peakG1 * 2;
                redraw(); return;
            }
            if (draggingPeak2) {
                peakG2 = dataXAt(e.getX());
                peakG1 = peakG2 / 2;
                redraw(); return;
            }
            if (zoomBoxEnabled && !Double.isNaN(zoomBoxStartX)) {
                inZoomDrag = true;
                zoomBoxCurrentX = e.getX();
                redraw();
            }
        });
        canvas.setOnMouseReleased(e -> {
            if (draggingThreshold) {
                draggingThreshold = false;
                if (onThresholdChange != null) onThresholdChange.accept(thresholdX);
                return;
            }
            if (draggingPeak1 || draggingPeak2) {
                draggingPeak1 = draggingPeak2 = false;
                if (onPeaksChanged != null) onPeaksChanged.accept(new double[]{peakG1, peakG2});
                return;
            }
            if (zoomBoxEnabled && inZoomDrag && !Double.isNaN(zoomBoxStartX) && !Double.isNaN(zoomBoxCurrentX)) {
                double x1 = dataXAt(Math.min(zoomBoxStartX, zoomBoxCurrentX));
                double x2 = dataXAt(Math.max(zoomBoxStartX, zoomBoxCurrentX));
                if (x2 - x1 > (xDataMax - xDataMin) * 0.01) { xMin = x1; xMax = x2; }
            }
            inZoomDrag = false;
            zoomBoxStartX = zoomBoxCurrentX = Double.NaN;
            redraw();
        });
        canvas.setOnMouseMoved(e -> {
            double mx = e.getX();
            if (thresholdEnabled && Math.abs(mx - sx(thresholdX, lastLeft, lastPw)) < 8) {
                canvas.setCursor(javafx.scene.Cursor.H_RESIZE); return;
            }
            if (peaksEnabled && (!Double.isNaN(peakG1) && Math.abs(mx - sx(peakG1, lastLeft, lastPw)) < 8
                    || !Double.isNaN(peakG2) && Math.abs(mx - sx(peakG2, lastLeft, lastPw)) < 8)) {
                canvas.setCursor(javafx.scene.Cursor.H_RESIZE); return;
            }
            canvas.setCursor(zoomBoxEnabled ? javafx.scene.Cursor.CROSSHAIR : javafx.scene.Cursor.DEFAULT);
        });
    }

    /** Set draggable G1/G2 peak lines; {@code onChange} fires with [g1, g2] after the user releases. */
    public void setPeaks(double g1, double g2, java.util.function.Consumer<double[]> onChange) {
        this.peakG1 = g1; this.peakG2 = g2;
        this.peaksEnabled = !Double.isNaN(g1);
        this.onPeaksChanged = onChange;
        redraw();
    }

    /** Hide the peak anchor lines. */
    public void clearPeaks() { peaksEnabled = false; peakG1 = peakG2 = Double.NaN; redraw(); }

    /** Enable drag-to-zoom (drag a box, double-click to reset). */
    public void setZoomBoxEnabled(boolean b) { this.zoomBoxEnabled = b; }

    /** Reset the X view to the full data range. */
    public void resetZoom() {
        xMin = xDataMin; xMax = xDataMax;
        inZoomDrag = false; zoomBoxStartX = zoomBoxCurrentX = Double.NaN;
        redraw();
    }

    /** Show or hide the series legend (hide on small charts where it overlaps the bars). */
    public void setLegendVisible(boolean b) { this.legendVisible = b; redraw(); }

    /** Show a draggable vertical threshold at data-X {@code x}; NaN hides it. */
    public void setThreshold(double x) { this.thresholdX = x; this.thresholdEnabled = !Double.isNaN(x); redraw(); }
    /** Notified with the new data-X when the user finishes dragging the threshold. */
    public void setOnThresholdChange(java.util.function.Consumer<Double> c) { this.onThresholdChange = c; }
    private double dataXAt(double px) {
        double t = xMin + (px - lastLeft) / Math.max(1, lastPw) * (xMax - xMin);
        return Math.max(xMin, Math.min(xMax, t));
    }

    // ---- data API ------------------------------------------------------------

    public void clearSeries() { series.clear(); }

    public void setX(double[] x) {
        this.x = x;
        if (x.length > 0) {
            xDataMin = x[0]; xDataMax = x[x.length - 1];
            xMin = xDataMin; xMax = xDataMax;
        }
    }

    public Series addSeries(String name, double[] y, Color color, Kind kind) {
        Series s = new Series(name, y, color, kind);
        series.put(name, s);
        return s;
    }

    /** Add a shaded band between {@code yLo} and {@code yHi} (e.g. mean±SD). */
    public Series addBandSeries(String name, double[] yLo, double[] yHi, Color color) {
        Series s = new Series(name, yLo, color, Kind.BAND);
        s.yHi = yHi;
        series.put(name, s);
        return s;
    }

    public void setTitle(String t) { this.title = t == null ? "" : t; }
    public void setAxisLabels(String x, String y) { this.xLabel = x; this.yLabel = y; }

    public void setVisible(String name, boolean v) { Series s = series.get(name); if (s != null) { s.visible = v; redraw(); } }
    public void setColor(String name, Color c) { Series s = series.get(name); if (s != null) { s.color = c; redraw(); } }

    /** Set the visible X range as a fraction (0..1) of the full data extent (for an X-zoom slider). */
    public void setXMaxFraction(double frac) {
        frac = Math.max(0.1, Math.min(1.0, frac));
        xMax = xDataMin + frac * (xDataMax - xDataMin);
        if (xMax <= xMin) xMax = xMin + 1e-6;
        redraw();
    }

    public void refresh() { redraw(); }

    // ---- layout + rendering --------------------------------------------------

    @Override protected void layoutChildren() {
        canvas.setWidth(getWidth());
        canvas.setHeight(getHeight());
        redraw();
    }

    private void redraw() {
        double W = getWidth(), H = getHeight();
        if (W <= 0 || H <= 0) return;
        GraphicsContext g = canvas.getGraphicsContext2D();
        // Draw static content, then add interactive overlays on top.
        drawTo(g, W, H, ML, MR, MT, MB, 13, 11, 9, 10);

        // Interactive overlays — only on the live canvas, never in the export render.
        double left = ML, top = MT, pw = Math.max(1, W - ML - MR), ph = Math.max(1, H - MT - MB);
        lastLeft = left; lastPw = pw;

        if (thresholdEnabled && !Double.isNaN(thresholdX)) {
            double tx = sx(thresholdX, left, pw);
            g.setStroke(Color.web("#00B4D8")); g.setLineWidth(1.6); g.setLineDashes(5, 4);
            g.strokeLine(tx, top, tx, top + ph);
            g.setLineDashes((double[]) null);
            g.setFill(Color.web("#00B4D8"));
            g.fillPolygon(new double[]{tx - 5, tx + 5, tx}, new double[]{top, top, top + 9}, 3);
        }
        if (peaksEnabled) {
            drawPeakLine(g, peakG1, "#2C7FB8", "G1", left, top, ph, pw);
            drawPeakLine(g, peakG2, "#D7301F", "G2", left, top, ph, pw);
        }
        if (inZoomDrag && !Double.isNaN(zoomBoxStartX) && !Double.isNaN(zoomBoxCurrentX)) {
            double bx1 = Math.min(zoomBoxStartX, zoomBoxCurrentX);
            double bx2 = Math.max(zoomBoxStartX, zoomBoxCurrentX);
            g.setFill(new Color(0.1, 0.5, 0.9, 0.12));
            g.fillRect(bx1, top, bx2 - bx1, ph);
            g.setStroke(Color.web("#1a8fe3")); g.setLineWidth(1.0); g.setLineDashes((double[]) null);
            g.strokeRect(bx1, top, bx2 - bx1, ph);
        }
    }

    /**
     * Full chart render to any GraphicsContext with explicit margins and font sizes.
     * Used by {@link #redraw()} for on-screen display and by {@link #exportImage} for
     * off-screen publication renders where fonts must be sized for the target DPI.
     */
    private void drawTo(GraphicsContext g, double W, double H,
                        double mL, double mR, double mT, double mB,
                        double titleFs, double axisFs, double tickFs, double legendFs) {
        g.clearRect(0, 0, W, H);
        g.setFill(Color.WHITE); g.fillRect(0, 0, W, H);

        double left = mL, top = mT, pw = Math.max(1, W - mL - mR), ph = Math.max(1, H - mT - mB);

        double yMax = 1e-9;
        for (Series s : series.values()) {
            if (!s.visible) continue;
            for (int i = 0; i < s.y.length; i++) if (within(i)) yMax = Math.max(yMax, s.y[i]);
            if (s.yHi != null) for (int i = 0; i < s.yHi.length; i++) if (within(i)) yMax = Math.max(yMax, s.yHi[i]);
        }
        yMax *= 1.08;

        // title
        g.setFill(Color.web("#1A1A1A"));
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, titleFs));
        g.fillText(title, left, mT * 0.7);

        // axes frame
        g.setStroke(Color.web("#666666")); g.setLineWidth(Math.max(1, titleFs * 0.07));
        g.strokeRect(left, top, pw, ph);

        // series
        for (Series s : series.values()) {
            if (!s.visible) continue;
            if (s.kind == Kind.BARS) {
                g.setFill(s.color);
                double bw = pw / Math.max(1, countWithin());
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), h = s.y[i] / yMax * ph;
                    g.fillRect(px - bw / 2, top + ph - h, Math.max(1, bw), h);
                }
            } else if (s.kind == Kind.AREA) {
                g.setFill(translucent(s.color, 0.45));
                g.beginPath(); boolean started = false;
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), py = top + ph - s.y[i] / yMax * ph;
                    if (!started) { g.moveTo(px, top + ph); g.lineTo(px, py); started = true; }
                    else g.lineTo(px, py);
                }
                if (started) { g.lineTo(sxLastWithin(left, pw), top + ph); g.closePath(); g.fill(); }
            } else if (s.kind == Kind.BAND) {
                g.setFill(translucent(s.color, 0.22));
                g.beginPath(); boolean started = false;
                for (int i = 0; i < x.length && i < (s.yHi != null ? s.yHi.length : 0); i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), py = top + ph - s.yHi[i] / yMax * ph;
                    if (!started) { g.moveTo(px, py); started = true; } else g.lineTo(px, py);
                }
                if (started) {
                    for (int i = x.length - 1; i >= 0; i--) {
                        if (!within(i) || i >= s.y.length) continue;
                        g.lineTo(sx(x[i], left, pw), top + ph - Math.max(0, s.y[i]) / yMax * ph);
                    }
                    g.closePath(); g.fill();
                }
            } else { // LINE
                g.setStroke(s.color); g.setLineWidth(Math.max(1, titleFs * 0.12));
                g.setLineDashes(s.dashed ? new double[]{titleFs * 0.5, titleFs * 0.3} : null);
                g.beginPath(); boolean started = false;
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), py = top + ph - s.y[i] / yMax * ph;
                    if (!started) { g.moveTo(px, py); started = true; } else g.lineTo(px, py);
                }
                g.stroke(); g.setLineDashes((double[]) null);
            }
        }

        // axis labels + ticks
        g.setFill(Color.web("#222222"));
        g.setFont(Font.font("Segoe UI", axisFs));
        g.fillText(xLabel, left + pw / 2 - xLabel.length() * axisFs * 0.28, top + ph + mB * 0.76);
        g.save(); g.translate(mL * 0.25, top + ph / 2 + yLabel.length() * axisFs * 0.28); g.rotate(-90);
        g.fillText(yLabel, 0, 0); g.restore();
        g.setFont(Font.font("Segoe UI", tickFs)); g.setFill(Color.web("#555555"));
        for (int k = 0; k <= 4; k++) {
            double xv = xMin + k / 4.0 * (xMax - xMin);
            g.fillText(fmt(xv), sx(xv, left, pw) - tickFs * 1.3, top + ph + tickFs + mB * 0.12);
            double yv = k / 4.0 * yMax;
            g.fillText(fmt(yv), 2, top + ph - k / 4.0 * ph + tickFs * 0.35);
        }

        // legend (top-right corner) with key swatch that scales with legendFs
        if (legendVisible) {
            double swW = legendFs * 1.1, swH = legendFs * 0.9;
            double legW = 20 + swW + series.values().stream()
                    .filter(s -> s.visible)
                    .mapToDouble(s -> s.name.length() * legendFs * 0.62)
                    .max().orElse(100);
            double ly = top + legendFs * 0.6, lx = left + pw - legW;
            g.setFont(Font.font("Segoe UI", legendFs));
            for (Series s : series.values()) {
                if (!s.visible) continue;
                g.setFill(s.color); g.fillRect(lx, ly - swH * 0.8, swW, swH);
                g.setFill(Color.web("#222222")); g.fillText(s.name, lx + swW + legendFs * 0.35, ly + 1);
                ly += legendFs * 1.45;
            }
        }
    }

    private void drawPeakLine(GraphicsContext g, double dataX, String hex, String label,
                               double left, double top, double ph, double pw) {
        if (Double.isNaN(dataX) || dataX < xMin || dataX > xMax) return;
        double px = sx(dataX, left, pw);
        g.setStroke(Color.web(hex)); g.setLineWidth(1.5); g.setLineDashes(4, 3);
        g.strokeLine(px, top, px, top + ph);
        g.setLineDashes((double[]) null);
        g.setFill(Color.web(hex));
        g.fillPolygon(new double[]{px - 5, px + 5, px}, new double[]{top - 1, top - 1, top + 9}, 3);
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 10));
        g.fillText(label, px + 4, top + 16);
    }

    private boolean within(int i) { return i < x.length && x[i] >= xMin && x[i] <= xMax; }
    private int countWithin() { int c = 0; for (double v : x) if (v >= xMin && v <= xMax) c++; return Math.max(1, c); }
    private double sx(double xv, double left, double pw) {
        return left + (xv - xMin) / (xMax - xMin) * pw;
    }
    private double sxLastWithin(double left, double pw) {
        double last = xMin; for (double v : x) if (v >= xMin && v <= xMax) last = v;
        return sx(last, left, pw);
    }
    private static Color translucent(Color c, double a) { return new Color(c.getRed(), c.getGreen(), c.getBlue(), a); }

    private static String fmt(double v) {
        double a = Math.abs(v);
        if (a >= 100000) return String.format("%.0e", v);
        if (a >= 1000) return String.format("%.0f", v);
        if (a >= 1) return String.format("%.0f", v);
        return String.format("%.2f", v);
    }

    // ---- export --------------------------------------------------------------

    /**
     * Publication-quality render to an off-screen canvas using AppSettings font sizes scaled to
     * the target DPI. Fonts are drawn at the correct point size for the output resolution rather
     * than being bitmap-upscaled, so titles, axis labels and the legend are all crisp and legible.
     */
    public javafx.scene.image.WritableImage exportImage(AppSettings settings) {
        double dpi   = settings.exportDpi();
        double scale = Math.max(1.0, dpi / 96.0);
        int W = (int) Math.round(getWidth()  * scale);
        int H = (int) Math.round(getHeight() * scale);
        if (W <= 0 || H <= 0) return null;

        double titleFs  = settings.chartTitleFontSize()  * scale;
        double axisFs   = settings.exportAxisFontSize()  * scale;
        double tickFs   = Math.max(7 * scale, axisFs - 4 * scale);
        double legendFs = settings.chartLegendFontSize() * scale;
        // Margins scale so axis labels + tick rows always have room at the target size
        double mL = Math.max(ML * scale, 18 + axisFs + tickFs * 2.8);
        double mR = MR * scale;
        double mT = Math.max(MT * scale, titleFs + 6);
        double mB = Math.max(MB * scale, axisFs + tickFs + 14 * scale);

        javafx.scene.canvas.Canvas off = new javafx.scene.canvas.Canvas(W, H);
        drawTo(off.getGraphicsContext2D(), W, H, mL, mR, mT, mB, titleFs, axisFs, tickFs, legendFs);
        javafx.scene.image.WritableImage img = new javafx.scene.image.WritableImage(W, H);
        off.snapshot(null, img);
        return img;
    }

    /** Legacy convenience — use {@link #exportImage(AppSettings)} for new callers. */
    public javafx.scene.image.WritableImage snapshotAtDpi(int dpi) {
        double scale = Math.max(1.0, dpi / 96.0);
        javafx.scene.SnapshotParameters sp = new javafx.scene.SnapshotParameters();
        sp.setTransform(javafx.scene.transform.Transform.scale(scale, scale));
        int w = (int) Math.round(getWidth() * scale), h = (int) Math.round(getHeight() * scale);
        if (w <= 0 || h <= 0) return null;
        return snapshot(sp, new javafx.scene.image.WritableImage(w, h));
    }

    /** Minimal vector SVG of the current view (bars + lines + axes), for editable export. */
    public String toSvg() {
        double W = getWidth(), H = getHeight();
        double left = ML, top = MT, pw = W - ML - MR, ph = H - MT - MB;
        double yMax = 1e-9;
        for (Series s : series.values()) if (s.visible) for (int i = 0; i < s.y.length; i++) if (within(i)) yMax = Math.max(yMax, s.y[i]);
        yMax *= 1.08;
        StringBuilder b = new StringBuilder();
        b.append(String.format("<svg xmlns='http://www.w3.org/2000/svg' width='%.0f' height='%.0f'>", W, H));
        b.append(String.format("<rect width='%.0f' height='%.0f' fill='white'/>", W, H));
        b.append(String.format("<text x='%.0f' y='18' font-family='sans-serif' font-size='13' font-weight='bold'>%s</text>", left, esc(title)));
        b.append(String.format("<rect x='%.1f' y='%.1f' width='%.1f' height='%.1f' fill='none' stroke='#666'/>", left, top, pw, ph));
        for (Series s : series.values()) {
            if (!s.visible) continue;
            String col = web(s.color);
            if (s.kind == Kind.BARS) {
                double bw = pw / countWithin();
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), hh = s.y[i] / yMax * ph;
                    b.append(String.format("<rect x='%.1f' y='%.1f' width='%.1f' height='%.1f' fill='%s'/>", px - bw / 2, top + ph - hh, Math.max(1, bw), hh));
                }
            } else {
                StringBuilder pts = new StringBuilder();
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    pts.append(String.format("%.1f,%.1f ", sx(x[i], left, pw), top + ph - s.y[i] / yMax * ph));
                }
                String dash = s.dashed ? " stroke-dasharray='6,4'" : "";
                String fill = s.kind == Kind.AREA ? col : "none";
                String op = s.kind == Kind.AREA ? " fill-opacity='0.45'" : "";
                b.append(String.format("<polyline points='%s' fill='%s'%s stroke='%s' stroke-width='1.6'%s/>", pts.toString().trim(), fill, op, col, dash));
            }
        }
        b.append("</svg>");
        return b.toString();
    }

    private static String web(Color c) {
        return String.format("#%02X%02X%02X", (int) (c.getRed() * 255), (int) (c.getGreen() * 255), (int) (c.getBlue() * 255));
    }
    private static String esc(String s) { return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"); }
}
