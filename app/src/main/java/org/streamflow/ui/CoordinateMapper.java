package org.streamflow.ui;

/**
 * Maps between canvas pixels and flow-data coordinates for the gating overlay,
 * using the plot-region pixel box + data ranges the engine returns from
 * {@code render_plot}. Canvas pixel-Y grows downward; data-Y grows upward, so the
 * Y mapping is flipped. Assumes the canvas is 1:1 with the rendered image
 * (the Graph Window renders at scale 1 and shows the image at natural size).
 */
public final class CoordinateMapper {

    private final double left, top, right, bottom;   // plot region, pixels
    private final double xmin, xmax, ymin, ymax;     // data extent

    public CoordinateMapper(double[] plotBoxPx, double[] xrange, double[] yrange) {
        this.left = plotBoxPx[0]; this.top = plotBoxPx[1];
        this.right = plotBoxPx[2]; this.bottom = plotBoxPx[3];
        this.xmin = xrange[0]; this.xmax = xrange[1];
        this.ymin = yrange[0]; this.ymax = yrange[1];
    }

    public double toDataX(double px) { return xmin + (px - left) / (right - left) * (xmax - xmin); }
    public double toDataY(double py) { return ymax - (py - top) / (bottom - top) * (ymax - ymin); }
    public double toPxX(double dx) { return left + (dx - xmin) / (xmax - xmin) * (right - left); }
    public double toPxY(double dy) { return top + (ymax - dy) / (ymax - ymin) * (bottom - top); }

    /** True if a canvas point is inside the plot region (where gating is valid). */
    public boolean inPlot(double px, double py) {
        return px >= Math.min(left, right) && px <= Math.max(left, right)
            && py >= Math.min(top, bottom) && py <= Math.max(top, bottom);
    }

    /** Clamp a pixel point to the plot region (so gates can't extend past the axes). */
    public double clampX(double px) { return Math.max(Math.min(left, right), Math.min(Math.max(left, right), px)); }
    public double clampY(double py) { return Math.max(Math.min(top, bottom), Math.min(Math.max(top, bottom), py)); }
}
