package org.streamflow.ui;

import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.layout.Region;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;
import javafx.scene.text.FontWeight;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
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

    public enum Kind { BARS, AREA, LINE }

    public static final class Series {
        final String name; final double[] y; Color color; final Kind kind;
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

    private static final double ML = 60, MR = 16, MT = 30, MB = 46;

    public AnalysisChart() {
        getChildren().add(canvas);
        widthProperty().addListener((o, a, b) -> redraw());
        heightProperty().addListener((o, a, b) -> redraw());
        setMinSize(200, 160);
        canvas.setOnMousePressed(e -> {
            if (thresholdEnabled && Math.abs(e.getX() - sx(thresholdX, lastLeft, lastPw)) < 8) draggingThreshold = true;
        });
        canvas.setOnMouseDragged(e -> { if (draggingThreshold) { thresholdX = dataXAt(e.getX()); redraw(); } });
        canvas.setOnMouseReleased(e -> {
            if (draggingThreshold) { draggingThreshold = false; if (onThresholdChange != null) onThresholdChange.accept(thresholdX); }
        });
        canvas.setOnMouseMoved(e -> {
            if (thresholdEnabled) canvas.setCursor(Math.abs(e.getX() - sx(thresholdX, lastLeft, lastPw)) < 8
                    ? javafx.scene.Cursor.H_RESIZE : javafx.scene.Cursor.DEFAULT);
        });
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
        g.clearRect(0, 0, W, H);
        // white publication background
        g.setFill(Color.WHITE); g.fillRect(0, 0, W, H);

        double left = ML, top = MT, pw = Math.max(1, W - ML - MR), ph = Math.max(1, H - MT - MB);

        // y range from visible series (and histogram)
        double yMax = 1e-9;
        for (Series s : series.values())
            if (s.visible) for (int i = 0; i < s.y.length; i++)
                if (within(i)) yMax = Math.max(yMax, s.y[i]);
        yMax *= 1.08;

        // title
        g.setFill(Color.web("#1A1A1A"));
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 13));
        g.fillText(title, left, 18);

        // axes frame
        g.setStroke(Color.web("#666666")); g.setLineWidth(1);
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
                g.beginPath();
                boolean started = false;
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), py = top + ph - s.y[i] / yMax * ph;
                    if (!started) { g.moveTo(px, top + ph); g.lineTo(px, py); started = true; }
                    else g.lineTo(px, py);
                }
                if (started) { g.lineTo(sxLastWithin(left, pw), top + ph); g.closePath(); g.fill(); }
            } else { // LINE
                g.setStroke(s.color); g.setLineWidth(1.8);
                g.setLineDashes(s.dashed ? new double[]{6, 4} : null);
                g.beginPath();
                boolean started = false;
                for (int i = 0; i < x.length && i < s.y.length; i++) {
                    if (!within(i)) continue;
                    double px = sx(x[i], left, pw), py = top + ph - s.y[i] / yMax * ph;
                    if (!started) { g.moveTo(px, py); started = true; } else g.lineTo(px, py);
                }
                g.stroke();
                g.setLineDashes((double[]) null);
            }
        }

        // axis labels + ticks
        g.setFill(Color.web("#222222"));
        g.setFont(Font.font("Segoe UI", 11));
        g.fillText(xLabel, left + pw / 2 - xLabel.length() * 3.0, top + ph + 34);
        g.save(); g.translate(14, top + ph / 2 + yLabel.length() * 3.0); g.rotate(-90);
        g.fillText(yLabel, 0, 0); g.restore();
        g.setFont(Font.font("Segoe UI", 9)); g.setFill(Color.web("#555555"));
        for (int k = 0; k <= 4; k++) {
            double xv = xMin + k / 4.0 * (xMax - xMin);
            g.fillText(fmt(xv), sx(xv, left, pw) - 12, top + ph + 15);
            double yv = k / 4.0 * yMax;
            g.fillText(fmt(yv), 26, top + ph - k / 4.0 * ph + 3);
        }

        // draggable threshold line (compensation positive/negative split)
        lastLeft = left; lastPw = pw;
        if (thresholdEnabled && !Double.isNaN(thresholdX)) {
            double tx = sx(thresholdX, left, pw);
            g.setStroke(Color.web("#00B4D8")); g.setLineWidth(1.6); g.setLineDashes(5, 4);
            g.strokeLine(tx, top, tx, top + ph);
            g.setLineDashes((double[]) null);
            g.setFill(Color.web("#00B4D8"));
            g.fillPolygon(new double[]{tx - 5, tx + 5, tx}, new double[]{top, top, top + 9}, 3);
        }

        // legend (top-right) — optional, since on small/compact charts it can overlap the bars
        if (legendVisible) {
            double ly = top + 6, lx = left + pw - 150;
            g.setFont(Font.font("Segoe UI", 10));
            for (Series s : series.values()) {
                if (!s.visible) continue;
                g.setFill(s.color); g.fillRect(lx, ly - 8, 12, 10);
                g.setFill(Color.web("#222222")); g.fillText(s.name, lx + 16, ly + 1);
                ly += 15;
            }
        }
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

    /** High-resolution PNG snapshot at the given DPI (96 = on-screen) for publication export. */
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
