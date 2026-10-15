//! Pseudocolour density rendering: events -> 2-D histogram -> RGBA pixels.

/// How a population is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlotKind {
    #[default]
    Pseudocolor,
    /// Occupied cells in a single colour.
    Dot,
    /// One-parameter histogram of the X axis.
    Histogram,
}

impl PlotKind {
    pub const ALL: [PlotKind; 3] = [PlotKind::Pseudocolor, PlotKind::Dot, PlotKind::Histogram];
    pub fn label(self) -> &'static str {
        match self {
            PlotKind::Pseudocolor => "Pseudocolor",
            PlotKind::Dot => "Dot plot",
            PlotKind::Histogram => "Histogram",
        }
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
}
