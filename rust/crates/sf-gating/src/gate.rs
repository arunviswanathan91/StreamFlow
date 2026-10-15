//! Gate geometry in DATA coordinates. Membership rules are a faithful port of the Java `CytoPlot`
//! gates so existing workspaces keep their meaning:
//!
//! * `Rectangle` / `Interval`: an interval ignores y. The edge rule is selectable ([`EdgeRule`]):
//!   `HalfOpen` (`min <= v < max`, the GatingML 2.0 / FlowJo rule, the DEFAULT for new gates) or
//!   `Inclusive` (`min <= v <= max`, what the Java app did; kept so old workspaces keep their meaning).
//!   (8.5% of a gate in the reference fixture), so a loaded gate always records which rule it uses.
//! * `Ellipse`: axis extents from `xs`/`ys`, optionally rotated CCW by `angle` (radians);
//!   degenerate (zero radius) ellipses contain nothing.
//! * `Polygon`: even-odd ray casting.
//! * `Quadrant`: the 5-point "spider" form (centre plus two dividing arms) or, for legacy
//!   workspaces with a single point, the plain axis-aligned quadrants.
//! * `invert` flips membership ("NOT" / exclusion gate). NaN events satisfy no shape, so they fall
//!   *inside* an inverted gate; this mirrors the Java behaviour and is documented rather than hidden.
//! * `Indexed`: membership defined by explicit event rows (subsample / plugin / embedding / FlowJo
//!   import), always over the full sample.

use crate::mask::Mask;
use sf_core::scale::ScaleSpec;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum GateError {
    Invalid(String),
    MissingChannel(String),
    RowOutOfRange { row: u32, n_events: usize },
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::Invalid(m) => write!(f, "invalid gate: {m}"),
            GateError::MissingChannel(c) => write!(f, "channel '{c}' is not present in this sample"),
            GateError::RowOutOfRange { row, n_events } => {
                write!(f, "indexed gate refers to event {row} but the sample has {n_events} events")
            }
        }
    }
}

impl std::error::Error for GateError {}

fn invalid<T>(m: impl Into<String>) -> Result<T, GateError> {
    Err(GateError::Invalid(m.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quad {
    /// top-right
    Q1,
    /// top-left
    Q2,
    /// bottom-left
    Q3,
    /// bottom-right
    Q4,
}

/// How a rectangle/interval treats events exactly on its upper edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeRule {
    /// `min <= v <= max` (the old Java behaviour; only for gates loaded from old workspaces).
    Inclusive,
    /// `min <= v < max` (GatingML 2.0 / FlowKit / FlowJo) - the default for new gates.
    #[default]
    HalfOpen,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateKind {
    Rectangle,
    Interval,
    Ellipse,
    Polygon,
    Quadrant(Quad),
    Indexed(Vec<u32>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gate {
    pub kind: GateKind,
    pub x_chan: String,
    pub y_chan: Option<String>,
    pub xs: Vec<f64>,
    pub ys: Vec<f64>,
    /// Ellipse rotation, radians counter-clockwise from +X in data space.
    pub angle: f64,
    pub invert: bool,
    /// Only used by `Rectangle` / `Interval`.
    pub edges: EdgeRule,
    /// The scale each axis was drawn on. Coordinates (`xs`/`ys`) are in THIS space, so membership is tested on
    /// the scaled event values; `Identity` (the default) means raw data units.
    pub x_scale: ScaleSpec,
    pub y_scale: ScaleSpec,
}

/// A read-only `n x n_cols` row-major `f32` event matrix with named columns.
#[derive(Debug, Clone, Copy)]
pub struct EventView<'a> {
    data: &'a [f32],
    n_cols: usize,
    channels: &'a [String],
}

impl<'a> EventView<'a> {
    pub fn new(data: &'a [f32], n_cols: usize, channels: &'a [String]) -> Result<EventView<'a>, GateError> {
        if n_cols == 0 || channels.len() != n_cols || data.len() % n_cols != 0 {
            return invalid(format!("event matrix of {} values cannot be {} columns named {:?}", data.len(), n_cols, channels));
        }
        Ok(EventView { data, n_cols, channels })
    }

    pub fn n_events(&self) -> usize {
        self.data.len() / self.n_cols
    }

    pub fn col(&self, name: &str) -> Option<usize> {
        self.channels.iter().position(|c| c == name)
    }

    #[inline]
    pub fn get(&self, row: usize, col: usize) -> f64 {
        self.data[row * self.n_cols + col] as f64
    }
}

fn min(a: &[f64]) -> f64 {
    a.iter().cloned().fold(f64::INFINITY, f64::min)
}

fn max(a: &[f64]) -> f64 {
    a.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
}

impl Gate {
    /// Validating constructor. `xs`/`ys` follow the Java convention: extents for rectangle,
    /// interval and ellipse; vertices for a polygon; `[cx, a1x, m1x, a2x, m2x]` (spider) or `[cx]`
    /// (legacy) for a quadrant.
    pub fn new(kind: GateKind, x_chan: &str, y_chan: Option<&str>, xs: Vec<f64>, ys: Vec<f64>) -> Result<Gate, GateError> {
        if xs.iter().chain(&ys).any(|v| !v.is_finite()) {
            return invalid("coordinates must be finite numbers");
        }
        if x_chan.is_empty() {
            return invalid("x channel is required");
        }
        match &kind {
            GateKind::Interval if xs.len() < 2 => return invalid("an interval needs two x coordinates"),
            GateKind::Rectangle | GateKind::Ellipse if xs.len() < 2 || ys.len() < 2 => {
                return invalid("a rectangle/ellipse needs two x and two y coordinates")
            }
            GateKind::Polygon if xs.len() < 3 || xs.len() != ys.len() => {
                return invalid(format!("a polygon needs >= 3 vertices with matching x/y (got {} x, {} y)", xs.len(), ys.len()))
            }
            GateKind::Quadrant(_) if xs.is_empty() || ys.is_empty() || !(xs.len() == 1 || xs.len() == 5) || xs.len() != ys.len() => {
                return invalid("a quadrant needs 1 (legacy) or 5 (spider) matching points")
            }
            GateKind::Indexed(_) => {}
            _ => {}
        }
        if !matches!(kind, GateKind::Interval | GateKind::Indexed(_)) && y_chan.is_none() {
            return invalid("a two-parameter gate needs a y channel");
        }
        Ok(Gate { kind, x_chan: x_chan.to_string(), y_chan: y_chan.map(str::to_string), xs, ys, angle: 0.0, invert: false, edges: EdgeRule::default(), x_scale: ScaleSpec::Identity, y_scale: ScaleSpec::Identity })
    }

    pub fn rectangle(x_chan: &str, y_chan: &str, x0: f64, x1: f64, y0: f64, y1: f64) -> Result<Gate, GateError> {
        Gate::new(GateKind::Rectangle, x_chan, Some(y_chan), vec![x0, x1], vec![y0, y1])
    }

    pub fn interval(x_chan: &str, x0: f64, x1: f64) -> Result<Gate, GateError> {
        Gate::new(GateKind::Interval, x_chan, None, vec![x0, x1], vec![])
    }

    pub fn ellipse(x_chan: &str, y_chan: &str, cx: f64, cy: f64, rx: f64, ry: f64, angle: f64) -> Result<Gate, GateError> {
        let mut g = Gate::new(GateKind::Ellipse, x_chan, Some(y_chan), vec![cx - rx, cx + rx], vec![cy - ry, cy + ry])?;
        if !angle.is_finite() {
            return invalid("ellipse angle must be finite");
        }
        g.angle = angle;
        Ok(g)
    }

    pub fn polygon(x_chan: &str, y_chan: &str, vertices: &[(f64, f64)]) -> Result<Gate, GateError> {
        Gate::new(GateKind::Polygon, x_chan, Some(y_chan), vertices.iter().map(|v| v.0).collect(), vertices.iter().map(|v| v.1).collect())
    }

    /// Axis-aligned quadrant about `(cx, cy)` (the legacy single-point form).
    pub fn quadrant(x_chan: &str, y_chan: &str, q: Quad, cx: f64, cy: f64) -> Result<Gate, GateError> {
        Gate::new(GateKind::Quadrant(q), x_chan, Some(y_chan), vec![cx], vec![cy])
    }

    pub fn indexed(rows: Vec<u32>) -> Gate {
        Gate { kind: GateKind::Indexed(rows), x_chan: String::new(), y_chan: None, xs: vec![], ys: vec![], angle: 0.0, invert: false, edges: EdgeRule::default(), x_scale: ScaleSpec::Identity, y_scale: ScaleSpec::Identity }
    }

    /// Declare the scales of the axes this gate was drawn on (its coordinates are in that space).
    pub fn with_scales(mut self, x: ScaleSpec, y: ScaleSpec) -> Gate {
        self.x_scale = x;
        self.y_scale = y;
        self
    }

    pub fn with_edges(mut self, edges: EdgeRule) -> Gate {
        self.edges = edges;
        self
    }

    pub fn inverted(mut self) -> Gate {
        self.invert = !self.invert;
        self
    }

    /// Geometric membership of one point, *before* `invert`. `Indexed` gates have no geometry.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        match &self.kind {
            GateKind::Rectangle => self.in_range(x, &self.xs) && self.in_range(y, &self.ys),
            GateKind::Interval => self.in_range(x, &self.xs),
            GateKind::Ellipse => {
                let (cx, cy) = ((min(&self.xs) + max(&self.xs)) / 2.0, (min(&self.ys) + max(&self.ys)) / 2.0);
                let (rx, ry) = ((max(&self.xs) - min(&self.xs)) / 2.0, (max(&self.ys) - min(&self.ys)) / 2.0);
                if rx == 0.0 || ry == 0.0 {
                    return false;
                }
                let (mut dx, mut dy) = (x - cx, y - cy);
                if self.angle != 0.0 {
                    let (c, s) = (self.angle.cos(), self.angle.sin());
                    let tx = dx * c + dy * s;
                    dy = -dx * s + dy * c;
                    dx = tx;
                }
                (dx / rx) * (dx / rx) + (dy / ry) * (dy / ry) <= 1.0
            }
            GateKind::Polygon => point_in_polygon(&self.xs, &self.ys, x, y),
            GateKind::Quadrant(q) => in_quadrant(*q, &self.xs, &self.ys, x, y),
            GateKind::Indexed(_) => false,
        }
    }

    #[inline]
    fn in_range(&self, v: f64, bounds: &[f64]) -> bool {
        let (lo, hi) = (min(bounds), max(bounds));
        match self.edges {
            EdgeRule::Inclusive => v >= lo && v <= hi,
            EdgeRule::HalfOpen => v >= lo && v < hi,
        }
    }

    /// Membership over a whole event set (includes `invert`). A gate whose channel is absent returns
    /// `Err(MissingChannel)`; the tree turns that into an empty population plus a warning, so one
    /// bad gate never aborts a whole evaluation (BUG-01).
    pub fn mask(&self, view: &EventView) -> Result<Mask, GateError> {
        let n = view.n_events();
        if let GateKind::Indexed(rows) = &self.kind {
            let mut m = Mask::zeros(n);
            for &r in rows {
                if r as usize >= n {
                    return Err(GateError::RowOutOfRange { row: r, n_events: n });
                }
                m.set(r as usize, true);
            }
            return Ok(if self.invert { m.not() } else { m });
        }
        let xc = view.col(&self.x_chan).ok_or_else(|| GateError::MissingChannel(self.x_chan.clone()))?;
        let yc = match &self.y_chan {
            Some(c) => Some(view.col(c).ok_or_else(|| GateError::MissingChannel(c.clone()))?),
            None => None,
        };
        let tx = if self.x_scale.is_identity() { None } else { Some(self.x_scale.build().map_err(|e| GateError::Invalid(format!("x axis scale: {e}")))?) };
        let ty = if self.y_scale.is_identity() { None } else { Some(self.y_scale.build().map_err(|e| GateError::Invalid(format!("y axis scale: {e}")))?) };
        Ok(Mask::from_fn(n, |r| {
            let x = view.get(r, xc);
            let x = tx.as_ref().map_or(x, |t| t.forward(x));
            let y = yc.map_or(0.0, |c| view.get(r, c));
            let y = if yc.is_some() { ty.as_ref().map_or(y, |t| t.forward(y)) } else { y };
            self.contains(x, y) ^ self.invert
        }))
    }
}

/// Even-odd ray casting (identical to the Java implementation).
fn point_in_polygon(xs: &[f64], ys: &[f64], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = xs.len() - 1;
    for i in 0..xs.len() {
        if (ys[i] > y) != (ys[j] > y) && x < (xs[j] - xs[i]) * (y - ys[i]) / (ys[j] - ys[i]) + xs[i] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn in_quadrant(q: Quad, xs: &[f64], ys: &[f64], x: f64, y: f64) -> bool {
    let (cx, cy) = (xs[0], ys[0]);
    if xs.len() < 5 {
        return match q {
            Quad::Q1 => x >= cx && y >= cy,
            Quad::Q2 => x < cx && y >= cy,
            Quad::Q3 => x < cx && y < cy,
            Quad::Q4 => x >= cx && y < cy,
        };
    }
    let (a1x, a1y) = (xs[1] - cx, ys[1] - cy);
    let (a2x, a2y) = (xs[3] - cx, ys[3] - cy);
    let (px, py) = (x - cx, y - cy);
    // > 0: the point is to the LEFT of the arm direction
    let s1 = a1x * py - a1y * px > 0.0;
    let s2 = a2x * py - a2y * px > 0.0;
    match q {
        Quad::Q1 => s1 && !s2,
        Quad::Q2 => s1 && s2,
        Quad::Q3 => !s1 && s2,
        Quad::Q4 => !s1 && !s2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rectangle_and_interval_are_inclusive_on_every_edge() {
        let r = Gate::rectangle("X", "Y", 0.0, 10.0, 0.0, 5.0).unwrap().with_edges(EdgeRule::Inclusive);
        for (x, y, want) in [(0.0, 0.0, true), (10.0, 5.0, true), (10.0, 0.0, true), (10.0001, 2.0, false), (5.0, -0.0001, false)] {
            assert_eq!(r.contains(x, y), want, "({x},{y})");
        }
        let i = Gate::interval("X", 2.0, 4.0).unwrap().with_edges(EdgeRule::Inclusive);
        assert!(i.contains(2.0, 1e9) && i.contains(4.0, -1e9) && !i.contains(4.5, 0.0));
        // reversed corner order describes the same box
        assert!(Gate::rectangle("X", "Y", 10.0, 0.0, 5.0, 0.0).unwrap().contains(5.0, 2.0));
    }

    #[test]
    fn half_open_edges_exclude_the_upper_bound_and_inclusive_keeps_it() {
        let inc = Gate::rectangle("X", "Y", 0.0, 10.0, 0.0, 5.0).unwrap().with_edges(EdgeRule::Inclusive);
        let half = Gate::rectangle("X", "Y", 0.0, 10.0, 0.0, 5.0).unwrap().with_edges(EdgeRule::HalfOpen);
        for (x, y, i, h) in [(0.0, 0.0, true, true), (10.0, 2.0, true, false), (5.0, 5.0, true, false), (9.999, 4.999, true, true), (-0.001, 1.0, false, false)] {
            assert_eq!((inc.contains(x, y), half.contains(x, y)), (i, h), "({x},{y})");
        }
        let (ii, hi) = (Gate::interval("X", 2.0, 4.0).unwrap().with_edges(EdgeRule::Inclusive), Gate::interval("X", 2.0, 4.0).unwrap().with_edges(EdgeRule::HalfOpen));
        assert!(ii.contains(4.0, 0.0) && !hi.contains(4.0, 0.0) && hi.contains(2.0, 0.0));
        assert_eq!(Gate::interval("X", 3.0, 3.0).unwrap().with_edges(EdgeRule::HalfOpen).contains(3.0, 0.0), false, "empty half-open interval");
        assert_eq!(EdgeRule::default(), EdgeRule::HalfOpen, "new gates follow FlowJo/GatingML");
    }

    #[test]
    fn ellipse_axis_aligned_and_rotated_hand_checked() {
        let e = Gate::ellipse("X", "Y", 0.0, 0.0, 4.0, 2.0, 0.0).unwrap();
        assert!(e.contains(4.0, 0.0) && e.contains(0.0, 2.0) && e.contains(0.0, 0.0));
        assert!(!e.contains(4.0001, 0.0) && !e.contains(3.0, 1.9)); // (3/4)^2+(1.9/2)^2 = 1.465 > 1
        // rotate 90 degrees CCW: the long axis now points along +Y
        let r = Gate::ellipse("X", "Y", 0.0, 0.0, 4.0, 2.0, std::f64::consts::FRAC_PI_2).unwrap();
        assert!(r.contains(0.0, 3.9) && !r.contains(3.9, 0.0) && r.contains(1.9, 0.0) && !r.contains(2.1, 0.0));
        // rotate 45 degrees: the point (2.8, 2.8) lies on the long axis (distance 3.96 < 4)
        let d = Gate::ellipse("X", "Y", 0.0, 0.0, 4.0, 2.0, std::f64::consts::FRAC_PI_4).unwrap();
        assert!(d.contains(2.8, 2.8) && !d.contains(2.8, -2.8));
        assert!(!Gate::ellipse("X", "Y", 0.0, 0.0, 0.0, 2.0, 0.0).unwrap().contains(0.0, 0.0), "zero radius contains nothing");
    }

    #[test]
    fn polygon_handles_concavity_and_both_winding_directions() {
        // an "L" shape (concave)
        let pts = [(0.0, 0.0), (4.0, 0.0), (4.0, 1.0), (1.0, 1.0), (1.0, 4.0), (0.0, 4.0)];
        let l = Gate::polygon("X", "Y", &pts).unwrap();
        assert!(l.contains(0.5, 3.0) && l.contains(3.0, 0.5) && l.contains(0.5, 0.5));
        assert!(!l.contains(2.0, 2.0), "inside the notch");
        let rev: Vec<_> = pts.iter().rev().cloned().collect();
        let l2 = Gate::polygon("X", "Y", &rev).unwrap();
        for (x, y) in [(0.5, 3.0), (2.0, 2.0), (3.0, 0.5), (5.0, 5.0)] {
            assert_eq!(l.contains(x, y), l2.contains(x, y), "winding must not matter at ({x},{y})");
        }
    }

    #[test]
    fn legacy_quadrants_partition_the_plane() {
        let qs = [Quad::Q1, Quad::Q2, Quad::Q3, Quad::Q4];
        let gates: Vec<Gate> = qs.iter().map(|q| Gate::quadrant("X", "Y", *q, 1.0, 1.0).unwrap()).collect();
        for (x, y) in [(2.0, 2.0), (0.0, 2.0), (0.0, 0.0), (2.0, 0.0), (1.0, 1.0), (1.0, 5.0), (-3.0, 1.0)] {
            assert_eq!(gates.iter().filter(|g| g.contains(x, y)).count(), 1, "({x},{y}) must be in exactly one quadrant");
        }
        assert!(gates[0].contains(2.0, 2.0) && gates[1].contains(0.0, 2.0) && gates[2].contains(0.0, 0.0) && gates[3].contains(2.0, 0.0));
    }

    #[test]
    fn spider_quadrants_with_default_arms_match_the_legacy_layout_and_rotate_with_the_arms() {
        // centre (0,0); arm1 -> (1,0) (right), arm2 -> (0,1) (up), mirrors on the other side
        let spider = |q| Gate::new(GateKind::Quadrant(q), "X", Some("Y"), vec![0.0, 1.0, -1.0, 0.0, 0.0], vec![0.0, 0.0, 0.0, 1.0, -1.0]).unwrap();
        assert!(spider(Quad::Q1).contains(2.0, 2.0) && spider(Quad::Q2).contains(-2.0, 2.0));
        assert!(spider(Quad::Q3).contains(-2.0, -2.0) && spider(Quad::Q4).contains(2.0, -2.0));
        // swing arm1 to point up-right (45 deg): the point (2, 0.5) is now BELOW arm1
        let tilted = Gate::new(GateKind::Quadrant(Quad::Q4), "X", Some("Y"), vec![0.0, 1.0, -1.0, 0.0, 0.0], vec![0.0, 1.0, -1.0, 1.0, -1.0]).unwrap();
        assert!(tilted.contains(2.0, 0.5));
    }

    #[test]
    fn invert_flips_membership_and_nan_falls_inside_an_inverted_gate() {
        let g = Gate::rectangle("X", "Y", 0.0, 1.0, 0.0, 1.0).unwrap();
        let inv = g.clone().inverted();
        let data = [0.5f32, 0.5, 5.0, 5.0, f32::NAN, 0.5];
        let c = ch(&["X", "Y"]);
        let v = EventView::new(&data, 2, &c).unwrap();
        assert_eq!(g.mask(&v).unwrap().indices(), vec![0]);
        assert_eq!(inv.mask(&v).unwrap().indices(), vec![1, 2], "NaN satisfies no shape, so it is outside g and inside NOT g");
    }

    #[test]
    fn missing_channel_is_a_named_error_and_indexed_gates_check_their_rows() {
        let data = [1.0f32, 2.0];
        let c = ch(&["X"]);
        let v = EventView::new(&data, 1, &c).unwrap();
        let g = Gate::rectangle("X", "CD3", 0.0, 1.0, 0.0, 1.0).unwrap();
        assert_eq!(g.mask(&v).unwrap_err(), GateError::MissingChannel("CD3".into()));
        assert_eq!(Gate::indexed(vec![1]).mask(&v).unwrap().indices(), vec![1]);
        assert_eq!(Gate::indexed(vec![5]).mask(&v).unwrap_err(), GateError::RowOutOfRange { row: 5, n_events: 2 });
        assert_eq!(Gate::indexed(vec![1]).inverted().mask(&v).unwrap().indices(), vec![0]);
    }

    #[test]
    fn constructors_reject_nonsense_with_a_reason() {
        assert!(Gate::polygon("X", "Y", &[(0.0, 0.0), (1.0, 1.0)]).unwrap_err().to_string().contains("3 vertices"));
        assert!(Gate::rectangle("X", "Y", f64::NAN, 1.0, 0.0, 1.0).unwrap_err().to_string().contains("finite"));
        assert!(Gate::new(GateKind::Rectangle, "X", None, vec![0.0, 1.0], vec![0.0, 1.0]).unwrap_err().to_string().contains("y channel"));
        assert!(Gate::new(GateKind::Quadrant(Quad::Q1), "X", Some("Y"), vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 2.0]).is_err());
        assert!(EventView::new(&[1.0, 2.0, 3.0], 2, &ch(&["A", "B"])).is_err(), "ragged matrix");
    }
}
