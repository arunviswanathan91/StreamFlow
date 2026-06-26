package org.streamflow.ui;

import javafx.application.Platform;
import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.control.ContextMenu;
import javafx.scene.control.Label;
import javafx.scene.control.MenuItem;
import javafx.scene.control.ProgressIndicator;
import javafx.scene.control.SeparatorMenuItem;
import javafx.scene.layout.Pane;
import javafx.scene.image.PixelFormat;
import javafx.scene.image.WritableImage;
import javafx.scene.input.ContextMenuEvent;
import javafx.scene.input.MouseButton;
import javafx.scene.input.MouseEvent;
import javafx.scene.layout.Region;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;

import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.function.Consumer;

/**
 * Native FlowJo-style plot + gating canvas (no engine round-trips). Renders a
 * sample's held {@link EventData} as pseudocolor density / dot / histogram for any
 * channel pair, owns its data↔pixel mapping, and hosts interactive gate drawing
 * (polygon / rectangle / ellipse / interval) with instant in-Java membership so
 * counts and drill-down are immediate.
 */
public class CytoPlot extends Region {

    /** A gate in DATA coordinates. For interval/rectangle/ellipse xs/ys hold extents. */
    public static final class Gate {
        public String name, type, xChan, yChan;
        public double[] xs, ys;
        public Color border = GATE_C;                 // outline colour
        public Color fill = Color.color(GATE_C.getRed(), GATE_C.getGreen(), GATE_C.getBlue(), 0.12); // translucent interior
        public double lblDx = 0, lblDy = -4;          // label offset (pixels) from its anchor
        public double angle = 0;                      // ellipse rotation (radians, data-space CCW from +X)
        public boolean invert = false;                // exclusion ("NOT") gate: keep events OUTSIDE the shape
        // statistics shown on the label: keys "parent","total","count","mfi:<chan>","geomean","cv"
        public final java.util.List<String> statKeys = new java.util.ArrayList<>(java.util.List.of("parent", "count"));
        public String statLine = "";                  // formatted stats (set by the controller)
        public Gate(String name, String type, String xChan, String yChan, double[] xs, double[] ys) {
            this.name = name; this.type = type; this.xChan = xChan; this.yChan = yChan;
            this.xs = xs; this.ys = ys;
        }
    }

    private static final double ML = 64, MR = 14, MT = 14, MB = 46; // plot margins
    private static final Color GATE_C = Color.web("#D7261E");
    private static final Color DRAW_C = Color.web("#0A7CFF");
    private static final Color[] LUT = buildLut();

    private final Canvas canvas = new Canvas();
    private final Pane overlay = new Pane();        // hosts interactive gate Labels above the canvas
    private final java.util.Map<Gate, Label> gateLabels = new java.util.IdentityHashMap<>();
    private final ProgressIndicator spinner = new ProgressIndicator();
    private EventData data;
    private String xChan, yChan, plotType = "pseudocolor";
    private double xmin, xmax, ymin, ymax;
    private WritableImage plotImg;          // cached density/dot raster at plot-rect size

    // ---- async rendering (heavy event binning never runs on the FX thread) ---
    private final ExecutorService renderExec = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "cyto-render"); t.setDaemon(true); return t;
    });
    private Future<?> renderTask;
    private double[] histHeights;           // cached normalised histogram (0..1), null for 2D
    private double histMaxCount = 0;        // raw peak count of the current histogram (for Y-axis labels)
    private Consumer<Boolean> onBusy;       // notify controller to disable axis controls while busy
    private boolean smooth = true;          // KDE/blur smoothing toggle
    private boolean lightExport = false;     // white bg + dark axes for publication export
    private String histMode = "Filled Smooth"; // "Filled Smooth" | "Raw Bars" | "Line Only"
    private double histBandwidth = 0.5;     // 0..1 KDE bandwidth fraction

    /** Per-axis display scale (data is stored raw; mapping goes raw -> scaled -> pixel). */
    public enum Scale { LINEAR, LOG, ARCSINH, LOGICLE }
    private static final double LG_M = 4.5;       // logicle decades
    private Scale xScale = Scale.LINEAR, yScale = Scale.LINEAR;
    private double xCof = 150, yCof = 150;        // arcsinh cofactors
    private double xW = -1, yW = -1;             // logicle width request (<0 = auto from data)
    private double xWeff = 0.5, yWeff = 0.5;     // effective logicle width actually used
    private Logicle xLogicle, yLogicle;          // built per range when scale == LOGICLE
    private double xMaxOv = Double.NaN, yMaxOv = Double.NaN; // display-max overrides (axis zoom)
    private double xMinOv = Double.NaN, yMinOv = Double.NaN; // display-min overrides (range extend)
    private double sxMin, sxMax, syMin, syMax;   // scaled-space ranges

    private final List<Gate> gates = new ArrayList<>();
    private String tool = "None";
    private final List<double[]> polyPx = new ArrayList<>();
    private double[] dragStart, dragCur;
    private Consumer<Gate> onGateDrawn;

    // ---- live gate stats while drawing (differentiator #2) ------------------
    private int[] liveIdx;                    // subsample of row indices for fast live counting
    private String liveStatText = "";         // "~N (X.X%)" shown beside the in-progress gate
    private static final int LIVE_SAMPLE = 40000;

    // ---- density-valley gate snapping (differentiator #1) -------------------
    private double[] valleyXs, valleyYs;      // scaled-space positions of marginal density valleys
    private boolean snapEnabled = false;      // magnetic snap of gate edges/vertices to valleys (opt-in)
    private static final double SNAP_PX = 12; // snap radius in pixels
    private static final Color SNAP_C = Color.web("#FF2EC4"); // ghost guide colour

    // ---- gate confidence border (differentiator #4) -------------------------
    private double[] confGrid;                 // coarse 2D density in scaled space (CG×CG)
    private double confMax;                    // max density in confGrid
    private static final int CG = 128;         // confidence-grid resolution
    private boolean showConfidence = false;    // draw the per-gate confidence dot (opt-in)

    // ---- backgating highlight (differentiator #9) ---------------------------
    private boolean[] highlightMask;           // rows (over current data) to overlay as dots
    private static final Color BACKGATE_C = Color.web("#FF8C00"); // orange overlay
    private static final int BACKGATE_CAP = 20000;                // max dots drawn

    // ---- FMO reference line (differentiator #5) -----------------------------
    private double fmoX = Double.NaN, fmoY = Double.NaN;  // data-space FMO p95 for current axes
    private boolean fmoVisible = true;                    // hide FMO lines without losing the stored level
    private static final Color FMO_C = Color.web("#7B1FA2"); // purple dashed reference

    // ---- editing state (tool == "None") -------------------------------------
    private enum Drag { NONE, MOVE_GATE, MOVE_VERTEX, ROTATE_GATE, MOVE_FMO_X, MOVE_FMO_Y, MOVE_QUADRANT }
    private Gate selected;
    private Drag dragMode = Drag.NONE;
    private int dragVertex = -1;
    private double[] editPressPx;            // pixel position at drag start
    private double[][] editOrigPx;           // gate vertices in pixel space at drag start
    private Consumer<Gate> onGateChanged, onGateDeleted, onRenameRequest, onColorRequest, onOpenChild, onStatsConfig, onHistoryRequest;
    private Consumer<Gate> onGateEditStart;  // fired with old state before any drag edit begins
    private double rotateDragCenterPxX, rotateDragCenterPxY;
    private java.util.function.BiConsumer<Double, Double> onFmoChanged;   // (x,y) when an FMO line is dragged
    /** Notified with the new (x,y) data-space FMO levels when the user drags an FMO line. */
    public void setOnFmoChanged(java.util.function.BiConsumer<Double, Double> c) { this.onFmoChanged = c; }
    private boolean labelsVisible = true;
    private static final double HANDLE = 4.5; // node handle radius (px)
    private static final double ROT_HANDLE_OFF = 20.0; // extra px past semi-major axis for rotation knob

    public CytoPlot() {
        getChildren().add(canvas);
        canvas.setOnMouseMoved(e -> {
            dragCur = new double[]{e.getX(), e.getY()};
            if (!polyPx.isEmpty()) { updateLiveStat(); paint(); }
            else if ("Quadrant".equals(tool)) paint();
        });
        canvas.setOnMousePressed(this::onPressed);
        canvas.setOnMouseDragged(this::onDragged);
        canvas.setOnMouseReleased(this::onReleased);
        canvas.setOnMouseClicked(this::onClicked);
        canvas.setOnContextMenuRequested(this::onContextMenu);
        overlay.setPickOnBounds(false);          // only the Labels catch mouse events, not the Pane
        getChildren().add(overlay);
        spinner.setVisible(false);
        spinner.setMaxSize(28, 28);
        spinner.setMouseTransparent(true);
        getChildren().add(spinner);
        widthProperty().addListener((o, a, b) -> { canvas.setWidth(getWidth()); invalidate(); });
        heightProperty().addListener((o, a, b) -> { canvas.setHeight(getHeight()); invalidate(); });
    }

    @Override protected void layoutChildren() {
        canvas.setWidth(getWidth());
        canvas.setHeight(getHeight());
        overlay.resizeRelocate(0, 0, getWidth(), getHeight());
        spinner.resizeRelocate(plotLeft() + plotW() - 36, plotTop() + 8, 28, 28); // top-right of plot area
    }

    // ---- public API ---------------------------------------------------------

    public void setData(EventData d) { this.data = d; liveIdx = null; liveStatText = ""; valleyXs = null; valleyYs = null; highlightMask = null; invalidate(); }
    public void setSnapEnabled(boolean b) { this.snapEnabled = b; paint(); }
    public boolean snapEnabled() { return snapEnabled; }

    /** Overlay the given rows (over the CURRENT data) as orange backgating dots; null clears. */
    public void setHighlight(boolean[] mask) { this.highlightMask = mask; paint(); }
    public boolean hasHighlight() { return highlightMask != null; }

    /** FMO reference levels (data space) for the current X / Y channels; NaN hides each line. */
    public void setFmo(double x, double y) { this.fmoX = x; this.fmoY = y; paint(); }
    /** Show/hide FMO reference lines without clearing the stored level (#4). */
    public void setFmoVisible(boolean b) { this.fmoVisible = b; paint(); }
    public boolean isFmoVisible() { return fmoVisible; }
    public void setOnGateDrawn(Consumer<Gate> c) { this.onGateDrawn = c; }
    public void setOnGateEditStart(Consumer<Gate> c) { this.onGateEditStart = c; }
    /** Toggle live light background (same as export mode but applied to live rendering). */
    public void setLightMode(boolean b) { this.lightExport = b; invalidate(); }
    /** Show/hide gate labels on the plot overlay. */
    public void setLabelsVisible(boolean b) { this.labelsVisible = b; paint(); }
    public void setTool(String t) { this.tool = t; selected = null; resetDrawing(); }
    public List<Gate> gates() { return gates; }

    public void setAxes(String x, String y) {
        this.xChan = x; this.yChan = y; invalidate();
    }
    public void setPlotType(String t) { this.plotType = t; invalidate(); }
    public boolean isHistogram() { return yChan == null || "histogram".equals(plotType); }

    /** Apply axes + scales + plot type in one shot so a change re-renders only once. */
    public void setView(String x, String y, Scale xs, Scale ys, String type) {
        this.xChan = x; this.yChan = y; this.xScale = xs; this.yScale = ys; this.plotType = type;
        invalidate();
    }
    public void setOnBusy(Consumer<Boolean> c) { this.onBusy = c; }
    public void setSmooth(boolean s) { this.smooth = s; invalidate(); }
    public boolean smooth() { return smooth; }
    public void setHistMode(String m) { this.histMode = m; invalidate(); }
    public void setHistBandwidth(double b) { this.histBandwidth = b; if (isHistogram()) invalidate(); }
    public double histBandwidth() { return histBandwidth; }

    public void addGate(Gate g) { gates.add(g); paint(); }
    public void removeGate(String name) { gates.removeIf(g -> g.name.equals(name)); paint(); }
    public void clearGates() { gates.clear(); selected = null; paint(); }

    /** Membership of a gate over an arbitrary population's events (for the gating tree). */
    public static boolean[] mask(EventData d, Gate g) {
        boolean[] keep = new boolean[d.rows()];
        int xc = d.indexOf(g.xChan);
        int yc = g.yChan == null ? -1 : d.indexOf(g.yChan);
        for (int r = 0; r < d.rows(); r++) {
            double x = d.get(r, xc);
            double y = yc < 0 ? 0 : d.get(r, yc);
            keep[r] = pointInGate(g, x, y) ^ g.invert;   // exclusion gate flips membership
        }
        return keep;
    }

    // ---- selection / editing callbacks (§5–§7) ------------------------------
    public void setOnGateChanged(Consumer<Gate> c) { this.onGateChanged = c; }
    public void setOnGateDeleted(Consumer<Gate> c) { this.onGateDeleted = c; }
    public void setOnRenameRequest(Consumer<Gate> c) { this.onRenameRequest = c; }
    public void setOnOpenChild(Consumer<Gate> c) { this.onOpenChild = c; }
    public void setOnStatsConfig(Consumer<Gate> c) { this.onStatsConfig = c; }
    public void setOnHistoryRequest(Consumer<Gate> c) { this.onHistoryRequest = c; }
    public void setChannelLabeler(java.util.function.Function<String, String> f) { this.channelLabeler = f; }
    private java.util.function.Function<String, String> channelLabeler;
    private String chLabel(String ch) { return (channelLabeler == null || ch == null) ? ch : channelLabeler.apply(ch); }
    public void setOnColorRequest(Consumer<Gate> c) { this.onColorRequest = c; }
    public Gate selectedGate() { return selected; }
    public void selectGate(Gate g) { selected = g; paint(); }

    /** Insert a vertex into a polygon gate — on the edge nearest the click (or the longest edge). */
    public void addNodeToPolygon(Gate g, double localX, double localY) {
        if (!"polygon".equals(g.type)) return;
        int n = g.xs.length, after = 0;
        if (!Double.isNaN(localX)) {
            double best = Double.MAX_VALUE;
            for (int i = 0; i < n; i++) {
                int j = (i + 1) % n;
                double d = ptSegDist(localX, localY, pxX(g.xs[i]), pxY(g.ys[i]), pxX(g.xs[j]), pxY(g.ys[j]));
                if (d < best) { best = d; after = i; }
            }
        } else {
            double best = -1;
            for (int i = 0; i < n; i++) {
                int j = (i + 1) % n;
                double d = Math.hypot(g.xs[i] - g.xs[j], g.ys[i] - g.ys[j]);
                if (d > best) { best = d; after = i; }
            }
        }
        int j = (after + 1) % n;
        double mx = (g.xs[after] + g.xs[j]) / 2, my = (g.ys[after] + g.ys[j]) / 2;
        double[] nxs = new double[n + 1], nys = new double[n + 1];
        for (int i = 0, k = 0; i < n; i++) {
            nxs[k] = g.xs[i]; nys[k] = g.ys[i]; k++;
            if (i == after) { nxs[k] = mx; nys[k] = my; k++; }
        }
        g.xs = nxs; g.ys = nys;
        selected = g; paint();
        if (onGateChanged != null) onGateChanged.accept(g);
    }

    private static double ptSegDist(double px, double py, double x1, double y1, double x2, double y2) {
        double dx = x2 - x1, dy = y2 - y1, len2 = dx * dx + dy * dy;
        double t = len2 == 0 ? 0 : ((px - x1) * dx + (py - y1) * dy) / len2;
        t = Math.max(0, Math.min(1, t));
        double cx = x1 + t * dx, cy = y1 + t * dy;
        return Math.hypot(px - cx, py - cy);
    }

    /** Convert a rectangle/ellipse gate to an editable polygon (keeps it; no delete). */
    public void convertToPolygon(Gate g) {
        // Only rectangle/ellipse convert cleanly; polygon/interval/quadrant are left as-is
        // (a quadrant's single centre point has no closed region to trace).
        if (!"rectangle".equals(g.type) && !"ellipse".equals(g.type)) return;
        double xlo = min(g.xs), xhi = max(g.xs), ylo = min(g.ys), yhi = max(g.ys);
        if ("rectangle".equals(g.type)) {
            g.xs = new double[]{xlo, xhi, xhi, xlo};
            g.ys = new double[]{ylo, ylo, yhi, yhi};
        } else { // ellipse -> 32-point polygon (apply rotation if any)
            double cx = (xlo + xhi) / 2, cy = (ylo + yhi) / 2, rx = (xhi - xlo) / 2, ry = (yhi - ylo) / 2;
            double cosA = Math.cos(g.angle), sinA = Math.sin(g.angle);
            int n = 32; double[] xs = new double[n], ys = new double[n];
            for (int k = 0; k < n; k++) {
                double a = 2 * Math.PI * k / n;
                double lx = rx * Math.cos(a), ly = ry * Math.sin(a);
                xs[k] = cx + lx * cosA - ly * sinA;
                ys[k] = cy + lx * sinA + ly * cosA;
            }
            g.xs = xs; g.ys = ys;
        }
        g.type = "polygon";
        paint();
        if (onGateChanged != null) onGateChanged.accept(g);
    }

    /** Delete the selected gate (Delete key / context menu); notifies the controller. */
    public void deleteSelected() {
        if (selected == null) return;
        Gate g = selected;
        gates.remove(g);
        selected = null;
        paint();
        if (onGateDeleted != null) onGateDeleted.accept(g);
    }

    /** Boolean membership over the current data rows for a gate (drill-down + counts). */
    public boolean[] membership(Gate g) {
        boolean[] keep = new boolean[data.rows()];
        int xc = data.indexOf(g.xChan);
        int yc = g.yChan == null ? -1 : data.indexOf(g.yChan);
        for (int r = 0; r < data.rows(); r++) {
            double x = data.get(r, xc);
            double y = yc < 0 ? 0 : data.get(r, yc);
            keep[r] = pointInGate(g, x, y) ^ g.invert;   // exclusion gate flips membership
        }
        return keep;
    }

    public int count(Gate g) {
        boolean[] m = membership(g); int n = 0; for (boolean b : m) if (b) n++; return n;
    }

    /** EventData restricted to the gated rows (for drill-down windows). */
    public EventData dataSubset(boolean[] keep) {
        return data == null ? null : data.subset(keep);
    }

    /** Cancel any in-progress gate drawing cleanly (Escape). */
    public void cancelDrawing() { resetDrawing(); }

    /** Vector SVG of the current view: white bg, axis frame + labels, gates and gate labels.
     *  (The density cloud itself is raster — use the PNG snapshot for that.) */
    public String exportSvg() {
        int w = (int) getWidth(), h = (int) getHeight();
        double px = plotLeft(), py = plotTop(), pw = plotW(), ph = plotH();
        StringBuilder s = new StringBuilder();
        s.append("<svg xmlns='http://www.w3.org/2000/svg' xmlns:xlink='http://www.w3.org/1999/xlink' width='")
                .append(w).append("' height='").append(h).append("' font-family='Segoe UI, sans-serif'>");
        s.append("<rect width='100%' height='100%' fill='white'/>");
        // embed the density raster (inherently raster) so the SVG isn't blank; gates/axes stay vector
        if (!isHistogram() && plotImg != null) {
            String b64 = pngBase64(plotImg);
            if (b64 != null) s.append("<image x='").append(px).append("' y='").append(py)
                    .append("' width='").append(pw).append("' height='").append(ph)
                    .append("' preserveAspectRatio='none' xlink:href='data:image/png;base64,").append(b64).append("'/>");
        }
        s.append("<rect x='").append(px).append("' y='").append(py).append("' width='").append(pw)
                .append("' height='").append(ph).append("' fill='none' stroke='#888' stroke-width='1'/>");
        if (xChan != null) s.append(svgText(px + pw / 2 - 30, py + ph + 34, chLabel(xChan), "#333"));
        if (!isHistogram() && yChan != null) s.append(svgText(16, py + ph / 2, yChan, "#333"));
        for (Gate gt : gates) {
            if (!visible(gt)) continue;
            String col = web(gt.border);
            switch (gt.type) {
                case "rectangle" -> {
                    double x = pxX(min(gt.xs)), y = pxY(max(gt.ys));
                    s.append("<rect x='").append(x).append("' y='").append(y).append("' width='")
                            .append(pxX(max(gt.xs)) - x).append("' height='").append(pxY(min(gt.ys)) - y)
                            .append("' fill='none' stroke='").append(col).append("' stroke-width='1.6'/>");
                }
                case "ellipse" -> {
                    double ecx = (pxX(min(gt.xs)) + pxX(max(gt.xs))) / 2, ecy = (pxY(min(gt.ys)) + pxY(max(gt.ys))) / 2;
                    double erx = Math.abs(pxX(max(gt.xs)) - pxX(min(gt.xs))) / 2;
                    double ery = Math.abs(pxY(min(gt.ys)) - pxY(max(gt.ys))) / 2;
                    String rotStr = gt.angle != 0 ? String.format(" transform='rotate(%.1f %.1f %.1f)'",
                            -Math.toDegrees(gt.angle), ecx, ecy) : "";
                    s.append("<ellipse cx='").append(ecx).append("' cy='").append(ecy)
                            .append("' rx='").append(erx).append("' ry='").append(ery)
                            .append(rotStr)
                            .append("' fill='none' stroke='").append(col).append("' stroke-width='1.6'/>");
                }
                case "polygon" -> {
                    StringBuilder pts = new StringBuilder();
                    for (int k = 0; k < gt.xs.length; k++) pts.append(pxX(gt.xs[k])).append(',').append(pxY(gt.ys[k])).append(' ');
                    s.append("<polygon points='").append(pts).append("' fill='none' stroke='").append(col).append("' stroke-width='1.6'/>");
                }
                case "interval" -> {
                    double a = pxX(min(gt.xs)), b = pxX(max(gt.xs));
                    s.append(svgLine(a, plotTop(), a, plotTop() + ph, col)).append(svgLine(b, plotTop(), b, plotTop() + ph, col));
                }
                case "q1" -> {
                    double cx = pxX(gt.xs[0]), cy = pxY(gt.ys[0]);
                    s.append(svgLine(cx, plotTop(), cx, plotTop() + ph, col));
                    s.append(svgLine(px, cy, px + pw, cy, col));
                }
            }
            double[] a = labelAnchorPx(gt);
            s.append(svgText(a[0] + 3 + gt.lblDx, a[1] + 10 + gt.lblDy, gt.name == null ? "" : gt.name, col));
        }
        s.append("</svg>");
        return s.toString();
    }
    private static String svgText(double x, double y, String t, String col) {
        return "<text x='" + x + "' y='" + y + "' fill='" + col + "' font-size='12'>" +
                t.replace("&", "&amp;").replace("<", "&lt;") + "</text>";
    }
    private static String svgLine(double x1, double y1, double x2, double y2, String col) {
        return "<line x1='" + x1 + "' y1='" + y1 + "' x2='" + x2 + "' y2='" + y2 + "' stroke='" + col + "' stroke-width='1.6'/>";
    }
    private static String web(Color c) {
        return String.format("#%02X%02X%02X", (int) (c.getRed() * 255), (int) (c.getGreen() * 255), (int) (c.getBlue() * 255));
    }
    private static String pngBase64(WritableImage img) {
        try {
            java.awt.image.BufferedImage bi = javafx.embed.swing.SwingFXUtils.fromFXImage(img, null);
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            javax.imageio.ImageIO.write(bi, "png", out);
            return java.util.Base64.getEncoder().encodeToString(out.toByteArray());
        } catch (Exception e) { return null; }
    }

    /** Publication snapshot: white background + dark axes, rendered at {@code scale}× (for DPI). */
    public WritableImage exportImage(double scale) {
        boolean prev = lightExport;
        lightExport = true; paint();
        javafx.scene.SnapshotParameters sp = new javafx.scene.SnapshotParameters();
        sp.setFill(Color.WHITE);
        sp.setTransform(javafx.scene.transform.Transform.scale(scale, scale));
        WritableImage img = snapshot(sp, null);
        lightExport = prev; paint();
        return img;
    }

    // ---- axis scales --------------------------------------------------------

    public void setXScale(Scale s) { xScale = s; invalidate(); }
    public void setYScale(Scale s) { yScale = s; invalidate(); }
    public Scale xScale() { return xScale; }
    public Scale yScale() { return yScale; }

    /** Logicle linearization width (W): explicit value, or <0 to auto-estimate from data. */
    public void setXWidth(double w) { xW = w; if (xScale == Scale.LOGICLE) { invalidate(); } }
    public void setYWidth(double w) { yW = w; if (yScale == Scale.LOGICLE) { invalidate(); } }
    public double xWidth() { return xWeff; }
    public double yWidth() { return yWeff; }

    /** ArcSinh cofactor (larger = more low-end compression). */
    public void setXCof(double c) { xCof = c; if (xScale == Scale.ARCSINH) { invalidate(); } }
    public void setYCof(double c) { yCof = c; if (yScale == Scale.ARCSINH) { invalidate(); } }
    public double xCof() { return xCof; }
    public double yCof() { return yCof; }

    /** Axis display range overrides (zoom / extend); NaN restores the data range. */
    public void setXMax(double m) { xMaxOv = m; invalidate(); }
    public void setYMax(double m) { yMaxOv = m; invalidate(); }
    public void setXMin(double m) { xMinOv = m; invalidate(); }
    public void setYMin(double m) { yMinOv = m; invalidate(); }
    public void resetXRange() { xMaxOv = Double.NaN; xMinOv = Double.NaN; invalidate(); }
    public void resetYRange() { yMaxOv = Double.NaN; yMinOv = Double.NaN; invalidate(); }
    public double xMin() { return xmin; }
    public double yMin() { return ymin; }
    public double xMax() { return xmax; }
    public double yMax() { return ymax; }
    public double xDataMax() { return (data == null || xChan == null) ? 1 : data.range(data.indexOf(xChan))[1]; }
    public double yDataMax() { return (data == null || yChan == null) ? 1 : data.range(data.indexOf(yChan))[1]; }

    private double sxv(double v) { return xScale == Scale.LOGICLE ? xLogicle.scale(v) : scaleVal(v, xScale, xCof); }
    private double syv(double v) { return yScale == Scale.LOGICLE ? yLogicle.scale(v) : scaleVal(v, yScale, yCof); }

    /** Effective logicle width, auto-estimated from the data minimum when not set. */
    private static double effW(double dataMin, double dataMax, double req) {
        double T = Math.max(dataMax, 1.0);
        double w = req >= 0 ? req : (LG_M - Math.log10(T / Math.abs(Math.min(dataMin, -1)))) / 2;
        if (!Double.isFinite(w)) w = 0.5;
        return Math.max(0.05, Math.min(LG_M / 2 - 0.01, w));
    }
    private static double scaleVal(double v, Scale s, double c) {
        switch (s) {
            case LOG:     return Math.log10(Math.max(v, 1.0));
            case ARCSINH: return asinh(v / c);
            default:      return v;
        }
    }
    private static double unscaleVal(double t, Scale s, double c) {
        switch (s) {
            case LOG:     return Math.pow(10, t);
            case ARCSINH: return c * Math.sinh(t);
            default:      return t;
        }
    }
    private static double asinh(double x) { return Math.log(x + Math.sqrt(x * x + 1)); }

    // ---- rendering ----------------------------------------------------------

    // Keep the data area SQUARE so maximising never stretches the cloud on one axis.
    private double plotSide() { return Math.max(1, Math.min(getWidth() - ML - MR, getHeight() - MT - MB)); }
    private double plotW() { return plotSide(); }
    private double plotH() { return plotSide(); }
    // Anchor the square plot at the top-left so the Y-axis labels (drawn in the ML margin) always sit
    // right next to the data area — centring it left a large gap between the axis and the plot.
    private double plotLeft() { return ML; }
    private double plotTop() { return MT; }
    private double pxX(double dx) { return plotLeft() + (sxv(dx) - sxMin) / (sxMax - sxMin) * plotW(); }
    private double pxY(double dy) { return plotTop() + (syMax - syv(dy)) / (syMax - syMin) * plotH(); }
    private double dataX(double px) {
        double t = sxMin + (px - plotLeft()) / plotW() * (sxMax - sxMin);
        return xScale == Scale.LOGICLE ? xLogicle.inverse(t) : unscaleVal(t, xScale, xCof);
    }
    private double dataY(double py) {
        double t = syMax - (py - plotTop()) / plotH() * (syMax - syMin);
        return yScale == Scale.LOGICLE ? yLogicle.inverse(t) : unscaleVal(t, yScale, yCof);
    }

    // last Logicle build params, so we rebuild the 16k-entry LUT only when T or W actually change
    private double xLogT = Double.NaN, xLogW = Double.NaN, yLogT = Double.NaN, yLogW = Double.NaN;

    private void recomputeRanges() {
        if (data == null || xChan == null) return;
        // Guard the mid-swap state: setData() invalidates synchronously while xChan may still hold a
        // previous view's channel that isn't in the new data — skip until the matching setView lands.
        int xc = data.indexOf(xChan);
        if (xc < 0) return;
        double[] xr = data.range(xc);
        xmin = Double.isNaN(xMinOv) ? xr[0] : xMinOv; xmax = Double.isNaN(xMaxOv) ? xr[1] : xMaxOv;
        if (xScale == Scale.LOGICLE) {
            xWeff = effW(xmin, xmax, xW);
            double T = Math.max(xmax, 1);
            if (xLogicle == null || T != xLogT || xWeff != xLogW) {   // reuse the LUT when unchanged
                xLogicle = new Logicle(T, xWeff, LG_M, 0); xLogT = T; xLogW = xWeff;
            }
        }
        sxMin = sxv(xmin); sxMax = sxv(xmax); if (sxMax <= sxMin) sxMax = sxMin + 1;
        if (!isHistogram()) {
            int yc = data.indexOf(yChan);
            if (yc < 0) return;
            double[] yr = data.range(yc);
            ymin = Double.isNaN(yMinOv) ? yr[0] : yMinOv; ymax = Double.isNaN(yMaxOv) ? yr[1] : yMaxOv;
            if (yScale == Scale.LOGICLE) {
                yWeff = effW(ymin, ymax, yW);
                double T = Math.max(ymax, 1);
                if (yLogicle == null || T != yLogT || yWeff != yLogW) {
                    yLogicle = new Logicle(T, yWeff, LG_M, 0); yLogT = T; yLogW = yWeff;
                }
            }
            syMin = syv(ymin); syMax = syv(ymax); if (syMax <= syMin) syMax = syMin + 1;
        }
    }

    // ---- async render pipeline (heavy event binning off the FX thread) ------

    private long renderGen = 0;
    private static final int WHITE_ARGB = 0xFFFFFFFF;
    private static final int[] LUT_ARGB = buildLutArgb();
    private static final Color HIST_FG = Color.web("#1F6FEB");

    /** Recompute ranges (cheap) then kick a background raster/histogram build. */
    private void invalidate() {
        recomputeRanges();
        scheduleCompute();
    }

    /** Cancel any in-flight render and start a fresh one; FX thread stays free. */
    private void scheduleCompute() {
        if (renderTask != null) renderTask.cancel(true);
        final long gen = ++renderGen;
        if (data == null || xChan == null) { plotImg = null; histHeights = null; paint(); return; }
        int w = (int) plotW(), h = (int) plotH();
        if (w < 2 || h < 2) { paint(); return; }
        final boolean hist = isHistogram();
        final int fw = w, fh = h;
        setBusy(true);
        renderTask = renderExec.submit(() -> {
            try {
                double[][] valleys = computeValleys();   // marginal density minima for snapping
                if (Thread.currentThread().isInterrupted()) return;
                if (hist) {
                    double[] heights = computeHistogram(256);
                    if (heights == null || Thread.currentThread().isInterrupted()) return;
                    Platform.runLater(() -> { if (gen == renderGen) {
                        histHeights = heights; plotImg = null; applyValleys(valleys); setBusy(false); paint(); } });
                } else {
                    int[] argb = computeScatter(fw, fh);
                    if (argb == null || Thread.currentThread().isInterrupted()) return;
                    Platform.runLater(() -> {
                        if (gen != renderGen) return;
                        WritableImage img = new WritableImage(fw, fh);
                        img.getPixelWriter().setPixels(0, 0, fw, fh, PixelFormat.getIntArgbInstance(), argb, 0, fw);
                        plotImg = img; histHeights = null; applyValleys(valleys); setBusy(false); paint();
                    });
                }
            } catch (Exception ignored) { }
        });
    }

    private void setBusy(boolean b) { spinner.setVisible(b); if (onBusy != null) onBusy.accept(b); }

    /** Cheap FX-thread paint from cached raster/histogram + gate overlay. */
    private void paint() {
        GraphicsContext g = canvas.getGraphicsContext2D();
        g.clearRect(0, 0, getWidth(), getHeight());
        g.setFill(lightExport ? Color.WHITE : Color.web("#0D1B2A"));
        g.fillRect(0, 0, getWidth(), getHeight());
        if (data == null || xChan == null) return;

        double px = plotLeft(), py = plotTop(), pw = plotW(), ph = plotH();
        g.setFill(Color.WHITE);
        g.fillRect(px, py, pw, ph);

        if (isHistogram()) {
            drawHistogram(g, px, py, pw, ph);
        } else if (plotImg != null) {
            g.setImageSmoothing("dot".equals(plotType) ? false : true);
            g.drawImage(plotImg, px, py, pw, ph);
        }
        drawAxes(g, px, py, pw, ph);
        drawHighlight(g);
        drawFmo(g);
        drawGates(g);
        drawInProgress(g);
    }

    /** Dashed FMO reference line(s) at the stored 95th-percentile level for the current channel(s). */
    private void drawFmo(GraphicsContext g) {
        if (data == null || !fmoVisible) return;
        double left = plotLeft(), top = plotTop(), w = plotW(), h = plotH();
        g.setStroke(FMO_C); g.setLineWidth(1.4); g.setLineDashes(6, 4);
        g.setFill(FMO_C); g.setFont(Font.font(10));
        if (!Double.isNaN(fmoX)) {
            double x = pxX(fmoX);
            if (x >= left && x <= left + w) {
                g.strokeLine(x, top, x, top + h); g.fillText("FMO", x + 2, top + 11);
                fmoHandle(g, x, top); fmoHandle(g, x, top + h);   // drag handles top + bottom
            }
        }
        if (!isHistogram() && !Double.isNaN(fmoY)) {
            double y = pxY(fmoY);
            if (y >= top && y <= top + h) {
                g.strokeLine(left, y, left + w, y); g.fillText("FMO", left + 2, y - 2);
                fmoHandle(g, left, y); fmoHandle(g, left + w, y);  // drag handles left + right
            }
        }
        g.setLineDashes((double[]) null);
    }

    /** A small solid square handle at an FMO line end, so users see the line is draggable. */
    private void fmoHandle(GraphicsContext g, double x, double y) {
        g.setLineDashes((double[]) null);
        g.fillRect(x - 3, y - 3, 6, 6);
    }

    /** Backgating: draw the highlighted descendant events as orange dots over the current plot. */
    private void drawHighlight(GraphicsContext g) {
        if (highlightMask == null || data == null || isHistogram()) return;
        int xc = data.indexOf(xChan), yc = (yChan == null) ? -1 : data.indexOf(yChan);
        if (xc < 0 || yc < 0) return;
        double left = plotLeft(), top = plotTop(), w = plotW(), h = plotH();
        g.setFill(BACKGATE_C);
        int drawn = 0, rows = Math.min(highlightMask.length, data.rows());
        for (int r = 0; r < rows && drawn < BACKGATE_CAP; r++) {
            if (!highlightMask[r]) continue;
            double x = pxX(data.get(r, xc)), y = pxY(data.get(r, yc));
            if (x < left || x > left + w || y < top || y > top + h) continue;
            g.fillOval(x - 1.4, y - 1.4, 2.8, 2.8);
            drawn++;
        }
    }

    // ---- background compute (no JavaFX, no scene-graph access) ---------------

    /** Bin events into a w×h density grid, then colour per plot type. Returns ARGB. */
    private int[] computeScatter(int w, int h) {
        EventData d = data;
        int xc = d.indexOf(xChan), yc = d.indexOf(yChan);
        double sMinX = sxMin, sRangeX = sxMax - sxMin, sMaxY = syMax, sRangeY = syMax - syMin;
        int[] grid = new int[w * h];
        int max = 0, rows = d.rows();
        for (int r = 0; r < rows; r++) {
            if ((r & 0x3FFF) == 0 && Thread.currentThread().isInterrupted()) return null;
            int px = (int) ((sxv(d.get(r, xc)) - sMinX) / sRangeX * (w - 1));
            int py = (int) ((sMaxY - syv(d.get(r, yc))) / sRangeY * (h - 1));
            if (px < 0 || px >= w || py < 0 || py >= h) continue;
            int c = ++grid[py * w + px];
            if (c > max) max = c;
        }
        String type = plotType;
        int[] argb = new int[w * h];
        java.util.Arrays.fill(argb, WHITE_ARGB);
        if (max == 0) return argb;

        switch (type) {
            case "dot" -> {
                int dotArgb = colorArgb(Color.web("#08306B"));
                for (int p = 0; p < grid.length; p++) if (grid[p] > 0) argb[p] = dotArgb;
            }
            case "density" -> {
                double[] dens = densityField(grid, w, h);
                double dmax = arrayMax(dens);
                for (int p = 0; p < dens.length; p++) {
                    if (dens[p] <= 0) continue;
                    int v = (int) (255 * (1.0 - Math.log(dens[p] + 1) / Math.log(dmax + 1)));
                    v = Math.max(0, Math.min(255, v));
                    argb[p] = 0xFF000000 | (v << 16) | (v << 8) | v;
                }
            }
            case "contour" -> {
                double[] dens = densityField(grid, w, h);
                double[] lv = equalProbLevels(dens, 10);
                int line = colorArgb(Color.web("#0B3D91"));
                for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
                    int b = bandOf(dens[y * w + x], lv);
                    int br = x + 1 < w ? bandOf(dens[y * w + x + 1], lv) : b;
                    int bd = y + 1 < h ? bandOf(dens[(y + 1) * w + x], lv) : b;
                    if (b != br || b != bd) argb[y * w + x] = line;
                }
            }
            case "zebra" -> {
                double[] dens = densityField(grid, w, h);
                int n = 20;
                double[] lv = equalProbLevels(dens, n);
                for (int p = 0; p < dens.length; p++) {
                    if (dens[p] <= 0) continue;
                    int b = bandOf(dens[p], lv);
                    int base = LUT_ARGB[(int) Math.min(255, (double) b / n * 255)];
                    argb[p] = (b % 2 == 0) ? base : shade(base, 0.78); // alternating stripe
                }
            }
            default -> { // pseudocolor
                double logMax = Math.log(max + 1);
                for (int p = 0; p < grid.length; p++) {
                    int c = grid[p];
                    if (c > 0) argb[p] = LUT_ARGB[(int) Math.min(255, Math.log(c + 1) / logMax * 255)];
                }
            }
        }
        return argb;
    }

    /** 1-D histogram over the X channel, optional KDE smoothing; heights normalised 0..1. */
    private double[] computeHistogram(int bins) {
        EventData d = data;
        int xc = d.indexOf(xChan);
        double sMin = sxMin, sRange = sxMax - sxMin;
        double[] h = new double[bins];
        int rows = d.rows();
        for (int r = 0; r < rows; r++) {
            if ((r & 0x3FFF) == 0 && Thread.currentThread().isInterrupted()) return null;
            int b = (int) ((sxv(d.get(r, xc)) - sMin) / sRange * (bins - 1));
            if (b >= 0 && b < bins) h[b]++;
        }
        if (!"Raw Bars".equals(histMode)) {
            int radius = Math.max(1, (int) (histBandwidth * 24)); // bandwidth slider -> blur radius
            h = blur1d(h, radius);
        }
        double max = arrayMax(h);
        histMaxCount = max;                       // keep the peak count for the Y-axis labels
        if (max > 0) for (int i = 0; i < h.length; i++) h[i] /= max;
        return h;
    }

    /** Marginal-density valleys (for snapping) + a coarse 2-D density grid (for gate confidence).
     *  Returns {scaled-X valleys, scaled-Y valleys, flattened CG×CG grid or null}. */
    private double[][] computeValleys() {
        EventData d = data;
        if (d == null || xChan == null) return null;
        int xc = d.indexOf(xChan);
        int yc = (isHistogram() || yChan == null) ? -1 : d.indexOf(yChan);
        final int bins = 256;
        double[] hx = new double[bins], hy = new double[bins];
        double[] cg = yc >= 0 ? new double[CG * CG] : null;
        double sMinX = sxMin, sRangeX = sxMax - sxMin;
        double sMinY = syMin, sRangeY = syMax - syMin;
        int rows = d.rows();
        for (int r = 0; r < rows; r++) {
            if ((r & 0x3FFF) == 0 && Thread.currentThread().isInterrupted()) return null;
            double sxr = sxv(d.get(r, xc));
            int bx = (int) ((sxr - sMinX) / sRangeX * (bins - 1));
            if (bx >= 0 && bx < bins) hx[bx]++;
            if (yc >= 0) {
                double syr = syv(d.get(r, yc));
                int by = (int) ((syr - sMinY) / sRangeY * (bins - 1));
                if (by >= 0 && by < bins) hy[by]++;
                int cx = (int) ((sxr - sMinX) / sRangeX * (CG - 1));
                int cyy = (int) ((syr - sMinY) / sRangeY * (CG - 1));
                if (cx >= 0 && cx < CG && cyy >= 0 && cyy < CG) cg[cyy * CG + cx]++;
            }
        }
        double[] vx = valleysFrom(hx, sMinX, sRangeX);
        double[] vy = yc < 0 ? new double[0] : valleysFrom(hy, sMinY, sRangeY);
        double[] conf = cg == null ? null : blur2d(cg, CG, CG, 2);
        return new double[][]{vx, vy, conf};
    }

    /** Local minima of a smoothed 1-D marginal that sit between two significant peaks
     *  (a real dip, &lt;85% of the lower flanking peak). Returns scaled-space positions. */
    private double[] valleysFrom(double[] h, double sMin, double sRange) {
        double[] s = blur1d(h, 6);
        double max = arrayMax(s);
        if (max <= 0) return new double[0];
        int bins = s.length;
        List<Integer> peaks = new ArrayList<>();
        for (int i = 1; i < bins - 1; i++)
            if (s[i] > s[i - 1] && s[i] >= s[i + 1] && s[i] > max * 0.05) peaks.add(i);
        List<Double> valleys = new ArrayList<>();
        for (int p = 0; p < peaks.size() - 1; p++) {
            int a = peaks.get(p), b = peaks.get(p + 1);
            int vi = a; double vmin = s[a];
            for (int i = a; i <= b; i++) if (s[i] < vmin) { vmin = s[i]; vi = i; }
            double flank = Math.min(s[a], s[b]);
            if (vmin < flank * 0.85)
                valleys.add(sMin + (double) vi / (bins - 1) * sRange);
        }
        double[] out = new double[valleys.size()];
        for (int i = 0; i < out.length; i++) out[i] = valleys.get(i);
        return out;
    }

    private void applyValleys(double[][] v) {
        if (v == null) { valleyXs = null; valleyYs = null; confGrid = null; confMax = 0; return; }
        valleyXs = v[0];
        valleyYs = v[1];
        confGrid = v.length > 2 ? v[2] : null;
        confMax = confGrid == null ? 0 : arrayMax(confGrid);
    }

    /** Mean density along a gate's boundary as a fraction of the peak density (0..1).
     *  A boundary cutting through a dense region (high value) = low-confidence gate.
     *  Returns NaN when no 2-D grid is available (histograms / quadrant gates). */
    private double gateBoundaryConfidence(Gate g) {
        if (confGrid == null || confMax <= 0) return Double.NaN;
        double[][] pts = boundarySamples(g, 64);
        if (pts == null || pts.length == 0) return Double.NaN;
        double sMinX = sxMin, sRangeX = sxMax - sxMin, sMinY = syMin, sRangeY = syMax - syMin;
        double sum = 0; int n = 0;
        for (double[] p : pts) {
            int cx = (int) ((sxv(p[0]) - sMinX) / sRangeX * (CG - 1));
            int cy = (int) ((syv(p[1]) - sMinY) / sRangeY * (CG - 1));
            if (cx < 0 || cx >= CG || cy < 0 || cy >= CG) continue;
            sum += confGrid[cy * CG + cx]; n++;
        }
        return n == 0 ? Double.NaN : (sum / n) / confMax;
    }

    /** Sample points along a gate's perimeter in data space (rectangle/ellipse/polygon). */
    private double[][] boundarySamples(Gate g, int per) {
        switch (g.type) {
            case "rectangle" -> {
                double xlo = min(g.xs), xhi = max(g.xs), ylo = min(g.ys), yhi = max(g.ys);
                double[][] out = new double[per * 4][2];
                for (int k = 0; k < per; k++) {
                    double t = (double) k / per;
                    out[k]           = new double[]{xlo + t * (xhi - xlo), ylo};
                    out[per + k]     = new double[]{xhi, ylo + t * (yhi - ylo)};
                    out[2 * per + k] = new double[]{xhi - t * (xhi - xlo), yhi};
                    out[3 * per + k] = new double[]{xlo, yhi - t * (yhi - ylo)};
                }
                return out;
            }
            case "ellipse" -> {
                double cx = (min(g.xs) + max(g.xs)) / 2, cy = (min(g.ys) + max(g.ys)) / 2;
                double rx = (max(g.xs) - min(g.xs)) / 2, ry = (max(g.ys) - min(g.ys)) / 2;
                double cosA = Math.cos(g.angle), sinA = Math.sin(g.angle);
                double[][] out = new double[per * 4][2];
                for (int k = 0; k < out.length; k++) {
                    double a = 2 * Math.PI * k / out.length;
                    double lx = rx * Math.cos(a), ly = ry * Math.sin(a);
                    out[k] = new double[]{cx + lx * cosA - ly * sinA, cy + lx * sinA + ly * cosA};
                }
                return out;
            }
            case "polygon" -> {
                int m = g.xs.length;
                double[][] out = new double[m * per][2];
                for (int e = 0; e < m; e++) {
                    int j = (e + 1) % m;
                    for (int k = 0; k < per; k++) {
                        double t = (double) k / per;
                        out[e * per + k] = new double[]{
                                g.xs[e] + t * (g.xs[j] - g.xs[e]),
                                g.ys[e] + t * (g.ys[j] - g.ys[e])};
                    }
                }
                return out;
            }
            default -> { return null; }   // interval / quadrant: no 2-D confidence
        }
    }

    /** Confidence colour: low boundary density = green (clean separation), high = red. */
    private static Color confidenceColor(double ratio) {
        if (ratio < 0.15) return Color.web("#2ECC71");
        if (ratio < 0.40) return Color.web("#F1C40F");
        return Color.web("#E74C3C");
    }

    // ---- snap helpers (pixel <-> scaled-valley) -----------------------------
    private double pxFromScaledX(double t) { return plotLeft() + (t - sxMin) / (sxMax - sxMin) * plotW(); }
    private double pxFromScaledY(double t) { return plotTop() + (syMax - t) / (syMax - syMin) * plotH(); }

    /** Snap a pixel-X to the nearest valley column within SNAP_PX; otherwise unchanged. */
    private double snapPxX(double px) {
        if (!snapEnabled || valleyXs == null) return px;
        double best = px, bd = SNAP_PX;
        for (double v : valleyXs) { double vp = pxFromScaledX(v); double dd = Math.abs(vp - px); if (dd < bd) { bd = dd; best = vp; } }
        return best;
    }
    /** Snap a pixel-Y to the nearest valley row within SNAP_PX; otherwise unchanged. */
    private double snapPxY(double px) {
        if (!snapEnabled || valleyYs == null) return px;
        double best = px, bd = SNAP_PX;
        for (double v : valleyYs) { double vp = pxFromScaledY(v); double dd = Math.abs(vp - px); if (dd < bd) { bd = dd; best = vp; } }
        return best;
    }

    private void drawHistogram(GraphicsContext g, double px, double py, double pw, double ph) {
        double[] h = histHeights;
        if (h == null) return;
        int bins = h.length;
        double bw = pw / bins;
        if ("Raw Bars".equals(histMode)) {
            g.setFill(Color.color(HIST_FG.getRed(), HIST_FG.getGreen(), HIST_FG.getBlue(), 0.35));
            g.setStroke(HIST_FG); g.setLineWidth(1);
            for (int b = 0; b < bins; b++) {
                double bh = h[b] * ph;
                g.fillRect(px + b * bw, py + ph - bh, Math.ceil(bw), bh);
            }
            return;
        }
        // Filled Smooth / Line Only: a KDE trace, optionally filled at 25%
        boolean fill = "Filled Smooth".equals(histMode);
        if (fill) {
            g.setFill(Color.color(HIST_FG.getRed(), HIST_FG.getGreen(), HIST_FG.getBlue(), 0.25));
            g.beginPath();
            g.moveTo(px, py + ph);
            for (int b = 0; b < bins; b++) g.lineTo(px + b * bw + bw / 2, py + ph - h[b] * ph);
            g.lineTo(px + pw, py + ph);
            g.closePath(); g.fill();
        }
        g.setStroke(HIST_FG); g.setLineWidth(2);
        g.beginPath();
        for (int b = 0; b < bins; b++) {
            double xx = px + b * bw + bw / 2, yy = py + ph - h[b] * ph;
            if (b == 0) g.moveTo(xx, yy); else g.lineTo(xx, yy);
        }
        g.stroke();
    }

    // ---- compute helpers ----------------------------------------------------

    /** Smooth density field for contour/zebra/density: coarse-bin → Gaussian blur → bilinear
     *  upsample. Coarse binning + interpolation removes the pixel-level noise that made contours
     *  jagged, giving FlowJo-like smooth iso-lines. (Raw pixel grid when Smooth is off.) */
    private double[] densityField(int[] grid, int w, int h) {
        if (!smooth) {
            double[] a = new double[grid.length];
            for (int i = 0; i < grid.length; i++) a[i] = grid[i];
            return a;
        }
        final int G = 128;                                   // coarse resolution
        double[] c = new double[G * G];
        for (int y = 0; y < h; y++) {
            int cy = Math.min(G - 1, y * G / h);
            for (int x = 0; x < w; x++) c[cy * G + Math.min(G - 1, x * G / w)] += grid[y * w + x];
        }
        c = blur2d(c, G, G, 3);                              // stronger blur on the coarse field
        double[] out = new double[w * h];
        for (int y = 0; y < h; y++) {
            double gy = (double) y / h * (G - 1); int y0 = (int) gy, y1 = Math.min(G - 1, y0 + 1); double fy = gy - y0;
            for (int x = 0; x < w; x++) {
                double gx = (double) x / w * (G - 1); int x0 = (int) gx, x1 = Math.min(G - 1, x0 + 1); double fx = gx - x0;
                double v00 = c[y0 * G + x0], v10 = c[y0 * G + x1], v01 = c[y1 * G + x0], v11 = c[y1 * G + x1];
                out[y * w + x] = (v00 * (1 - fx) + v10 * fx) * (1 - fy) + (v01 * (1 - fx) + v11 * fx) * fy;
            }
        }
        return out;
    }

    private static double arrayMax(double[] a) { double m = 0; for (double v : a) if (v > m) m = v; return m; }

    private static int bandOf(double dv, double[] levels) {
        if (dv <= 0) return 0;
        int b = 0; for (double L : levels) if (dv >= L) b++;
        return b;
    }

    /** Density thresholds enclosing equal fractions of the event mass (FlowJo equal-probability). */
    private static double[] equalProbLevels(double[] dens, int n) {
        double[] s = dens.clone();
        java.util.Arrays.sort(s);                 // ascending
        double total = 0; for (double v : s) total += v;
        double[] lv = new double[n];
        if (total <= 0) return lv;
        double acc = 0; int k = 0;
        for (int i = s.length - 1; i >= 0 && k < n; i--) {
            acc += s[i];
            double frac = acc / total;
            while (k < n && frac >= (double) (k + 1) / (n + 1)) lv[k++] = s[i];
        }
        while (k < n) lv[k++] = 0;
        return lv;
    }

    private static double[] blur1d(double[] a, int r) {
        if (r <= 0) return a;
        double[] k = gaussKernel(r);
        double[] out = new double[a.length];
        for (int i = 0; i < a.length; i++) {
            double s = 0;
            for (int j = -r; j <= r; j++) {
                int ii = Math.max(0, Math.min(a.length - 1, i + j));
                s += a[ii] * k[j + r];
            }
            out[i] = s;
        }
        return out;
    }

    private static double[] blur2d(double[] a, int w, int h, int r) {
        double[] k = gaussKernel(r);
        double[] tmp = new double[a.length], out = new double[a.length];
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            double s = 0;
            for (int j = -r; j <= r; j++) { int xx = Math.max(0, Math.min(w - 1, x + j)); s += a[y * w + xx] * k[j + r]; }
            tmp[y * w + x] = s;
        }
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            double s = 0;
            for (int j = -r; j <= r; j++) { int yy = Math.max(0, Math.min(h - 1, y + j)); s += tmp[yy * w + x] * k[j + r]; }
            out[y * w + x] = s;
        }
        return out;
    }

    private static double[] gaussKernel(int r) {
        double[] k = new double[2 * r + 1]; double sig = Math.max(1, r), sum = 0;
        for (int i = -r; i <= r; i++) { k[i + r] = Math.exp(-(i * i) / (2 * sig * sig)); sum += k[i + r]; }
        for (int i = 0; i < k.length; i++) k[i] /= sum;
        return k;
    }

    private static int colorArgb(Color c) {
        return 0xFF000000 | ((int) (c.getRed() * 255) << 16) | ((int) (c.getGreen() * 255) << 8) | (int) (c.getBlue() * 255);
    }
    private static int shade(int argb, double f) {
        int r = (int) (((argb >> 16) & 0xFF) * f), g = (int) (((argb >> 8) & 0xFF) * f), b = (int) ((argb & 0xFF) * f);
        return 0xFF000000 | (r << 16) | (g << 8) | b;
    }
    private static int[] buildLutArgb() {
        int[] a = new int[256];
        for (int i = 0; i < 256; i++) a[i] = colorArgb(LUT[i]);
        return a;
    }

    private void drawAxes(GraphicsContext g, double px, double py, double pw, double ph) {
        Color frame = lightExport ? Color.web("#666666") : Color.web("#B0C4D8");
        Color labelC = lightExport ? Color.web("#222222") : Color.web("#E0E0E0");
        Color tickC = lightExport ? Color.web("#555555") : Color.web("#90A4B8");
        g.setStroke(frame); g.setLineWidth(1);
        g.strokeRect(px, py, pw, ph);
        g.setFill(labelC); g.setFont(Font.font(12));
        g.fillText(chLabel(xChan), px + pw / 2 - 30, py + ph + 34);
        g.save();
        g.translate(16, py + ph / 2 + 30); g.rotate(-90);
        g.fillText(isHistogram() ? "Count" : chLabel(yChan), 0, 0);
        g.restore();
        // scale-aware ticks (decades for log/arcsinh, even for linear). On logicle/log the decades
        // pile up near zero, so drop any label closer than a min pixel gap to the previous one.
        g.setFill(tickC); g.setFont(Font.font(10));
        java.util.List<Double> xt = new ArrayList<>(axisTicks(xmin, xmax, xScale));
        xt.sort(java.util.Comparator.comparingDouble(this::pxX));
        double lastXp = -1e9;
        for (double tv : xt) {
            double xp = pxX(tv);
            if (xp - lastXp < 30) continue;          // labels are wide horizontally
            lastXp = xp;
            g.fillText(fmt(tv), xp - 14, py + ph + 16);
        }
        if (!isHistogram()) {
            java.util.List<Double> yt = new ArrayList<>(axisTicks(ymin, ymax, yScale));
            yt.sort(java.util.Comparator.comparingDouble(this::pxY));
            double lastYp = -1e9;
            for (double tv : yt) {
                double yp = pxY(tv);
                if (yp - lastYp < 14) continue;      // ~one line height vertically
                lastYp = yp;
                g.fillText(fmt(tv), 24, yp + 3);
            }
        } else {
            // Histogram Y axis: label counts from 0 to the peak count.
            for (int k = 0; k <= 4; k++) {
                double frac = k / 4.0;
                g.fillText(fmt(histMaxCount * frac), 24, py + ph - frac * ph + 3);
            }
        }
    }

    private List<Double> axisTicks(double lo, double hi, Scale s) {
        List<Double> t = new ArrayList<>();
        if (s == Scale.LINEAR) {
            for (int k = 0; k <= 4; k++) t.add(lo + k / 4.0 * (hi - lo));
        } else {
            if (lo <= 0 && hi >= 0) t.add(0.0);
            for (int e = 1; e <= 6; e++) {
                double v = Math.pow(10, e);
                if (v >= lo && v <= hi) t.add(v);
                if ((s == Scale.ARCSINH || s == Scale.LOGICLE) && -v >= lo && -v <= hi) t.add(-v);
            }
        }
        return t;
    }

    private void drawGates(GraphicsContext g) {
        g.setFont(Font.font(12));
        for (Gate gt : gates) {
            if (!visible(gt)) continue;
            boolean sel = gt == selected;
            g.setStroke(gt.border); g.setFill(gt.fill); g.setLineWidth(sel ? 2.4 : 1.6);
            switch (gt.type) {
                case "rectangle" -> {
                    double x1 = pxX(min(gt.xs)), x2 = pxX(max(gt.xs));
                    double y1 = pxY(max(gt.ys)), y2 = pxY(min(gt.ys));
                    double rx = Math.min(x1, x2), ry = Math.min(y1, y2), rw = Math.abs(x2 - x1), rh = Math.abs(y2 - y1);
                    g.fillRect(rx, ry, rw, rh); g.strokeRect(rx, ry, rw, rh);
                }
                case "ellipse" -> {
                    double cxPx = (pxX(min(gt.xs)) + pxX(max(gt.xs))) / 2;
                    double cyPx = (pxY(min(gt.ys)) + pxY(max(gt.ys))) / 2;
                    double rxPx = Math.abs(pxX(max(gt.xs)) - pxX(min(gt.xs))) / 2;
                    double ryPx = Math.abs(pxY(max(gt.ys)) - pxY(min(gt.ys))) / 2;
                    if (gt.angle != 0) {
                        g.save();
                        g.translate(cxPx, cyPx);
                        g.rotate(-Math.toDegrees(gt.angle));
                        g.fillOval(-rxPx, -ryPx, 2 * rxPx, 2 * ryPx);
                        g.strokeOval(-rxPx, -ryPx, 2 * rxPx, 2 * ryPx);
                        g.restore();
                    } else {
                        g.fillOval(cxPx - rxPx, cyPx - ryPx, 2 * rxPx, 2 * ryPx);
                        g.strokeOval(cxPx - rxPx, cyPx - ryPx, 2 * rxPx, 2 * ryPx);
                    }
                }
                case "polygon" -> {
                    int n = gt.xs.length; double[] xp = new double[n], yp = new double[n];
                    for (int k = 0; k < n; k++) { xp[k] = pxX(gt.xs[k]); yp[k] = pxY(gt.ys[k]); }
                    g.fillPolygon(xp, yp, n); g.strokePolygon(xp, yp, n);
                }
                case "interval" -> {
                    double a = pxX(min(gt.xs)), b = pxX(max(gt.xs));
                    g.fillRect(Math.min(a, b), plotTop(), Math.abs(b - a), plotH());
                    g.strokeLine(a, plotTop(), a, plotTop() + plotH()); g.strokeLine(b, plotTop(), b, plotTop() + plotH());
                }
                case "line" -> {
                    // Line gate: thin vertical boundary lines only (no filled chunk) — used for
                    // cell-cycle phase dividers. Counts like an interval; renders clean.
                    double a = pxX(min(gt.xs)), b = pxX(max(gt.xs));
                    g.setLineWidth(sel ? 2.4 : 1.6);
                    g.strokeLine(a, plotTop(), a, plotTop() + plotH());
                    g.strokeLine(b, plotTop(), b, plotTop() + plotH());
                }
                case "q1" -> {
                    // Draw the full quadrant crosshair once for the q1 gate; q2/q3/q4 are silent.
                    double cx = pxX(gt.xs[0]), cy = pxY(gt.ys[0]);
                    g.strokeLine(cx, plotTop(), cx, plotTop() + plotH());
                    g.strokeLine(plotLeft(), cy, plotLeft() + plotW(), cy);
                }
            }
            if (showConfidence) drawConfidenceDot(g, gt);
            if (sel) drawHandles(g, gt);
        }
        syncLabels();
    }

    /** Small green/yellow/red dot at the gate's label anchor signalling boundary confidence. */
    private void drawConfidenceDot(GraphicsContext g, Gate gt) {
        double ratio = gateBoundaryConfidence(gt);
        if (Double.isNaN(ratio)) return;
        double[] a = labelAnchorPx(gt);
        double cx = a[0] - 6, cy = a[1] - 2, rr = 3.5;
        g.setFill(confidenceColor(ratio));
        g.fillOval(cx - rr, cy - rr, rr * 2, rr * 2);
        g.setStroke(Color.WHITE); g.setLineWidth(1.0);
        g.strokeOval(cx - rr, cy - rr, rr * 2, rr * 2);
    }

    public void setShowConfidence(boolean b) { this.showConfidence = b; paint(); }
    public boolean showConfidence() { return showConfidence; }

    /** Repaint request from an external animation (e.g. elastic paste). */
    public void refresh() { paint(); }

    /** Offset in DATA space to move a gate's centroid onto the nearest density peak (elastic
     *  templates, #13). Returns {0,0} when there's no 2-D density or no peak nearby. */
    public double[] nearestPeakOffset(Gate g) {
        double[] zero = {0, 0};
        if (confGrid == null || confMax <= 0 || g.xs == null || g.ys == null) return zero;
        double cx = 0, cy = 0; int n = g.xs.length;
        for (int i = 0; i < n; i++) { cx += g.xs[i]; cy += g.ys[i]; }
        cx /= n; cy /= n;
        double sMinX = sxMin, sRangeX = sxMax - sxMin, sMinY = syMin, sRangeY = syMax - syMin;
        int gx = (int) ((sxv(cx) - sMinX) / sRangeX * (CG - 1));
        int gy = (int) ((syv(cy) - sMinY) / sRangeY * (CG - 1));
        int win = CG / 6;                          // search a local neighbourhood for the peak
        int bx = gx, by = gy; double best = -1;
        for (int y = Math.max(0, gy - win); y <= Math.min(CG - 1, gy + win); y++)
            for (int x = Math.max(0, gx - win); x <= Math.min(CG - 1, gx + win); x++) {
                double v = confGrid[y * CG + x];
                if (v > best) { best = v; bx = x; by = y; }
            }
        if (best <= 0) return zero;
        double pSX = sMinX + (double) bx / (CG - 1) * sRangeX;
        double pSY = sMinY + (double) by / (CG - 1) * sRangeY;
        double pX = xScale == Scale.LOGICLE ? xLogicle.inverse(pSX) : unscaleVal(pSX, xScale, xCof);
        double pY = yScale == Scale.LOGICLE ? yLogicle.inverse(pSY) : unscaleVal(pSY, yScale, yCof);
        return new double[]{pX - cx, pY - cy};
    }

    /** Public confidence accessor (0..1, NaN if N/A) so the gating tree can show a matching dot. */
    public double confidenceOf(Gate g) { return gateBoundaryConfidence(g); }

    private boolean visible(Gate g) {
        if (!g.xChan.equals(xChan)) return false;
        return "interval".equals(g.type) || "line".equals(g.type)
                || (yChan != null && g.yChan != null && g.yChan.equals(yChan));
    }

    private double[] labelAnchorPx(Gate g) {
        return switch (g.type) {
            case "polygon"  -> new double[]{pxX(g.xs[0]), pxY(g.ys[0])};
            case "interval" -> new double[]{pxX(min(g.xs)), plotTop()};
            case "line" -> new double[]{(pxX(min(g.xs)) + pxX(max(g.xs))) / 2 - 24, plotTop() + 4};
            case "q1" -> new double[]{pxX(g.xs[0]) + 6, plotTop() + 4};
            case "q2" -> new double[]{plotLeft() + 4, plotTop() + 4};
            case "q3" -> new double[]{plotLeft() + 4, plotTop() + plotH() - 30};
            case "q4" -> new double[]{pxX(g.xs[0]) + 6, plotTop() + plotH() - 30};
            default    -> new double[]{pxX(min(g.xs)), pxY(max(g.ys))};
        };
    }

    /** Reconcile overlay Label nodes with the currently visible gates and reposition them. */
    private void syncLabels() {
        gateLabels.values().forEach(l -> l.setVisible(false));
        for (Gate gt : gates) {
            if (!visible(gt)) continue;
            Label lbl = gateLabels.computeIfAbsent(gt, this::makeGateLabel);
            lbl.setText(labelText(gt));
            lbl.setTextFill(gt.border);
            lbl.setVisible(labelsVisible);
            double[] a = labelAnchorPx(gt);
            lbl.relocate(a[0] + 3 + gt.lblDx, a[1] + gt.lblDy);
        }
        // drop labels whose gate is gone
        gateLabels.entrySet().removeIf(en -> {
            if (!gates.contains(en.getKey())) { overlay.getChildren().remove(en.getValue()); return true; }
            return false;
        });
    }

    /** Label text — name plus configured statistics (one stat line below the name). */
    private String labelText(Gate gt) {
        String nm = gt.name == null ? "" : gt.name;
        return (gt.statLine == null || gt.statLine.isBlank()) ? nm : nm + "\n" + gt.statLine;
    }

    private Label makeGateLabel(Gate gt) {
        Label lbl = new Label();
        lbl.setStyle("-fx-font-size:12; -fx-font-weight:bold; -fx-cursor:hand;");
        final double[] press = new double[4]; // sceneX, sceneY, origDx, origDy
        lbl.setOnMousePressed(e -> {
            selected = gt; paint();
            press[0] = e.getSceneX(); press[1] = e.getSceneY();
            press[2] = gt.lblDx; press[3] = gt.lblDy;
            e.consume();
        });
        lbl.setOnMouseDragged(e -> {
            gt.lblDx = press[2] + (e.getSceneX() - press[0]);
            gt.lblDy = press[3] + (e.getSceneY() - press[1]);
            double[] a = labelAnchorPx(gt);
            lbl.relocate(a[0] + 3 + gt.lblDx, a[1] + gt.lblDy);
            e.consume();
        });
        lbl.setOnMouseClicked(e -> {
            if (e.getClickCount() == 2 && onRenameRequest != null) onRenameRequest.accept(gt);
            e.consume();
        });
        lbl.setOnContextMenuRequested(e -> {
            showGateMenu(gt, lbl, e.getScreenX(), e.getScreenY(), Double.NaN, Double.NaN);
            e.consume();
        });
        overlay.getChildren().add(lbl);
        return lbl;
    }

    private double[][] handlesPx(Gate gt) {
        int n = gt.xs.length;
        double[][] h = new double[n][2];
        double midY = plotTop() + plotH() / 2;
        for (int k = 0; k < n; k++) {
            h[k][0] = pxX(gt.xs[k]);
            h[k][1] = ("interval".equals(gt.type) || "line".equals(gt.type)) ? midY : pxY(gt.ys[k]);
        }
        return h;
    }

    private void drawHandles(GraphicsContext g, Gate gt) {
        g.setFill(Color.WHITE); g.setStroke(gt.border); g.setLineWidth(1.4);
        for (double[] h : handlesPx(gt)) {
            g.fillOval(h[0] - HANDLE, h[1] - HANDLE, HANDLE * 2, HANDLE * 2);
            g.strokeOval(h[0] - HANDLE, h[1] - HANDLE, HANDLE * 2, HANDLE * 2);
        }
        if ("ellipse".equals(gt.type)) {
            double cxPx = (pxX(min(gt.xs)) + pxX(max(gt.xs))) / 2;
            double cyPx = (pxY(min(gt.ys)) + pxY(max(gt.ys))) / 2;
            double rxPx = Math.abs(pxX(max(gt.xs)) - pxX(min(gt.xs))) / 2;
            double rhx = cxPx + (rxPx + ROT_HANDLE_OFF) * Math.cos(gt.angle);
            double rhy = cyPx - (rxPx + ROT_HANDLE_OFF) * Math.sin(gt.angle);
            g.setStroke(gt.border); g.setLineWidth(1.0); g.setLineDashes(3, 3);
            g.strokeLine(cxPx, cyPx, rhx, rhy);
            g.setLineDashes((double[]) null);
            g.setFill(Color.web("#FF9900")); g.setStroke(Color.WHITE); g.setLineWidth(1.4);
            g.fillOval(rhx - HANDLE, rhy - HANDLE, HANDLE * 2, HANDLE * 2);
            g.strokeOval(rhx - HANDLE, rhy - HANDLE, HANDLE * 2, HANDLE * 2);
        }
    }

    /** True if (px,py) hits the orange rotation handle of an ellipse gate. */
    private boolean hitRotationHandle(Gate gt, double px, double py) {
        if (!"ellipse".equals(gt.type)) return false;
        double cxPx = (pxX(min(gt.xs)) + pxX(max(gt.xs))) / 2;
        double cyPx = (pxY(min(gt.ys)) + pxY(max(gt.ys))) / 2;
        double rxPx = Math.abs(pxX(max(gt.xs)) - pxX(min(gt.xs))) / 2;
        double rhx = cxPx + (rxPx + ROT_HANDLE_OFF) * Math.cos(gt.angle);
        double rhy = cyPx - (rxPx + ROT_HANDLE_OFF) * Math.sin(gt.angle);
        return Math.hypot(px - rhx, py - rhy) <= HANDLE + 3;
    }

    private void drawInProgress(GraphicsContext g) {
        g.setStroke(DRAW_C); g.setLineWidth(1.8);
        if ("Polygon".equals(tool) && !polyPx.isEmpty()) {
            for (int k = 1; k < polyPx.size(); k++)
                g.strokeLine(polyPx.get(k - 1)[0], polyPx.get(k - 1)[1], polyPx.get(k)[0], polyPx.get(k)[1]);
            double[] last = polyPx.get(polyPx.size() - 1);
            if (dragCur != null) g.strokeLine(last[0], last[1], dragCur[0], dragCur[1]);
        } else if ("Quadrant".equals(tool) && dragCur != null && inPlot(dragCur[0], dragCur[1])) {
            g.setLineDashes(5, 5);
            g.strokeLine(dragCur[0], plotTop(), dragCur[0], plotTop() + plotH());
            g.strokeLine(plotLeft(), dragCur[1], plotLeft() + plotW(), dragCur[1]);
            g.setLineDashes((double[]) null);
        } else if (dragStart != null && dragCur != null) {
            if ("Ellipse".equals(tool)) {
                // ellipse edges aren't axis-aligned populations — draw raw, no snapping
                double x = Math.min(dragStart[0], dragCur[0]), y = Math.min(dragStart[1], dragCur[1]);
                g.strokeOval(x, y, Math.abs(dragCur[0] - dragStart[0]), Math.abs(dragCur[1] - dragStart[1]));
            } else if ("Rectangle".equals(tool)) {
                double sx0 = snapPxX(dragStart[0]), sx1 = snapPxX(dragCur[0]);
                double sy0 = snapPxY(dragStart[1]), sy1 = snapPxY(dragCur[1]);
                g.strokeRect(Math.min(sx0, sx1), Math.min(sy0, sy1), Math.abs(sx1 - sx0), Math.abs(sy1 - sy0));
                drawSnapGuide(g, sx0, dragStart[0], true);  drawSnapGuide(g, sx1, dragCur[0], true);
                drawSnapGuide(g, sy0, dragStart[1], false); drawSnapGuide(g, sy1, dragCur[1], false);
            } else if ("Interval".equals(tool)) {
                double sx0 = snapPxX(dragStart[0]), sx1 = snapPxX(dragCur[0]);
                g.strokeLine(sx0, plotTop(), sx0, plotTop() + plotH());
                g.strokeLine(sx1, plotTop(), sx1, plotTop() + plotH());
                drawSnapGuide(g, sx0, dragStart[0], true); drawSnapGuide(g, sx1, dragCur[0], true);
            }
        }
        drawLiveStat(g);
    }

    /** Dashed ghost guide showing a valley snap target (only drawn when a snap is active). */
    private void drawSnapGuide(GraphicsContext g, double snapped, double raw, boolean vertical) {
        if (Math.abs(snapped - raw) < 0.5) return;     // no snap happened
        g.setStroke(SNAP_C); g.setLineWidth(1.2); g.setLineDashes(4, 4);
        if (vertical) g.strokeLine(snapped, plotTop(), snapped, plotTop() + plotH());
        else          g.strokeLine(plotLeft(), snapped, plotLeft() + plotW(), snapped);
        g.setLineDashes((double[]) null);
        g.setStroke(DRAW_C); g.setLineWidth(1.8);      // restore for any subsequent strokes
    }

    /** Floating "~N (X%)" badge beside the cursor while a gate is being drawn. */
    private void drawLiveStat(GraphicsContext g) {
        if (liveStatText == null || liveStatText.isEmpty() || dragCur == null) return;
        g.setFont(Font.font(12));
        double tw = liveStatText.length() * 7.2 + 12;
        double bx = Math.min(dragCur[0] + 12, plotLeft() + plotW() - tw);
        double by = Math.max(dragCur[1] - 26, plotTop() + 2);
        g.setFill(Color.color(1, 1, 1, 0.85));
        g.fillRoundRect(bx, by, tw, 18, 6, 6);
        g.setStroke(DRAW_C); g.setLineWidth(1.0);
        g.strokeRoundRect(bx, by, tw, 18, 6, 6);
        g.setFill(Color.web("#08306B"));
        g.fillText(liveStatText, bx + 6, by + 13);
    }

    // ---- gate drawing -------------------------------------------------------

    private boolean inPlot(double x, double y) {
        return x >= plotLeft() && x <= plotLeft() + plotW() && y >= plotTop() && y <= plotTop() + plotH();
    }

    private boolean isDrawingTool() {
        return "Polygon".equals(tool) || "Rectangle".equals(tool)
                || "Ellipse".equals(tool) || "Interval".equals(tool)
                || "Quadrant".equals(tool);
    }

    private void onPressed(MouseEvent e) {
        if (data == null) return;
        double mx = e.getX(), my = e.getY();
        if (isDrawingTool()) {
            if ("Quadrant".equals(tool) && inPlot(mx, my)) {
                finish("quadrant", new double[]{dataX(mx)}, new double[]{dataY(my)});
                return;
            }
            if (inPlot(mx, my) && !"Polygon".equals(tool)) { dragStart = new double[]{mx, my}; dragCur = dragStart; }
            return;
        }
        if (e.getButton() != MouseButton.PRIMARY) return;     // right-click handled by context menu

        // 0a. grab an FMO reference line to reposition it (tight 4px tolerance so gates still win)
        if (fmoVisible && inPlot(mx, my)) {
            if (!Double.isNaN(fmoX) && Math.abs(mx - pxX(fmoX)) <= 4) { dragMode = Drag.MOVE_FMO_X; return; }
            if (!isHistogram() && !Double.isNaN(fmoY) && Math.abs(my - pxY(fmoY)) <= 4) { dragMode = Drag.MOVE_FMO_Y; return; }
        }
        // 0b. grab a quadrant crosshair centre to move all four quadrants together
        Gate q = quadrantGate();
        if (q != null && inPlot(mx, my)
                && Math.hypot(mx - pxX(q.xs[0]), my - pxY(q.ys[0])) <= HANDLE + 4) {
            if (onGateEditStart != null) onGateEditStart.accept(q);
            selected = q; dragMode = Drag.MOVE_QUADRANT; return;
        }

        // 1. grab a handle of the already-selected gate (labels are overlay Nodes now)
        if (selected != null) {
            if (hitRotationHandle(selected, mx, my)) {
                if (onGateEditStart != null) onGateEditStart.accept(selected);
                dragMode = Drag.ROTATE_GATE;
                rotateDragCenterPxX = (pxX(min(selected.xs)) + pxX(max(selected.xs))) / 2;
                rotateDragCenterPxY = (pxY(min(selected.ys)) + pxY(max(selected.ys))) / 2;
                return;
            }
            int hv = hitHandle(selected, mx, my);
            if (hv >= 0) {
                if (onGateEditStart != null) onGateEditStart.accept(selected);
                dragMode = Drag.MOVE_VERTEX; dragVertex = hv; return;
            }
        }
        // 2. click a gate body -> select (then allow move / handle)
        Gate g = gateAt(mx, my);
        if (g != null) {
            selected = g; paint();
            if (hitRotationHandle(g, mx, my)) {
                if (onGateEditStart != null) onGateEditStart.accept(g);
                dragMode = Drag.ROTATE_GATE;
                rotateDragCenterPxX = (pxX(min(g.xs)) + pxX(max(g.xs))) / 2;
                rotateDragCenterPxY = (pxY(min(g.ys)) + pxY(max(g.ys))) / 2;
                return;
            }
            int hv = hitHandle(g, mx, my);
            if (hv >= 0) {
                if (onGateEditStart != null) onGateEditStart.accept(g);
                dragMode = Drag.MOVE_VERTEX; dragVertex = hv; return;
            }
            if (onGateEditStart != null) onGateEditStart.accept(g);
            startGateMove(mx, my);
            return;
        }
        // 3. empty space -> deselect
        if (selected != null) { selected = null; paint(); }
        dragMode = Drag.NONE;
    }

    private void onDragged(MouseEvent e) {
        double mx = e.getX(), my = e.getY();
        if (isDrawingTool()) { if (dragStart != null) { dragCur = new double[]{mx, my}; updateLiveStat(); paint(); } return; }
        switch (dragMode) {
            case MOVE_GATE -> { applyGateMove(mx, my); paint(); }
            case MOVE_VERTEX -> { applyVertexMove(mx, my); paint(); }
            case ROTATE_GATE -> {
                if (selected != null) {
                    selected.angle = Math.atan2(-(my - rotateDragCenterPxY), mx - rotateDragCenterPxX);
                    paint();
                }
            }
            case MOVE_FMO_X -> { fmoX = dataX(clampX(mx)); paint(); }
            case MOVE_FMO_Y -> { fmoY = dataY(clampY(my)); paint(); }
            case MOVE_QUADRANT -> {
                double nx = dataX(clampX(mx)), ny = dataY(clampY(my));
                for (Gate g : gates) if (isQuadrant(g)) { g.xs[0] = nx; g.ys[0] = ny; }
                paint();
            }
            default -> { }
        }
    }

    private double clampX(double px) { return Math.max(plotLeft(), Math.min(plotLeft() + plotW(), px)); }
    private double clampY(double py) { return Math.max(plotTop(), Math.min(plotTop() + plotH(), py)); }
    private static boolean isQuadrant(Gate g) {
        return g.type != null && g.type.length() == 2 && g.type.charAt(0) == 'q' && Character.isDigit(g.type.charAt(1));
    }
    private Gate quadrantGate() { for (Gate g : gates) if (isQuadrant(g)) return g; return null; }

    private void onReleased(MouseEvent e) {
        if (isDrawingTool()) {
            if ("Quadrant".equals(tool)) return;  // finished on press
            if (dragStart == null) return;
            double[] end = {e.getX(), e.getY()};
            if (Math.hypot(end[0] - dragStart[0], end[1] - dragStart[1]) > 3) {
                if ("Rectangle".equals(tool))
                    finish("rectangle", new double[]{dataX(snapPxX(dragStart[0])), dataX(snapPxX(end[0]))},
                            new double[]{dataY(snapPxY(dragStart[1])), dataY(snapPxY(end[1]))});
                else if ("Ellipse".equals(tool))   // ellipse not snapped
                    finish("ellipse", new double[]{dataX(dragStart[0]), dataX(end[0])}, new double[]{dataY(dragStart[1]), dataY(end[1])});
                else if ("Interval".equals(tool))
                    finish("interval", new double[]{dataX(snapPxX(dragStart[0])), dataX(snapPxX(end[0]))}, null);
            }
            dragStart = null; dragCur = null;
            return;
        }
        if ((dragMode == Drag.MOVE_GATE || dragMode == Drag.MOVE_VERTEX || dragMode == Drag.ROTATE_GATE
                || dragMode == Drag.MOVE_QUADRANT)
                && selected != null && onGateChanged != null) {
            onGateChanged.accept(selected);
        }
        if ((dragMode == Drag.MOVE_FMO_X || dragMode == Drag.MOVE_FMO_Y) && onFmoChanged != null) {
            onFmoChanged.accept(fmoX, fmoY);
        }
        dragMode = Drag.NONE;
    }

    private void onClicked(MouseEvent e) {
        if (data == null) return;
        if ("Polygon".equals(tool)) {
            if (e.getClickCount() == 2 && polyPx.size() >= 3) {
                double[] xs = new double[polyPx.size()], ys = new double[polyPx.size()];
                for (int k = 0; k < polyPx.size(); k++) { xs[k] = dataX(polyPx.get(k)[0]); ys[k] = dataY(polyPx.get(k)[1]); }
                resetDrawing();
                finish("polygon", xs, ys);
            } else if (inPlot(e.getX(), e.getY())) {
                polyPx.add(new double[]{snapPxX(e.getX()), snapPxY(e.getY())}); paint();
            }
            return;
        }
        // edit mode: double-click inside a gate body opens it in a new window (child population)
        if (e.getClickCount() == 2) {
            Gate g = gateAt(e.getX(), e.getY());
            if (g != null) { selected = g; paint(); if (onOpenChild != null) onOpenChild.accept(g); }
        }
    }

    private void onContextMenu(ContextMenuEvent e) {
        if (data == null) return;
        Gate g = gateAt(e.getX(), e.getY());
        if (g == null) return;
        selected = g; paint();
        showGateMenu(g, canvas, e.getScreenX(), e.getScreenY(), e.getX(), e.getY());
    }

    /** Shared gate context menu (canvas right-click and label right-click). localX/Y in pixels,
     *  or NaN when invoked from the label (no plot point). */
    private void showGateMenu(Gate g, javafx.scene.Node anchor, double screenX, double screenY,
                              double localX, double localY) {
        selected = g; paint();
        MenuItem open = new MenuItem("Open in new window");
        open.setOnAction(a -> { if (onOpenChild != null) onOpenChild.accept(g); });
        MenuItem rename = new MenuItem("Rename…");
        rename.setOnAction(a -> { if (onRenameRequest != null) onRenameRequest.accept(g); });
        MenuItem toPoly = new MenuItem("Convert to polygon");
        toPoly.setOnAction(a -> convertToPolygon(g));
        toPoly.setDisable("polygon".equals(g.type) || "interval".equals(g.type));
        MenuItem addNode = new MenuItem("Add node");
        addNode.setOnAction(a -> addNodeToPolygon(g, localX, localY));
        addNode.setDisable(!"polygon".equals(g.type));
        MenuItem stats = new MenuItem("Change Statistics Displayed…");
        stats.setOnAction(a -> { if (onStatsConfig != null) onStatsConfig.accept(g); });
        MenuItem color = new MenuItem("Color…");
        color.setOnAction(a -> { if (onColorRequest != null) onColorRequest.accept(g); });
        MenuItem history = new MenuItem("Gate history…");
        history.setOnAction(a -> { if (onHistoryRequest != null) onHistoryRequest.accept(g); });
        MenuItem del = new MenuItem("Delete");
        del.setOnAction(a -> deleteSelected());
        new ContextMenu(open, rename, toPoly, addNode, stats, color, history, new SeparatorMenuItem(), del).show(anchor, screenX, screenY);
    }

    // ---- edit hit-testing & drag application --------------------------------

    private Gate gateAt(double px, double py) {
        if (!inPlot(px, py)) return null;
        double dx = dataX(px), dy = dataY(py);
        for (int i = gates.size() - 1; i >= 0; i--) {
            Gate g = gates.get(i);
            if (g.type.length() == 2 && g.type.charAt(0) == 'q' && Character.isDigit(g.type.charAt(1))) continue;
            if (visible(g) && pointInGate(g, dx, dy)) return g;
        }
        return null;
    }

    private int hitHandle(Gate g, double px, double py) {
        if (!visible(g)) return -1;
        double[][] hs = handlesPx(g);
        for (int k = 0; k < hs.length; k++) {
            if (Math.hypot(px - hs[k][0], py - hs[k][1]) <= HANDLE + 3) return k;
        }
        return -1;
    }

    private void startGateMove(double mx, double my) {
        dragMode = Drag.MOVE_GATE;
        editPressPx = new double[]{mx, my};
        int n = selected.xs.length;
        editOrigPx = new double[n][2];
        for (int k = 0; k < n; k++) {
            editOrigPx[k][0] = pxX(selected.xs[k]);
            editOrigPx[k][1] = selected.ys == null ? 0 : pxY(selected.ys[k]);
        }
    }

    private void applyGateMove(double mx, double my) {
        double dx = mx - editPressPx[0], dy = my - editPressPx[1];
        for (int k = 0; k < selected.xs.length; k++) {
            selected.xs[k] = dataX(editOrigPx[k][0] + dx);
            if (selected.ys != null) selected.ys[k] = dataY(editOrigPx[k][1] + dy);
        }
    }

    private void applyVertexMove(double mx, double my) {
        mx = Math.max(plotLeft(), Math.min(plotLeft() + plotW(), mx));
        my = Math.max(plotTop(), Math.min(plotTop() + plotH(), my));
        selected.xs[dragVertex] = dataX(mx);
        if (selected.ys != null) selected.ys[dragVertex] = dataY(my);
    }


    private void finish(String type, double[] xs, double[] ys) {
        Gate g = new Gate(null, type, xChan, ("interval".equals(type) ? null : yChan), xs, ys);
        if (onGateDrawn != null) onGateDrawn.accept(g);
    }
    private void resetDrawing() { polyPx.clear(); dragStart = null; dragCur = null; liveStatText = ""; paint(); }

    // ---- live gate stats while drawing --------------------------------------

    /** Lazily build a random subsample of row indices for fast live counting. */
    private void ensureLiveIdx() {
        if (data == null) { liveIdx = null; return; }
        int n = data.rows();
        if (liveIdx != null && liveIdx.length == Math.min(n, LIVE_SAMPLE)) return;
        if (n <= LIVE_SAMPLE) {
            liveIdx = new int[n];
            for (int r = 0; r < n; r++) liveIdx[r] = r;
        } else {
            liveIdx = new int[LIVE_SAMPLE];
            java.util.Random rng = new java.util.Random(42);
            for (int k = 0; k < LIVE_SAMPLE; k++) liveIdx[k] = rng.nextInt(n);
        }
    }

    /** Recompute the live "~N (X%)" label for the in-progress gate (subsampled, fast). */
    private void updateLiveStat() {
        if (data == null) { liveStatText = ""; return; }
        Gate g = inProgressGate();
        if (g == null) { liveStatText = ""; return; }
        ensureLiveIdx();
        int xc = data.indexOf(g.xChan);
        int yc = (g.yChan == null) ? -1 : data.indexOf(g.yChan);
        if (xc < 0) { liveStatText = ""; return; }
        int hit = 0;
        for (int idx : liveIdx) {
            double x = data.get(idx, xc);
            double y = yc < 0 ? 0 : data.get(idx, yc);
            if (pointInGate(g, x, y)) hit++;
        }
        double pct = liveIdx.length == 0 ? 0 : 100.0 * hit / liveIdx.length;
        long est = Math.round((double) hit * data.rows() / liveIdx.length);
        boolean exact = liveIdx.length == data.rows();
        liveStatText = String.format("%s%,d (%.1f%%)", exact ? "" : "~", est, pct);
    }

    /** Build the gate geometry currently being drawn, in data space (or null if not enough). */
    private Gate inProgressGate() {
        if ("Polygon".equals(tool)) {
            int n = polyPx.size() + (dragCur != null ? 1 : 0);
            if (n < 3) return null;
            double[] xs = new double[n], ys = new double[n];
            for (int k = 0; k < polyPx.size(); k++) { xs[k] = dataX(polyPx.get(k)[0]); ys[k] = dataY(polyPx.get(k)[1]); }
            if (dragCur != null) { xs[n - 1] = dataX(dragCur[0]); ys[n - 1] = dataY(dragCur[1]); }
            return new Gate(null, "polygon", xChan, yChan, xs, ys);
        }
        if (dragStart == null || dragCur == null) return null;
        double[] xs = {dataX(dragStart[0]), dataX(dragCur[0])};
        if ("Rectangle".equals(tool)) return new Gate(null, "rectangle", xChan, yChan, xs, new double[]{dataY(dragStart[1]), dataY(dragCur[1])});
        if ("Ellipse".equals(tool)) return new Gate(null, "ellipse", xChan, yChan, xs, new double[]{dataY(dragStart[1]), dataY(dragCur[1])});
        if ("Interval".equals(tool)) return new Gate(null, "interval", xChan, null, xs, null);
        return null;
    }

    // ---- membership math ----------------------------------------------------

    private static boolean pointInGate(Gate g, double x, double y) {
        switch (g.type) {
            case "rectangle": return x >= min(g.xs) && x <= max(g.xs) && y >= min(g.ys) && y <= max(g.ys);
            case "interval": case "line": return x >= min(g.xs) && x <= max(g.xs);  // line = interval, drawn as edges only
            case "ellipse": {
                double cx = (min(g.xs) + max(g.xs)) / 2, cy = (min(g.ys) + max(g.ys)) / 2;
                double rx = (max(g.xs) - min(g.xs)) / 2, ry = (max(g.ys) - min(g.ys)) / 2;
                if (rx == 0 || ry == 0) return false;
                double dx = x - cx, dy = y - cy;
                if (g.angle != 0) {
                    double cos = Math.cos(g.angle), sin = Math.sin(g.angle);
                    double tx = dx * cos + dy * sin;
                    dy = -dx * sin + dy * cos;
                    dx = tx;
                }
                return (dx / rx) * (dx / rx) + (dy / ry) * (dy / ry) <= 1.0;
            }
            case "polygon": return pointInPoly(g.xs, g.ys, x, y);
            case "q1": return x >= g.xs[0] && y >= g.ys[0];
            case "q2": return x <  g.xs[0] && y >= g.ys[0];
            case "q3": return x <  g.xs[0] && y <  g.ys[0];
            case "q4": return x >= g.xs[0] && y <  g.ys[0];
            default: return false;
        }
    }
    private static boolean pointInPoly(double[] xs, double[] ys, double x, double y) {
        boolean in = false;
        for (int i = 0, j = xs.length - 1; i < xs.length; j = i++) {
            if (((ys[i] > y) != (ys[j] > y))
                    && (x < (xs[j] - xs[i]) * (y - ys[i]) / (ys[j] - ys[i]) + xs[i])) in = !in;
        }
        return in;
    }

    private static double min(double[] a) { double m = a[0]; for (double v : a) m = Math.min(m, v); return m; }
    private static double max(double[] a) { double m = a[0]; for (double v : a) m = Math.max(m, v); return m; }
    private static String fmt(double v) {
        double a = Math.abs(v);
        if (a >= 1e5 || (a > 0 && a < 0.01)) return String.format("%.0e", v);
        return String.format("%.0f", v);
    }

    private static Color[] buildLut() {
        Color[] stops = {Color.web("#0000CC"), Color.web("#00CCFF"), Color.web("#00CC44"),
                Color.web("#FFFF00"), Color.web("#FF9900"), Color.web("#FF0000")};
        Color[] lut = new Color[256];
        for (int i = 0; i < 256; i++) {
            double t = i / 255.0 * (stops.length - 1);
            int s = (int) Math.floor(t); double f = t - s;
            Color a = stops[s], b = stops[Math.min(s + 1, stops.length - 1)];
            lut[i] = a.interpolate(b, f);
        }
        return lut;
    }
}
