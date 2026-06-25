package org.streamflow.ui;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.FloatBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * An in-memory event matrix for one sample (or a gated subset), read from the
 * engine's {@code get_events} little-endian float32 blob. Held in Java so the
 * Graph Window can render any channel pair and gate instantly with no engine
 * round-trips (the FlowJo-style interaction model).
 */
public final class EventData {

    private final float[] data;   // row-major, rows*cols
    private final int rows;
    private final int cols;
    private final List<String> channels;

    public EventData(float[] data, int rows, int cols, List<String> channels) {
        this.data = data; this.rows = rows; this.cols = cols; this.channels = channels;
    }

    /** Read the engine blob: a flat float32 array of rows*cols, channels in order. */
    public static EventData read(Path bin, List<String> channels, int rows, int cols) throws IOException {
        byte[] bytes = Files.readAllBytes(bin);
        FloatBuffer fb = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN).asFloatBuffer();
        float[] d = new float[rows * cols];
        fb.get(d);
        return new EventData(d, rows, cols, List.copyOf(channels));
    }

    public int rows() { return rows; }
    public List<String> channels() { return channels; }
    public int indexOf(String channel) { return channels.indexOf(channel); }
    public float get(int row, int col) { return data[row * cols + col]; }

    /** Build a subset EventData containing only rows where {@code keep[row]} is true. */
    public EventData subset(boolean[] keep) {
        int n = 0;
        for (boolean b : keep) if (b) n++;
        float[] d = new float[n * cols];
        int w = 0;
        for (int r = 0; r < rows; r++) {
            if (keep[r]) {
                System.arraycopy(data, r * cols, d, w * cols, cols);
                w++;
            }
        }
        return new EventData(d, n, cols, channels);
    }

    /** [min, max] over a column, ignoring non-finite values. */
    public double[] range(int col) {
        double mn = Double.POSITIVE_INFINITY, mx = Double.NEGATIVE_INFINITY;
        for (int r = 0; r < rows; r++) {
            float v = data[r * cols + col];
            if (Float.isFinite(v)) { if (v < mn) mn = v; if (v > mx) mx = v; }
        }
        if (mn > mx) { mn = 0; mx = 1; }
        if (mn == mx) { mn -= 1; mx += 1; }
        return new double[]{mn, mx};
    }
}
