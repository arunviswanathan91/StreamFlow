package org.streamflow.ui;

import javafx.scene.canvas.Canvas;
import javafx.scene.canvas.GraphicsContext;
import javafx.scene.control.TextField;
import javafx.scene.input.MouseEvent;
import javafx.scene.layout.Pane;
import javafx.scene.layout.Region;
import javafx.scene.paint.Color;
import javafx.scene.text.Font;
import javafx.scene.text.FontWeight;
import javafx.scene.text.Text;

import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * A native, interactive spillover-matrix heatmap (FlowJo-style compensation editor). Renders the
 * N×N matrix as a colour-coded grid: the diagonal is locked at 1.0 (grey), off-diagonal spillover
 * is shaded white→red by magnitude (negative → blue). Clicking an off-diagonal cell opens an inline
 * editor; committing fires {@link #setOnCellEdit}. Channel pairs flagged by the residual diagnostic
 * are outlined so over/under-compensation is visible at a glance.
 *
 * <p>Convention matches the engine: row = source detector, column = where it spills, so
 * cell (r,c) is "how much of channel r leaks into channel c".
 */
public final class CompMatrixView extends Region {

    /** Notified when a cell edit commits: (row, col, newValue). */
    public interface CellEditHandler { void edited(int row, int col, double value); }

    private final Canvas canvas = new Canvas();
    private final Pane overlay = new Pane();      // hosts the inline editor above the canvas
    private List<String> channels = List.of();
    private double[][] values = new double[0][0];
    private final Set<Long> flagged = new HashSet<>();   // (row<<32 | col) for highlighted pairs
    private CellEditHandler onCellEdit;

    private double leftMargin = 130, topMargin = 96;     // room for row/col labels
    private double cell = 28;                              // square cell size (recomputed on layout)
    private int hoverR = -1, hoverC = -1;

    private static final Color GRID = Color.web("#33425a");
    private static final Color DIAG = Color.web("#2a3950");
    private static final Color TEXT = Color.web("#C7D6E8");
    private static final Color FLAG = Color.web("#FF8C00");

    public CompMatrixView() {
        getChildren().addAll(canvas, overlay);
        overlay.setPickOnBounds(false);
        setMinSize(200, 200);
        canvas.setOnMouseMoved(this::onMove);
        canvas.setOnMouseExited(e -> { hoverR = hoverC = -1; redraw(); });
        canvas.setOnMouseClicked(this::onClick);
        widthProperty().addListener((o, a, b) -> redraw());
        heightProperty().addListener((o, a, b) -> redraw());
    }

    public void setOnCellEdit(CellEditHandler h) { this.onCellEdit = h; }

    /** Replace the displayed matrix. {@code values[r][c]} is row r's spillover into column c. */
    public void setMatrix(List<String> channels, double[][] values) {
        this.channels = channels == null ? List.of() : List.copyOf(channels);
        this.values = values == null ? new double[0][0] : values;
        flagged.clear();          // stale residual flags don't carry over to a different matrix
        hoverR = hoverC = -1;
        overlay.getChildren().clear();   // dismiss any open inline editor
        // widen the left/top margins to fit the longest channel label
        double maxLabel = 0;
        Text probe = new Text();
        probe.setFont(Font.font("Segoe UI", 10));
        for (String c : this.channels) { probe.setText(c); maxLabel = Math.max(maxLabel, probe.getLayoutBounds().getWidth()); }
        leftMargin = Math.min(150, Math.max(80, maxLabel + 12));
        topMargin = Math.min(150, Math.max(70, maxLabel + 12));
        redraw();
    }

    /** Update a single cell's value (after an external table edit) and repaint. */
    public void setValue(int r, int c, double v) {
        if (r >= 0 && r < values.length && c >= 0 && c < values[r].length) { values[r][c] = v; redraw(); }
    }

    /** Highlight the given channel-name pairs (unordered) as flagged residual-correlation cells. */
    public void setFlagged(List<String[]> pairs) {
        flagged.clear();
        if (pairs != null) for (String[] p : pairs) {
            int a = channels.indexOf(p[0]), b = channels.indexOf(p[1]);
            if (a >= 0 && b >= 0) { flagged.add(key(a, b)); flagged.add(key(b, a)); }
        }
        redraw();
    }

    public void clear() { channels = List.of(); values = new double[0][0]; flagged.clear(); redraw(); }

    private static long key(int r, int c) { return ((long) r << 32) | (c & 0xffffffffL); }

    @Override protected void layoutChildren() {
        canvas.setWidth(getWidth());
        canvas.setHeight(getHeight());
        overlay.resizeRelocate(0, 0, getWidth(), getHeight());
        redraw();
    }

    private void recomputeCell() {
        int n = channels.size();
        if (n == 0) { cell = 28; return; }
        double availW = getWidth() - leftMargin - 8, availH = getHeight() - topMargin - 8;
        cell = Math.max(14, Math.min(56, Math.min(availW / n, availH / n)));
    }

    private void redraw() {
        double W = getWidth(), H = getHeight();
        if (W <= 0 || H <= 0) return;
        recomputeCell();
        GraphicsContext g = canvas.getGraphicsContext2D();
        g.clearRect(0, 0, W, H);
        int n = channels.size();
        if (n == 0) {
            g.setFill(TEXT); g.setFont(Font.font("Segoe UI", 12));
            g.fillText("Extract or Compute a spillover matrix to begin.", 16, 28);
            return;
        }
        double maxOff = 0.001;
        for (int r = 0; r < n; r++)
            for (int c = 0; c < n; c++)
                if (r != c) maxOff = Math.max(maxOff, Math.abs(values[r][c]));
        double ref = Math.max(0.1, maxOff);   // keep faint matrices readable

        // value font scales with cell so coefficients stay readable at any matrix size
        double valFs = Math.max(7.5, Math.min(12, cell * 0.4));
        boolean showVals = cell >= 16;
        g.setTextAlign(javafx.scene.text.TextAlignment.CENTER);
        g.setTextBaseline(javafx.geometry.VPos.CENTER);
        for (int r = 0; r < n; r++) {
            for (int c = 0; c < n; c++) {
                double x = leftMargin + c * cell, y = topMargin + r * cell;
                double v = values[r][c];
                Color bg = (r == c) ? DIAG : rampColor(v, ref);
                g.setFill(bg);
                g.fillRect(x, y, cell, cell);
                // flagged outline
                if (flagged.contains(key(r, c))) {
                    g.setStroke(FLAG); g.setLineWidth(2);
                    g.strokeRect(x + 1, y + 1, cell - 2, cell - 2);
                }
                // hover outline
                if (r == hoverR && c == hoverC) {
                    g.setStroke(Color.web("#0A7CFF")); g.setLineWidth(2);
                    g.strokeRect(x + 1, y + 1, cell - 2, cell - 2);
                }
                // always show the coefficient (so an identity/near-zero matrix reads clearly too)
                if (showVals) {
                    g.setFont(Font.font("Segoe UI", valFs));
                    g.setFill(textOn(bg));
                    g.fillText(r == c ? "1.00" : String.format("%.2f", v), x + cell / 2, y + cell / 2);
                }
            }
        }
        g.setTextAlign(javafx.scene.text.TextAlignment.LEFT);
        g.setTextBaseline(javafx.geometry.VPos.BASELINE);
        // grid lines
        g.setStroke(GRID); g.setLineWidth(0.5);
        for (int k = 0; k <= n; k++) {
            g.strokeLine(leftMargin, topMargin + k * cell, leftMargin + n * cell, topMargin + k * cell);
            g.strokeLine(leftMargin + k * cell, topMargin, leftMargin + k * cell, topMargin + n * cell);
        }
        // labels
        g.setFill(TEXT); g.setFont(Font.font("Segoe UI", 10));
        for (int r = 0; r < n; r++) {
            String lbl = channels.get(r);
            g.fillText(lbl, leftMargin - textW(lbl, 10) - 6, topMargin + r * cell + cell / 2 + 3);
        }
        for (int c = 0; c < n; c++) {
            String lbl = channels.get(c);
            double cx = leftMargin + c * cell + cell / 2;
            g.save();
            g.translate(cx + 3, topMargin - 6);
            g.rotate(-90);
            g.fillText(lbl, 0, 0);
            g.restore();
        }
        // axis caption (top-left corner, above the row labels)
        g.setFont(Font.font("Segoe UI", FontWeight.BOLD, 10));
        g.setFill(Color.web("#8FA8C4"));
        g.fillText("spills into →", 8, 14);
    }

    // light slate at zero spillover (so cells are visible against the dark pane), ramping to red.
    private static final Color ZERO_BG = Color.web("#E9EEF5");
    private static final Color POS_END = Color.web("#D7261E");   // strong spillover → red
    private static final Color NEG_END = Color.web("#1565C0");   // negative (over-comp) → blue
    /** Light-slate→red for positive spillover, light-slate→blue for negative (over-compensation). */
    private static Color rampColor(double v, double ref) {
        double t = Math.max(-1, Math.min(1, v / ref));
        return t >= 0 ? ZERO_BG.interpolate(POS_END, t) : ZERO_BG.interpolate(NEG_END, -t);
    }
    private static Color textOn(Color bg) {
        double lum = 0.299 * bg.getRed() + 0.587 * bg.getGreen() + 0.114 * bg.getBlue();
        return lum > 0.6 ? Color.web("#1A2330") : Color.web("#EAEFF5");
    }
    private static double textW(String s, double size) {
        Text t = new Text(s); t.setFont(Font.font("Segoe UI", size));
        return t.getLayoutBounds().getWidth();
    }

    // ---- interaction --------------------------------------------------------

    private int[] cellAt(double mx, double my) {
        int n = channels.size();
        if (n == 0) return null;
        if (mx < leftMargin || my < topMargin) return null;
        int c = (int) ((mx - leftMargin) / cell), r = (int) ((my - topMargin) / cell);
        if (r < 0 || r >= n || c < 0 || c >= n) return null;
        return new int[]{r, c};
    }

    private void onMove(MouseEvent e) {
        int[] rc = cellAt(e.getX(), e.getY());
        int nr = rc == null ? -1 : rc[0], nc = rc == null ? -1 : rc[1];
        if (nr != hoverR || nc != hoverC) {
            hoverR = nr; hoverC = nc;
            if (rc != null) {
                String tip = rc[0] == rc[1]
                        ? channels.get(rc[0]) + " (diagonal = 1.00, locked)"
                        : String.format("%s spills %.1f%% into %s",
                              channels.get(rc[0]), values[rc[0]][rc[1]] * 100.0, channels.get(rc[1]));
                setOnMouseMovedTooltip(tip);
            }
            redraw();
        }
    }

    private javafx.scene.control.Tooltip tooltip;
    private void setOnMouseMovedTooltip(String text) {
        if (tooltip == null) { tooltip = new javafx.scene.control.Tooltip(); javafx.scene.control.Tooltip.install(this, tooltip); }
        tooltip.setText(text);
    }

    private void onClick(MouseEvent e) {
        int[] rc = cellAt(e.getX(), e.getY());
        if (rc == null || rc[0] == rc[1]) return;   // diagonal locked
        beginEdit(rc[0], rc[1]);
    }

    private void beginEdit(int r, int c) {
        overlay.getChildren().clear();
        double x = leftMargin + c * cell, y = topMargin + r * cell;
        TextField tf = new TextField(String.format("%.3f", values[r][c]));
        tf.setPrefSize(cell, cell);
        tf.relocate(x, y);
        tf.setStyle("-fx-font-size:10; -fx-padding:1; -fx-background-color:#FFFFFF; -fx-text-fill:#111;");
        Runnable commit = () -> {
            try {
                double nv = Double.parseDouble(tf.getText().trim());
                values[r][c] = nv;
                if (onCellEdit != null) onCellEdit.edited(r, c, nv);
            } catch (NumberFormatException ignored) { }
            overlay.getChildren().clear();
            redraw();
        };
        tf.setOnAction(ev -> commit.run());
        tf.focusedProperty().addListener((o, was, is) -> { if (!is) commit.run(); });
        tf.setOnKeyPressed(ev -> { if (ev.getCode() == javafx.scene.input.KeyCode.ESCAPE) { overlay.getChildren().clear(); redraw(); } });
        overlay.getChildren().add(tf);
        tf.requestFocus();
        tf.selectAll();
    }
}
