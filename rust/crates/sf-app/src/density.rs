//! Pseudocolour density rendering: events -> 2-D histogram -> RGBA pixels.

/// How a population is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotKind {
    #[default]
    Pseudocolor,
    /// Occupied cells in a single colour.
    Dot,
    /// Iso-density lines enclosing equal fractions of the events.
    Contour,
    /// Smoothed density in greys (dark = dense).
    Density,
    /// Pseudocolour in alternating bands of equal event fractions.
    Zebra,
    /// One-parameter histogram of the X axis.
    Histogram,
}

impl PlotKind {
    pub const ALL: [PlotKind; 6] = [PlotKind::Pseudocolor, PlotKind::Contour, PlotKind::Density, PlotKind::Zebra, PlotKind::Dot, PlotKind::Histogram];
    pub fn label(self) -> &'static str {
        match self {
            PlotKind::Pseudocolor => "Pseudocolor",
            PlotKind::Dot => "Dot plot",
            PlotKind::Contour => "Contour",
            PlotKind::Density => "Density",
            PlotKind::Zebra => "Zebra",
            PlotKind::Histogram => "Histogram",
        }
    }
    /// Stable name used in saved files.
    pub fn key(self) -> &'static str {
        match self {
            PlotKind::Pseudocolor => "pseudocolor",
            PlotKind::Dot => "dot",
            PlotKind::Contour => "contour",
            PlotKind::Density => "density",
            PlotKind::Zebra => "zebra",
            PlotKind::Histogram => "histogram",
        }
    }
    pub fn from_key(s: &str) -> Option<PlotKind> {
        PlotKind::ALL.into_iter().find(|k| k.key() == s)
    }
}

/// What the Y axis of a histogram measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistY {
    /// Events per bin.
    Count,
    /// Scaled so the tallest bin is 100% (FlowJo's default, "Normalized To Mode").
    #[default]
    Mode,
    /// Percent of the tallest bin (same picture as Mode, labelled "% of Max").
    PercentMax,
    /// Area under the curve is 1 (events per unit of the displayed axis).
    UnitArea,
    /// Cumulative percent of events (a CDF).
    Cdf,
}

impl HistY {
    pub const ALL: [HistY; 5] = [HistY::Mode, HistY::PercentMax, HistY::Count, HistY::UnitArea, HistY::Cdf];
    pub fn label(self) -> &'static str {
        match self {
            HistY::Count => "Count",
            HistY::Mode => "Normalized to mode",
            HistY::PercentMax => "% of max",
            HistY::UnitArea => "Unit area",
            HistY::Cdf => "Cumulative %",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            HistY::Count => "count",
            HistY::Mode => "mode",
            HistY::PercentMax => "percent_max",
            HistY::UnitArea => "unit_area",
            HistY::Cdf => "cdf",
        }
    }
    pub fn from_key(s: &str) -> Option<HistY> {
        HistY::ALL.into_iter().find(|k| k.key() == s)
    }
}

/// How a histogram curve is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistStyle {
    /// Filled under the curve, with an outline.
    #[default]
    Filled,
    /// One step per bin, filled.
    Bars,
    /// Outline only.
    Line,
}

impl HistStyle {
    pub const ALL: [HistStyle; 3] = [HistStyle::Filled, HistStyle::Bars, HistStyle::Line];
    pub fn label(self) -> &'static str {
        match self {
            HistStyle::Filled => "Filled",
            HistStyle::Bars => "Bars",
            HistStyle::Line => "Line only",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            HistStyle::Filled => "filled",
            HistStyle::Bars => "bars",
            HistStyle::Line => "line",
        }
    }
    pub fn from_key(s: &str) -> Option<HistStyle> {
        HistStyle::ALL.into_iter().find(|k| k.key() == s)
    }
}

/// Every display option of the density-based and histogram plots. Defaults are the previous Java app's.
#[derive(Debug, Clone, PartialEq)]
pub struct PlotStyle {
    // ---- histogram
    pub hist_y: HistY,
    pub hist_style: HistStyle,
    pub hist_smooth: bool,
    /// 0..=1; the blur radius is `bandwidth * 24` bins at 500 bins across (scaled with the bin count).
    pub hist_bandwidth: f64,
    pub hist_fill: [u8; 3],
    /// 0..=255
    pub hist_fill_alpha: u8,
    pub hist_line: [u8; 3],
    // ---- density and zebra
    pub smooth: bool,
    /// Gaussian blur radius (1..=8) in the 128 x 128 working grid.
    pub smooth_strength: usize,
    // ---- contour
    pub contour_levels: usize,
    pub contour_smooth: bool,
    pub contour_strength: usize,
    pub contour_color: [u8; 3],
    /// Draw single events that lie outside the outermost contour.
    pub outliers: bool,
    pub outlier_size: usize,
    // ---- arrangement
    /// Histograms of the X and Y parameters along the edges of a two-parameter plot.
    pub adjunct: bool,
    /// Light grid lines at the major ticks.
    pub grid: bool,
}

impl Default for PlotStyle {
    fn default() -> Self {
        PlotStyle {
            hist_y: HistY::Mode,
            hist_style: HistStyle::Filled,
            hist_smooth: true,
            hist_bandwidth: 0.5,
            hist_fill: [0, 140, 200],
            hist_fill_alpha: 110,
            hist_line: [0, 100, 160],
            smooth: true,
            smooth_strength: 3,
            contour_levels: 10,
            contour_smooth: true,
            contour_strength: 3,
            contour_color: [0x0B, 0x3D, 0x91],
            outliers: false,
            outlier_size: 0,
            adjunct: false,
            grid: true,
        }
    }
}

impl PlotStyle {
    /// Keep every number inside the range the controls allow.
    pub fn clamped(mut self) -> PlotStyle {
        self.contour_levels = self.contour_levels.clamp(3, 30);
        self.contour_strength = self.contour_strength.clamp(1, 8);
        self.smooth_strength = self.smooth_strength.clamp(1, 8);
        self.outlier_size = self.outlier_size.min(6);
        self.hist_bandwidth = if self.hist_bandwidth.is_finite() { self.hist_bandwidth.clamp(0.0, 1.0) } else { 0.5 };
        self
    }
}

/// Display-space rectangle shown by a plot.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
}

impl Range {
    pub fn is_valid(&self) -> bool {
        self.x1 > self.x0 && self.y1 > self.y0 && [self.x0, self.x1, self.y0, self.y1].iter().all(|v| v.is_finite())
    }
}

#[derive(Debug, Clone)]
pub struct Grid {
    pub w: usize,
    pub h: usize,
    /// Row-major with row 0 = LOWEST y (data orientation); [`render`] flips for screen order.
    pub counts: Vec<u32>,
    pub max: u32,
    /// Events that fell inside the range.
    pub inside: usize,
}

/// Bin points into a `w x h` grid over `range`. Non-finite points and points outside the range are skipped;
/// a point exactly on the upper edge belongs to the last cell.
pub fn bin(points: impl Iterator<Item = (f64, f64)>, range: Range, w: usize, h: usize) -> Grid {
    assert!(w > 0 && h > 0 && range.is_valid(), "bin needs a non-empty grid and a valid range");
    let mut counts = vec![0u32; w * h];
    let (sx, sy) = (w as f64 / (range.x1 - range.x0), h as f64 / (range.y1 - range.y0));
    let (mut max, mut inside) = (0u32, 0usize);
    for (x, y) in points {
        if !(x.is_finite() && y.is_finite()) || x < range.x0 || x > range.x1 || y < range.y0 || y > range.y1 {
            continue;
        }
        let cx = (((x - range.x0) * sx) as usize).min(w - 1);
        let cy = (((y - range.y0) * sy) as usize).min(h - 1);
        let c = &mut counts[cy * w + cx];
        *c += 1;
        max = max.max(*c);
        inside += 1;
    }
    Grid { w, h, counts, max, inside }
}

/// FlowJo-style pseudocolour ramp: blue -> cyan -> green -> yellow -> red for `t` in `[0, 1]`.
pub fn pseudocolor(t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let stops: [(f64, [f64; 3]); 5] = [(0.0, [0.0, 0.0, 200.0]), (0.25, [0.0, 190.0, 255.0]), (0.5, [0.0, 210.0, 60.0]), (0.75, [255.0, 235.0, 0.0]), (1.0, [220.0, 0.0, 0.0])];
    for w in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            let f = (t - t0) / (t1 - t0);
            return [0, 1, 2].map(|i| (c0[i] + f * (c1[i] - c0[i])).round() as u8);
        }
    }
    [220, 0, 0]
}

/// The count that maps to the hot end of the ramp: the 99th percentile of the occupied cells (floored at 6), so one
/// saturated spike (debris in a corner) cannot wash the rest of the plot to a single cold colour and a sparse
/// population is not painted hot.
pub fn reference_count(g: &Grid) -> u32 {
    let mut occupied: Vec<u32> = g.counts.iter().copied().filter(|&c| c > 0).collect();
    if occupied.is_empty() {
        return 6;
    }
    occupied.sort_unstable();
    // never below 6: a handful of events per cell is sparse, and must not be painted as the hottest colour
    occupied[((occupied.len() - 1) as f64 * 0.99).round() as usize].max(6)
}

/// RGBA pixels in SCREEN order (top row first). Empty cells are fully transparent; colour follows the
/// square root of the count relative to [`reference_count`] (cells above it are simply the hottest colour).
pub fn render(g: &Grid) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; g.w * g.h];
    if g.max == 0 {
        return px;
    }
    let reference = reference_count(g) as f64;
    for row in 0..g.h {
        let src = (g.h - 1 - row) * g.w; // flip: data row 0 is the bottom of the screen
        for col in 0..g.w {
            let c = g.counts[src + col];
            if c > 0 {
                let [r, gr, b] = pseudocolor((c as f64 / reference).sqrt());
                px[row * g.w + col] = [r, gr, b, 255];
            }
        }
    }
    px
}

/// Occupied cells in one colour, SCREEN order (see [`render`]).
pub fn render_dots(g: &Grid, rgb: [u8; 3]) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; g.w * g.h];
    for row in 0..g.h {
        let src = (g.h - 1 - row) * g.w;
        for col in 0..g.w {
            if g.counts[src + col] > 0 {
                px[row * g.w + col] = [rgb[0], rgb[1], rgb[2], 255];
            }
        }
    }
    px
}

/// 1-D histogram of `values` over `[lo, hi]` in `bins` bins (values outside, or non-finite, are skipped; the
/// upper edge belongs to the last bin). Returns the counts.
pub fn histogram(values: impl Iterator<Item = f64>, lo: f64, hi: f64, bins: usize) -> Vec<u32> {
    assert!(bins > 0 && hi > lo && lo.is_finite() && hi.is_finite(), "histogram needs a valid range");
    let mut h = vec![0u32; bins];
    let s = bins as f64 / (hi - lo);
    for v in values {
        if v.is_finite() && v >= lo && v <= hi {
            h[(((v - lo) * s) as usize).min(bins - 1)] += 1;
        }
    }
    h
}

// ---- density, contour, zebra ----------------------------------------------------------------------------------------------

/// Gaussian kernel of radius `r` bins (sigma `max(1, r)`), the same one the previous Java app used.
fn gauss_kernel(r: usize) -> Vec<f64> {
    let sig = (r as f64).max(1.0);
    let mut k: Vec<f64> = (0..=2 * r).map(|i| (-((i as f64 - r as f64).powi(2)) / (2.0 * sig * sig)).exp()).collect();
    let sum: f64 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    k
}

/// 1-D Gaussian blur with the edge value repeated.
pub fn blur1d(a: &[f64], r: usize) -> Vec<f64> {
    if r == 0 || a.is_empty() {
        return a.to_vec();
    }
    let k = gauss_kernel(r);
    let n = a.len() as isize;
    (0..a.len()).map(|i| (0..=2 * r).map(|j| a[(i as isize + j as isize - r as isize).clamp(0, n - 1) as usize] * k[j]).sum()).collect()
}

fn blur2d(a: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let k = gauss_kernel(r);
    let at = |x: isize, y: isize| a[(y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize];
    let mut tmp = vec![0.0; a.len()];
    for y in 0..h {
        for x in 0..w {
            tmp[y * w + x] = (0..=2 * r).map(|j| at(x as isize + j as isize - r as isize, y as isize) * k[j]).sum();
        }
    }
    let at2 = |x: isize, y: isize| tmp[(y.clamp(0, h as isize - 1) as usize) * w + x.clamp(0, w as isize - 1) as usize];
    let mut out = vec![0.0; a.len()];
    for y in 0..h {
        for x in 0..w {
            out[y * w + x] = (0..=2 * r).map(|j| at2(x as isize, y as isize + j as isize - r as isize) * k[j]).sum();
        }
    }
    out
}

/// The smooth density of a binned plot, in SCREEN order (top row first), `w * h` values. With `smooth` it is
/// coarse-binned to 128 x 128, blurred and bilinearly interpolated back, which removes the pixel noise that makes
/// iso-lines jagged; without it the raw counts are returned.
pub fn density_field(g: &Grid, smooth: bool, strength: usize) -> Vec<f64> {
    let (w, h) = (g.w, g.h);
    // screen order: row 0 of the result is the top, i.e. the grid's last row
    let counts_screen: Vec<f64> = (0..h).flat_map(|row| (0..w).map(move |col| (row, col))).map(|(row, col)| g.counts[(h - 1 - row) * w + col] as f64).collect();
    if !smooth {
        return counts_screen;
    }
    const G: usize = 128;
    let mut c = vec![0.0; G * G];
    for y in 0..h {
        let cy = (y * G / h).min(G - 1);
        for x in 0..w {
            c[cy * G + (x * G / w).min(G - 1)] += counts_screen[y * w + x];
        }
    }
    let c = blur2d(&c, G, G, strength.max(1));
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        let gy = y as f64 / h as f64 * (G - 1) as f64;
        let (y0, fy) = (gy as usize, gy - gy.floor());
        let y1 = (y0 + 1).min(G - 1);
        for x in 0..w {
            let gx = x as f64 / w as f64 * (G - 1) as f64;
            let (x0, fx) = (gx as usize, gx - gx.floor());
            let x1 = (x0 + 1).min(G - 1);
            let (v00, v10, v01, v11) = (c[y0 * G + x0], c[y0 * G + x1], c[y1 * G + x0], c[y1 * G + x1]);
            out[y * w + x] = (v00 * (1.0 - fx) + v10 * fx) * (1.0 - fy) + (v01 * (1.0 - fx) + v11 * fx) * fy;
        }
    }
    out
}

/// `n` density thresholds, innermost first, that enclose 1/(n+1), 2/(n+1), ... of the total event mass.
pub fn equal_prob_levels(dens: &[f64], n: usize) -> Vec<f64> {
    let mut s: Vec<f64> = dens.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let total: f64 = s.iter().sum();
    let mut lv = vec![0.0; n];
    if total <= 0.0 {
        return lv;
    }
    let (mut acc, mut k) = (0.0, 0);
    for &v in s.iter().rev() {
        if k >= n {
            break;
        }
        acc += v;
        let frac = acc / total;
        while k < n && frac >= (k + 1) as f64 / (n + 1) as f64 {
            lv[k] = v;
            k += 1;
        }
    }
    lv
}

/// Which band a density value falls in: 0 below every level, `levels.len()` above all of them.
pub fn band_of(d: f64, levels: &[f64]) -> usize {
    if d <= 0.0 {
        return 0;
    }
    levels.iter().filter(|&&l| d >= l).count()
}

fn shade(c: [u8; 3], f: f64) -> [u8; 3] {
    [0, 1, 2].map(|i| (c[i] as f64 * f) as u8)
}

fn opaque(c: [u8; 3]) -> [u8; 4] {
    [c[0], c[1], c[2], 255]
}

/// Greyscale density: dark where dense, light where sparse (log scaled), transparent where there is none.
pub fn render_density(g: &Grid, style: &PlotStyle) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; g.w * g.h];
    if g.max == 0 {
        return px;
    }
    let dens = density_field(g, style.smooth, style.smooth_strength);
    let dmax = dens.iter().cloned().fold(0.0, f64::max);
    if !(dmax > 0.0) {
        return px;
    }
    for (p, &d) in dens.iter().enumerate() {
        if d > 0.0 {
            let v = (255.0 * (1.0 - (d + 1.0).ln() / (dmax + 1.0).ln())).clamp(0.0, 255.0) as u8;
            px[p] = [v, v, v, 255];
        }
    }
    px
}

/// Contour lines at equal-probability density levels; optionally single events outside the outermost line.
pub fn render_contour(g: &Grid, style: &PlotStyle) -> Vec<[u8; 4]> {
    let (w, h) = (g.w, g.h);
    let mut px = vec![[0u8; 4]; w * h];
    if g.max == 0 {
        return px;
    }
    let dens = density_field(g, style.contour_smooth, style.contour_strength);
    let lv = equal_prob_levels(&dens, style.contour_levels);
    let line = opaque(style.contour_color);
    if style.outliers && !lv.is_empty() {
        let lowest = lv[lv.len() - 1];
        let r = style.outlier_size as isize;
        for row in 0..h {
            for col in 0..w {
                if g.counts[(h - 1 - row) * w + col] == 0 || dens[row * w + col] >= lowest {
                    continue;
                }
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (yy, xx) = (row as isize + dy, col as isize + dx);
                        if yy >= 0 && yy < h as isize && xx >= 0 && xx < w as isize {
                            px[yy as usize * w + xx as usize] = line;
                        }
                    }
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let b = band_of(dens[y * w + x], &lv);
            let right = if x + 1 < w { band_of(dens[y * w + x + 1], &lv) } else { b };
            let below = if y + 1 < h { band_of(dens[(y + 1) * w + x], &lv) } else { b };
            if b != right || b != below {
                px[y * w + x] = line;
            }
        }
    }
    px
}

/// Pseudocolour in 20 alternating bands of equal event fractions.
pub fn render_zebra(g: &Grid, style: &PlotStyle) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; g.w * g.h];
    if g.max == 0 {
        return px;
    }
    const N: usize = 20;
    let dens = density_field(g, style.smooth, style.smooth_strength);
    let lv = equal_prob_levels(&dens, N);
    for (p, &d) in dens.iter().enumerate() {
        if d <= 0.0 {
            continue;
        }
        let b = band_of(d, &lv);
        let base = pseudocolor(b as f64 / N as f64);
        px[p] = opaque(if b % 2 == 0 { base } else { shade(base, 0.78) });
    }
    px
}

/// RGBA pixels (screen order) of a binned population for any two-parameter plot type.
pub fn render_kind(g: &Grid, kind: PlotKind, style: &PlotStyle) -> Vec<[u8; 4]> {
    match kind {
        PlotKind::Pseudocolor => render(g),
        PlotKind::Dot => render_dots(g, [31, 111, 235]),
        PlotKind::Contour => render_contour(g, style),
        PlotKind::Density => render_density(g, style),
        PlotKind::Zebra => render_zebra(g, style),
        PlotKind::Histogram => vec![[0; 4]; g.w * g.h],
    }
}

// ---- histograms -------------------------------------------------------------------------------------------------------------

/// A histogram ready to draw. `heights[i]` is bin `i` as a fraction of the axis height; the axis tops out at
/// `axis_max` (in the unit named by `y_label`), and `axis_max` sits at `scale` of the height.
#[derive(Debug, Clone, PartialEq)]
pub struct HistCurve {
    pub heights: Vec<f64>,
    pub scale: f64,
    pub axis_max: f64,
    pub y_label: &'static str,
    /// Axis values are percentages.
    pub percent: bool,
    /// Events in range.
    pub total: f64,
}

/// Bin `values` over `[lo, hi]` and scale the result for the chosen Y axis, with the style's smoothing.
pub fn histogram_curve(values: impl Iterator<Item = f64>, lo: f64, hi: f64, bins: usize, style: &PlotStyle) -> HistCurve {
    let counts: Vec<f64> = histogram(values, lo, hi, bins).into_iter().map(|c| c as f64).collect();
    let total: f64 = counts.iter().sum();
    // raw bars are never smoothed; a cumulative curve is built from the raw counts
    let smooth = style.hist_smooth && style.hist_style != HistStyle::Bars && style.hist_y != HistY::Cdf;
    let counts = if smooth {
        let radius = ((style.hist_bandwidth * 24.0 * bins as f64 / 500.0).round() as usize).max(1);
        blur1d(&counts, radius)
    } else {
        counts
    };
    let peak = counts.iter().cloned().fold(0.0, f64::max);
    let flat = |label| HistCurve { heights: vec![0.0; bins], scale: 1.0, axis_max: 1.0, y_label: label, percent: false, total };
    if !(peak > 0.0) {
        return flat(style.hist_y.label());
    }
    match style.hist_y {
        HistY::Mode | HistY::PercentMax => HistCurve { heights: counts.iter().map(|c| c / peak * 0.94).collect(), scale: 0.94, axis_max: 100.0, y_label: style.hist_y.label(), percent: true, total },
        HistY::Count => {
            let top = crate::axis::nice_top(peak);
            HistCurve { heights: counts.iter().map(|c| c / top).collect(), scale: 1.0, axis_max: top, y_label: "Count", percent: false, total }
        }
        HistY::UnitArea => {
            let sum: f64 = counts.iter().sum();
            let bin_w = (hi - lo) / bins as f64;
            let dens: Vec<f64> = counts.iter().map(|c| c / (sum * bin_w)).collect();
            let top = crate::axis::nice_top(dens.iter().cloned().fold(0.0, f64::max));
            HistCurve { heights: dens.iter().map(|d| d / top).collect(), scale: 1.0, axis_max: top, y_label: "Unit area", percent: false, total }
        }
        HistY::Cdf => {
            let mut acc = 0.0;
            let cum: Vec<f64> = counts.iter().map(|c| { acc += c; acc / total }).collect();
            HistCurve { heights: cum, scale: 1.0, axis_max: 100.0, y_label: "Cumulative %", percent: true, total }
        }
    }
}

/// Put overlaid histograms on one Y axis. Percent-style curves already share one; count-style curves (count, unit
/// area) are rescaled to the tallest axis so heights stay comparable.
pub fn unify_curves(curves: &mut [HistCurve]) {
    if curves.len() < 2 || curves[0].percent {
        return;
    }
    let top = curves.iter().map(|c| c.axis_max).fold(0.0, f64::max);
    for c in curves.iter_mut() {
        let k = c.axis_max / top;
        c.heights.iter_mut().for_each(|h| *h *= k);
        c.axis_max = top;
    }
}

/// Y-axis ticks of a histogram: `(fraction of the axis height, label)`, from 0 up to the axis top.
pub fn hist_ticks(c: &HistCurve) -> Vec<(f64, String)> {
    let step = crate::axis::nice_step(c.axis_max, 4);
    let mut out = Vec::new();
    let mut k = 0;
    loop {
        let v = k as f64 * step;
        if v > c.axis_max * (1.0 + 1e-9) || k > 20 {
            break;
        }
        let label = if c.percent || v == 0.0 || v.abs() >= 1.0 { crate::axis::format_raw(v) } else { let s = format!("{v:.4}"); s.trim_end_matches('0').trim_end_matches('.').to_string() };
        out.push((v / c.axis_max * c.scale, label));
        k += 1;
    }
    out
}

/// The curve as `(x fraction 0..1, height)` points. Bars become steps (two points per bin).
pub fn hist_path(c: &HistCurve, style: HistStyle) -> Vec<(f64, f64)> {
    let n = c.heights.len().max(1) as f64;
    match style {
        HistStyle::Bars => c.heights.iter().enumerate().flat_map(|(i, &h)| [(i as f64 / n, h), ((i + 1) as f64 / n, h)]).collect(),
        _ => c.heights.iter().enumerate().map(|(i, &h)| ((i as f64 + 0.5) / n, h)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Range {
        Range { x0: 0.0, x1: 10.0, y0: 0.0, y1: 10.0 }
    }

    #[test]
    fn binning_counts_every_in_range_point_once_and_skips_the_rest() {
        let pts = [(0.5, 0.5), (0.6, 0.4), (9.99, 9.99), (10.0, 10.0), (-0.1, 5.0), (5.0, 11.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)];
        let g = bin(pts.iter().copied(), r(), 10, 10);
        assert_eq!(g.inside, 4);
        assert_eq!(g.counts.iter().sum::<u32>(), 4);
        assert_eq!(g.counts[0], 2, "(0.5,0.5) and (0.6,0.4) share the bottom-left cell");
        assert_eq!(g.counts[9 * 10 + 9], 2, "the upper edge belongs to the last cell");
        assert_eq!(g.max, 2);
    }

    #[test]
    fn rendering_flips_to_screen_order_and_leaves_empty_cells_transparent() {
        let g = bin([(0.5, 0.5)].into_iter(), r(), 10, 10); // bottom-left in data space
        let px = render(&g);
        assert_eq!(px[9 * 10][3], 255, "bottom-left data cell is the bottom-left pixel");
        assert_eq!(px.iter().filter(|p| p[3] != 0).count(), 1);
        assert!(render(&bin(std::iter::empty(), r(), 4, 4)).iter().all(|p| p[3] == 0));
    }

    #[test]
    fn one_saturated_cell_does_not_wash_out_the_rest_of_the_plot() {
        // 200 occupied cells with 10 events each, plus one cell with 100,000 events
        let mut counts = vec![0u32; 400];
        for c in counts.iter_mut().take(200) {
            *c = 10;
        }
        counts[399] = 100_000;
        let g = Grid { w: 20, h: 20, counts, max: 100_000, inside: 0 };
        assert_eq!(reference_count(&g), 10, "the reference follows the bulk, not the spike");
        let px = render(&g);
        let bulk = px.iter().filter(|p| p[3] == 255 && p[0] > 200).count();
        assert!(bulk >= 200, "ordinary cells reach the hot colours (got {bulk} hot pixels)");
        assert_eq!(reference_count(&Grid { w: 1, h: 1, counts: vec![0], max: 0, inside: 0 }), 6);
        // a sparse plot (1 event per cell) stays on the cool half of the ramp
        let sparse = Grid { w: 10, h: 10, counts: vec![1; 100], max: 1, inside: 100 };
        assert!(render(&sparse).iter().all(|p| p[0] < 50), "single events must not look hot");
    }

    #[test]
    fn the_colour_ramp_runs_from_blue_to_red_and_is_continuous() {
        assert_eq!(pseudocolor(0.0), [0, 0, 200]);
        assert_eq!(pseudocolor(1.0), [220, 0, 0]);
        assert_eq!(pseudocolor(5.0), pseudocolor(1.0), "clamped");
        let mut prev = pseudocolor(0.0);
        for i in 1..=200 {
            let c = pseudocolor(i as f64 / 200.0);
            assert!((0..3).all(|k| (c[k] as i32 - prev[k] as i32).abs() <= 24), "jump at {i}");
            prev = c;
        }
        // the densest cell is red, a cell with 1/100 of the peak is on the cool half of the ramp
        let hot = pseudocolor(1.0);
        let cool = pseudocolor((1.0f64 / 100.0).sqrt());
        assert!(hot[0] > 200 && cool[0] < 50);
    }

    #[test]
    #[should_panic]
    fn an_empty_range_is_a_programming_error() {
        bin(std::iter::empty(), Range { x0: 1.0, x1: 1.0, y0: 0.0, y1: 1.0 }, 4, 4);
    }
    // ---- contour / density / zebra / histogram -------------------------------------------------------------------

    /// A tight 2-D Gaussian blob centred in the range plus a few far-away events.
    fn blob(n: usize) -> Vec<(f64, f64)> {
        use sf_core::rng::Rng;
        let mut rng = Rng::new(9);
        let mut g = || {
            let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
            (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
        };
        (0..n).map(|_| (50.0 + 8.0 * g(), 50.0 + 8.0 * g())).collect()
    }

    fn big() -> Range {
        Range { x0: 0.0, x1: 100.0, y0: 0.0, y1: 100.0 }
    }

    #[test]
    fn defaults_are_the_previous_apps_and_out_of_range_values_are_clamped() {
        let d = PlotStyle::default();
        assert_eq!((d.contour_levels, d.contour_strength, d.smooth_strength, d.outliers, d.outlier_size), (10, 3, 3, false, 0));
        assert_eq!((d.hist_y, d.hist_style, d.hist_smooth, d.hist_bandwidth, d.adjunct), (HistY::Mode, HistStyle::Filled, true, 0.5, false));
        let c = PlotStyle { contour_levels: 1, contour_strength: 99, smooth_strength: 0, outlier_size: 50, hist_bandwidth: f64::NAN, ..d }.clamped();
        assert_eq!((c.contour_levels, c.contour_strength, c.smooth_strength, c.outlier_size, c.hist_bandwidth), (3, 8, 1, 6, 0.5));
        assert_eq!(PlotKind::ALL.len(), 6);
        assert!(PlotKind::ALL.iter().all(|k| PlotKind::from_key(k.key()) == Some(*k)) && PlotKind::from_key("scatter").is_none());
    }

    #[test]
    fn the_density_field_is_the_raw_counts_without_smoothing_and_peaks_at_the_blob_with_it() {
        let g = bin(blob(20_000).into_iter(), big(), 100, 100);
        let raw = density_field(&g, false, 3);
        assert_eq!(raw.iter().sum::<f64>() as u32, g.inside as u32);
        let sm = density_field(&g, true, 3);
        let at = |v: &[f64]| v.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| (i % 100, i / 100)).unwrap();
        let (px, py) = at(&sm);
        assert!((px as i32 - 50).abs() <= 4 && (py as i32 - 50).abs() <= 4, "the smoothed peak is at the blob centre: ({px}, {py})");
        assert!(sm.iter().cloned().fold(0.0, f64::max) < raw.iter().cloned().fold(0.0, f64::max), "smoothing lowers the noisy peak");
        // screen order: a blob placed in the LOWER half of the data range is in the LOWER half of the field
        let low = bin((0..3000).map(|i| (50.0 + (i % 7) as f64 * 0.1, 10.0 + (i % 5) as f64 * 0.1)), big(), 100, 100);
        let f = density_field(&low, false, 3);
        let (_, row) = at(&f);
        assert!(row > 80, "data y = 10 is near the bottom of the screen: row {row}");
    }

    #[test]
    fn equal_probability_levels_enclose_the_promised_fractions_of_the_events() {
        let g = bin(blob(40_000).into_iter(), big(), 200, 200);
        let dens = density_field(&g, true, 3);
        let n = 10;
        let lv = equal_prob_levels(&dens, n);
        assert_eq!(lv.len(), n);
        assert!(lv.windows(2).all(|w| w[0] >= w[1]), "innermost threshold first, decreasing: {lv:?}");
        let total: f64 = dens.iter().sum();
        for (k, &l) in lv.iter().enumerate() {
            let inside: f64 = dens.iter().filter(|&&d| d >= l).sum::<f64>() / total;
            let want = (k + 1) as f64 / (n + 1) as f64;
            assert!((inside - want).abs() < 0.05, "level {k} encloses {inside:.3}, wanted about {want:.3}");
        }
        assert_eq!(equal_prob_levels(&[0.0, 0.0], 4), vec![0.0; 4], "an empty field has no levels");
        assert_eq!(band_of(0.0, &lv), 0);
        assert_eq!(band_of(f64::MAX, &lv), n);
    }

    #[test]
    fn a_contour_plot_is_nested_rings_around_the_blob_and_nothing_else() {
        let g = bin(blob(30_000).into_iter(), big(), 200, 200);
        let px = render_contour(&g, &PlotStyle::default());
        let opaque_count = px.iter().filter(|p| p[3] > 0).count();
        assert!(opaque_count > 200 && opaque_count < 200 * 200 / 4, "only the lines are painted: {opaque_count}");
        // walking along the middle row from the left edge to the centre crosses every ring once
        let row = &px[100 * 200..101 * 200];
        let crossings = row[..100].windows(2).filter(|w| w[0][3] == 0 && w[1][3] > 0).count();
        assert!(crossings >= 8 && crossings <= 12, "10 levels give about 10 rings from the edge to the centre: {crossings}");
        // more levels, more rings
        let many = PlotStyle { contour_levels: 20, ..PlotStyle::default() };
        let lines = |p: &Vec<[u8; 4]>| p.iter().filter(|q| q[3] > 0).count();
        assert!(lines(&render_contour(&g, &many)) as f64 > 1.3 * lines(&px) as f64, "more levels draw more line");
        // the line colour is the style's
        let red = PlotStyle { contour_color: [255, 0, 0], ..PlotStyle::default() };
        assert!(render_contour(&g, &red).iter().filter(|p| p[3] > 0).all(|p| p[..3] == [255, 0, 0]));
        // an empty plot draws nothing
        assert!(render_contour(&bin(std::iter::empty(), big(), 20, 20), &PlotStyle::default()).iter().all(|p| p[3] == 0));
    }

    #[test]
    fn outliers_are_single_events_outside_the_outermost_contour_and_only_when_asked() {
        let mut pts = blob(20_000);
        pts.push((5.0, 5.0)); // one far-away event
        pts.push((95.0, 90.0));
        let g = bin(pts.into_iter(), big(), 200, 200);
        let (off, on) = (render_contour(&g, &PlotStyle::default()), render_contour(&g, &PlotStyle { outliers: true, ..PlotStyle::default() }));
        let corner = |px: &Vec<[u8; 4]>| (px[(200 - 1 - 10) * 200 + 10][3] > 0, px[(200 - 1 - 180) * 200 + 190][3] > 0);
        assert_eq!(corner(&off), (false, false));
        assert_eq!(corner(&on), (true, true), "both stray events are drawn");
        let big_dots = render_contour(&g, &PlotStyle { outliers: true, outlier_size: 2, ..PlotStyle::default() });
        assert!(big_dots.iter().filter(|p| p[3] > 0).count() > on.iter().filter(|p| p[3] > 0).count(), "a larger outlier size stamps more pixels");
    }

    #[test]
    fn zebra_alternates_shades_between_bands_and_density_is_darkest_where_densest() {
        let g = bin(blob(30_000).into_iter(), big(), 200, 200);
        let z = render_zebra(&g, &PlotStyle::default());
        let mut colours: Vec<[u8; 4]> = z.iter().copied().filter(|p| p[3] > 0).collect();
        colours.sort();
        colours.dedup();
        assert!(colours.len() >= 12, "bands give many distinct colours, alternating light and dark: {}", colours.len());
        let d = render_density(&g, &PlotStyle::default());
        let centre = d[100 * 200 + 100];
        let edge = d.iter().copied().filter(|p| p[3] > 0).max_by_key(|p| p[0]).unwrap();
        assert!(centre[0] < 40, "the densest cell is nearly black: {centre:?}");
        assert!(edge[0] > centre[0] + 100, "the sparse haze is much lighter: {edge:?}");
        assert_eq!(d[0][3], 0, "far from any event there is no density");
        // render_kind dispatches every kind and a histogram draws no 2-D pixels
        for k in [PlotKind::Pseudocolor, PlotKind::Dot, PlotKind::Contour, PlotKind::Density, PlotKind::Zebra] {
            assert!(render_kind(&g, k, &PlotStyle::default()).iter().any(|p| p[3] > 0), "{k:?}");
        }
        assert!(render_kind(&g, PlotKind::Histogram, &PlotStyle::default()).iter().all(|p| p[3] == 0));
    }

    fn values(n: usize) -> Vec<f64> {
        blob(n).into_iter().map(|p| p.0).collect()
    }

    #[test]
    fn histogram_y_modes_scale_as_their_names_say() {
        let v = values(20_000);
        let none = PlotStyle { hist_smooth: false, ..PlotStyle::default() };
        // normalized to mode / % of max: the tallest bin is 100% (drawn at 94% of the height)
        for y in [HistY::Mode, HistY::PercentMax] {
            let c = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle { hist_y: y, ..none.clone() });
            assert!((c.heights.iter().cloned().fold(0.0, f64::max) - 0.94).abs() < 1e-9 && c.axis_max == 100.0 && c.percent, "{y:?}");
            assert_eq!(c.y_label, y.label());
        }
        // count: the axis tops out on a clean number at or above the tallest bin
        let c = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle { hist_y: HistY::Count, ..none.clone() });
        let raw_peak = histogram(v.iter().copied(), 0.0, 100.0, 100).into_iter().max().unwrap() as f64;
        assert!(c.axis_max >= raw_peak && c.axis_max < raw_peak * 1.45 + 1.0, "the axis hugs the peak: {} vs {raw_peak}", c.axis_max);
        assert!((c.heights.iter().cloned().fold(0.0, f64::max) * c.axis_max - raw_peak).abs() < 1e-6);
        assert!(!c.percent && c.y_label == "Count");
        // unit area: the curve integrates to 1 over the displayed axis
        let u = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle { hist_y: HistY::UnitArea, ..none.clone() });
        let area: f64 = u.heights.iter().map(|h| h * u.axis_max).sum::<f64>() * (100.0 / 100.0);
        assert!((area - 1.0).abs() < 1e-9, "{area}");
        // cumulative: monotone and reaches 100%
        let c = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle { hist_y: HistY::Cdf, ..PlotStyle::default() });
        assert!(c.heights.windows(2).all(|w| w[1] >= w[0]) && (c.heights.last().unwrap() - 1.0).abs() < 1e-9 && c.axis_max == 100.0);
        // empty input is a flat zero curve, not a panic
        let e = histogram_curve(std::iter::empty(), 0.0, 1.0, 10, &PlotStyle::default());
        assert!(e.heights.iter().all(|h| *h == 0.0) && e.total == 0.0);
    }

    #[test]
    fn smoothing_reduces_noise_but_raw_bars_and_cdf_are_never_smoothed() {
        let v = values(3_000);
        let rough = |c: &HistCurve| c.heights.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>();
        let none = histogram_curve(v.iter().copied(), 0.0, 100.0, 200, &PlotStyle { hist_smooth: false, ..PlotStyle::default() });
        let some = histogram_curve(v.iter().copied(), 0.0, 100.0, 200, &PlotStyle::default());
        let more = histogram_curve(v.iter().copied(), 0.0, 100.0, 200, &PlotStyle { hist_bandwidth: 1.0, ..PlotStyle::default() });
        assert!(rough(&some) < 0.5 * rough(&none) && rough(&more) < rough(&some), "{} {} {}", rough(&none), rough(&some), rough(&more));
        let bars = histogram_curve(v.iter().copied(), 0.0, 100.0, 200, &PlotStyle { hist_style: HistStyle::Bars, ..PlotStyle::default() });
        assert_eq!(bars.heights, none.heights, "bars show the raw counts");
    }

    #[test]
    fn overlaid_count_histograms_share_the_tallest_axis() {
        let big: Vec<f64> = (0..4000).map(|i| (i % 100) as f64).collect();
        let small: Vec<f64> = (0..400).map(|i| (i % 100) as f64).collect();
        let st = PlotStyle { hist_y: HistY::Count, hist_smooth: false, ..PlotStyle::default() };
        let mut cs = vec![histogram_curve(big.iter().copied(), 0.0, 100.0, 100, &st), histogram_curve(small.iter().copied(), 0.0, 100.0, 100, &st)];
        assert!(cs[0].axis_max > cs[1].axis_max);
        unify_curves(&mut cs);
        assert_eq!(cs[0].axis_max, cs[1].axis_max);
        let tops: Vec<f64> = cs.iter().map(|c| c.heights.iter().cloned().fold(0.0, f64::max) * c.axis_max).collect();
        assert!((tops[0] - 40.0).abs() < 1e-9 && (tops[1] - 4.0).abs() < 1e-9, "each curve keeps its own counts: {tops:?}");
        // percent curves are left alone (every layer already peaks at the same height)
        let pc = PlotStyle { hist_smooth: false, ..PlotStyle::default() };
        let mut ps = vec![histogram_curve(big.iter().copied(), 0.0, 100.0, 100, &pc), histogram_curve(small.iter().copied(), 0.0, 100.0, 100, &pc)];
        let before = ps.clone();
        unify_curves(&mut ps);
        assert_eq!(ps, before);
    }

    #[test]
    fn histogram_ticks_and_paths() {
        let v = values(5_000);
        let c = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle::default());
        let t = hist_ticks(&c);
        assert_eq!(t[0], (0.0, "0".to_string()));
        assert!(t.iter().any(|(_, l)| l == "100"), "{t:?}");
        assert!((t.last().unwrap().0 - 0.94).abs() < 1e-9, "100% sits at 94% of the height: {t:?}");
        let cnt = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle { hist_y: HistY::Count, ..PlotStyle::default() });
        assert!(hist_ticks(&cnt).len() >= 3 && hist_ticks(&cnt).iter().all(|(f, _)| *f >= 0.0 && *f <= 1.0 + 1e-9));
        let ua = histogram_curve(v.iter().copied(), 0.0, 100.0, 100, &PlotStyle { hist_y: HistY::UnitArea, hist_smooth: false, ..PlotStyle::default() });
        assert!(hist_ticks(&ua).iter().any(|(_, l)| l.contains('.')), "small unit-area values keep their decimals: {:?}", hist_ticks(&ua));
        // a path point per bin, or a step (two points) per bin for bars
        assert_eq!(hist_path(&c, HistStyle::Filled).len(), 100);
        let steps = hist_path(&c, HistStyle::Bars);
        assert_eq!(steps.len(), 200);
        assert_eq!((steps[0].0, steps[1].0, steps[1].1 == steps[0].1), (0.0, 0.01, true));
    }

}
