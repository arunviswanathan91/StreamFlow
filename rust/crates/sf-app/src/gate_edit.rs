//! Moving and resizing an existing gate: which handle the pointer is on, and the gate that results from dragging
//! that handle. Pure geometry in the gate's own (display) coordinates, so it is testable without a window.
//!
//! * Rectangle: 4 corners and 4 edges resize, the body moves.
//! * Interval: the 2 edges resize, the body moves (y is ignored).
//! * Ellipse: 4 axis-end handles (along the ellipse's own, possibly rotated, axes) resize, the body moves.
//! * Polygon: each vertex moves on its own, the body moves.
//! * Quadrant and index-set gates have no editable geometry here (a quadrant is four linked gates).

use sf_gating::{Gate, GateKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Bottom,
    Top,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Body,
    Vertex(usize),
    Corner { right: bool, top: bool },
    Edge(Side),
    /// Ellipse axis end: `minor` selects the short axis, `positive` the end in the +direction.
    Axis { minor: bool, positive: bool },
}

fn lo(v: &[f64]) -> f64 {
    v.iter().cloned().fold(f64::INFINITY, f64::min)
}
fn hi(v: &[f64]) -> f64 {
    v.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
}

/// Ellipse centre, radii and axis directions `(major, minor)` in data space.
fn ellipse_frame(g: &Gate) -> ((f64, f64), (f64, f64), ((f64, f64), (f64, f64))) {
    let c = ((lo(&g.xs) + hi(&g.xs)) / 2.0, (lo(&g.ys) + hi(&g.ys)) / 2.0);
    let r = ((hi(&g.xs) - lo(&g.xs)) / 2.0, (hi(&g.ys) - lo(&g.ys)) / 2.0);
    let (cs, sn) = (g.angle.cos(), g.angle.sin());
    (c, r, ((cs, sn), (-sn, cs)))
}

/// The handles of a gate and where they sit. `y_mid` places an interval's handles (it has no y extent).
pub fn handles(g: &Gate, y_mid: f64) -> Vec<(Handle, (f64, f64))> {
    match &g.kind {
        GateKind::Rectangle => {
            let (x0, x1, y0, y1) = (lo(&g.xs), hi(&g.xs), lo(&g.ys), hi(&g.ys));
            let (xm, ym) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            vec![
                (Handle::Corner { right: false, top: false }, (x0, y0)),
                (Handle::Corner { right: true, top: false }, (x1, y0)),
                (Handle::Corner { right: false, top: true }, (x0, y1)),
                (Handle::Corner { right: true, top: true }, (x1, y1)),
                (Handle::Edge(Side::Left), (x0, ym)),
                (Handle::Edge(Side::Right), (x1, ym)),
                (Handle::Edge(Side::Bottom), (xm, y0)),
                (Handle::Edge(Side::Top), (xm, y1)),
            ]
        }
        GateKind::Interval => vec![(Handle::Edge(Side::Left), (lo(&g.xs), y_mid)), (Handle::Edge(Side::Right), (hi(&g.xs), y_mid))],
        GateKind::Ellipse => {
            let ((cx, cy), (rx, ry), (u, v)) = ellipse_frame(g);
            vec![
                (Handle::Axis { minor: false, positive: true }, (cx + rx * u.0, cy + rx * u.1)),
                (Handle::Axis { minor: false, positive: false }, (cx - rx * u.0, cy - rx * u.1)),
                (Handle::Axis { minor: true, positive: true }, (cx + ry * v.0, cy + ry * v.1)),
                (Handle::Axis { minor: true, positive: false }, (cx - ry * v.0, cy - ry * v.1)),
            ]
        }
        GateKind::Polygon => g.xs.iter().zip(&g.ys).enumerate().map(|(i, (x, y))| (Handle::Vertex(i), (*x, *y))).collect(),
        GateKind::Quadrant(_) | GateKind::Indexed(_) => vec![],
    }
}

/// The nearest handle within `tol` (data units on each axis) of `p`, if any.
pub fn hit_handle(g: &Gate, p: (f64, f64), tol: (f64, f64), y_mid: f64) -> Option<Handle> {
    let mut best: Option<(f64, Handle)> = None;
    for (h, at) in handles(g, y_mid) {
        let (dx, dy) = ((p.0 - at.0) / tol.0, (p.1 - at.1) / tol.1);
        // an interval's handle is a whole vertical line: only x matters
        let d = if matches!(g.kind, GateKind::Interval) { dx.abs() } else { (dx * dx + dy * dy).sqrt() };
        if d <= 1.0 && best.map_or(true, |(b, _)| d < b) {
            best = Some((d, h));
        }
    }
    best.map(|(_, h)| h)
}

/// What the pointer at `p` would grab: a handle if one is near, else the body of the gate.
pub fn hit(g: &Gate, p: (f64, f64), tol: (f64, f64), y_mid: f64) -> Option<Handle> {
    if let Some(h) = hit_handle(g, p, tol, y_mid) {
        return Some(h);
    }
    if !handles(g, y_mid).is_empty() && g.contains(p.0, p.1) != g.invert {
        return Some(Handle::Body);
    }
    None
}

/// The gate after dragging `handle` from `from` to `to` (both data coordinates). `None` if the result would be
/// degenerate or the gate cannot be edited this way. The original is never modified.
pub fn apply(g: &Gate, handle: Handle, from: (f64, f64), to: (f64, f64)) -> Option<Gate> {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    if !(dx.is_finite() && dy.is_finite()) {
        return None;
    }
    let mut out = g.clone();
    match (&g.kind, handle) {
        (GateKind::Rectangle | GateKind::Interval | GateKind::Ellipse | GateKind::Polygon, Handle::Body) => {
            out.xs.iter_mut().for_each(|v| *v += dx);
            if !matches!(g.kind, GateKind::Interval) {
                out.ys.iter_mut().for_each(|v| *v += dy);
            }
        }
        (GateKind::Rectangle, Handle::Corner { right, top }) => {
            let (mut x0, mut x1, mut y0, mut y1) = (lo(&g.xs), hi(&g.xs), lo(&g.ys), hi(&g.ys));
            if right { x1 += dx } else { x0 += dx }
            if top { y1 += dy } else { y0 += dy }
            out.xs = sorted(x0, x1)?;
            out.ys = sorted(y0, y1)?;
        }
        (GateKind::Rectangle, Handle::Edge(side)) => {
            let (mut x0, mut x1, mut y0, mut y1) = (lo(&g.xs), hi(&g.xs), lo(&g.ys), hi(&g.ys));
            match side {
                Side::Left => x0 += dx,
                Side::Right => x1 += dx,
                Side::Bottom => y0 += dy,
                Side::Top => y1 += dy,
            }
            out.xs = sorted(x0, x1)?;
            out.ys = sorted(y0, y1)?;
        }
        (GateKind::Interval, Handle::Edge(side)) => {
            let (mut x0, mut x1) = (lo(&g.xs), hi(&g.xs));
            match side {
                Side::Left => x0 += dx,
                Side::Right => x1 += dx,
                _ => return None,
            }
            out.xs = sorted(x0, x1)?;
        }
        (GateKind::Ellipse, Handle::Axis { minor, .. }) => {
            let ((cx, cy), (rx, ry), (u, v)) = ellipse_frame(g);
            let dir = if minor { v } else { u };
            // the new radius is how far along that axis the pointer now is from the centre
            let r = ((to.0 - cx) * dir.0 + (to.1 - cy) * dir.1).abs();
            let scale = rx.max(ry).max(1e-12);
            if r < 1e-6 * scale {
                return None;
            }
            let (rx2, ry2) = if minor { (rx, r) } else { (r, ry) };
            out.xs = vec![cx - rx2, cx + rx2];
            out.ys = vec![cy - ry2, cy + ry2];
        }
        (GateKind::Polygon, Handle::Vertex(i)) if i < g.xs.len() => {
            out.xs[i] += dx;
            out.ys[i] += dy;
        }
        _ => return None,
    }
    if out.xs.iter().chain(&out.ys).any(|v| !v.is_finite()) {
        return None;
    }
    Some(out)
}

/// `[min, max]` of the two, refusing a zero-width result.
fn sorted(a: f64, b: f64) -> Option<Vec<f64>> {
    let (l, h) = (a.min(b), a.max(b));
    if h - l <= 1e-12 * (l.abs().max(h.abs()).max(1.0)) {
        None
    } else {
        Some(vec![l, h])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Gate {
        Gate::rectangle("X", "Y", 2.0, 6.0, 1.0, 5.0).unwrap()
    }

    #[test]
    fn rectangle_corners_edges_and_body() {
        let g = rect();
        let grown = apply(&g, Handle::Corner { right: true, top: true }, (6.0, 5.0), (8.0, 9.0)).unwrap();
        assert_eq!((grown.xs.clone(), grown.ys.clone()), (vec![2.0, 8.0], vec![1.0, 9.0]));
        let edge = apply(&g, Handle::Edge(Side::Left), (2.0, 3.0), (0.5, 100.0)).unwrap();
        assert_eq!((edge.xs.clone(), edge.ys.clone()), (vec![0.5, 6.0], vec![1.0, 5.0]), "an edge moves only its own side; the other axis is untouched");
        let moved = apply(&g, Handle::Body, (4.0, 3.0), (5.0, 1.0)).unwrap();
        assert_eq!((moved.xs.clone(), moved.ys.clone()), (vec![3.0, 7.0], vec![-1.0, 3.0]));
        // dragging a corner across the opposite one flips the rectangle instead of inverting it
        let flipped = apply(&g, Handle::Corner { right: false, top: false }, (2.0, 1.0), (9.0, 8.0)).unwrap();
        assert_eq!((flipped.xs, flipped.ys), (vec![6.0, 9.0], vec![5.0, 8.0]));
        // collapsing to zero width is refused
        assert!(apply(&g, Handle::Edge(Side::Left), (2.0, 3.0), (6.0, 3.0)).is_none());
        // the original is untouched
        assert_eq!(g.xs, vec![2.0, 6.0]);
    }

    #[test]
    fn interval_edges_and_body_ignore_y() {
        let g = Gate::interval("X", 10.0, 20.0).unwrap();
        let r = apply(&g, Handle::Edge(Side::Right), (20.0, 0.0), (25.0, 99.0)).unwrap();
        assert_eq!(r.xs, vec![10.0, 25.0]);
        let m = apply(&g, Handle::Body, (15.0, 0.0), (12.0, 99.0)).unwrap();
        assert_eq!((m.xs, m.ys), (vec![7.0, 17.0], vec![]));
        assert!(apply(&g, Handle::Edge(Side::Top), (0.0, 0.0), (1.0, 1.0)).is_none());
    }

    #[test]
    fn ellipse_resizes_along_its_own_rotated_axes() {
        let angle = std::f64::consts::FRAC_PI_4;
        let g = Gate::ellipse("X", "Y", 10.0, 10.0, 4.0, 2.0, angle).unwrap();
        // major axis end sits at centre + 4 * (cos45, sin45)
        let hs = handles(&g, 0.0);
        let (_, at) = hs.iter().find(|(h, _)| *h == Handle::Axis { minor: false, positive: true }).unwrap();
        assert!((at.0 - (10.0 + 4.0 * angle.cos())).abs() < 1e-12 && (at.1 - (10.0 + 4.0 * angle.sin())).abs() < 1e-12);
        // dragging that handle out to distance 6 along the axis makes the major radius 6 and leaves the minor alone
        let to = (10.0 + 6.0 * angle.cos(), 10.0 + 6.0 * angle.sin());
        let r = apply(&g, Handle::Axis { minor: false, positive: true }, *at, to).unwrap();
        assert!((hi(&r.xs) - lo(&r.xs) - 12.0).abs() < 1e-9, "{:?}", r.xs);
        assert!((hi(&r.ys) - lo(&r.ys) - 4.0).abs() < 1e-9, "minor radius unchanged: {:?}", r.ys);
        assert_eq!(r.angle, angle, "rotation is kept");
        // pulling the minor handle through the centre cannot make a zero-size ellipse
        let (_, minor_at) = hs.iter().find(|(h, _)| *h == Handle::Axis { minor: true, positive: true }).unwrap();
        assert!(apply(&g, Handle::Axis { minor: true, positive: true }, *minor_at, (10.0, 10.0)).is_none());
    }

    #[test]
    fn polygon_vertices_move_individually() {
        let g = Gate::polygon("X", "Y", &[(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)]).unwrap();
        let r = apply(&g, Handle::Vertex(2), (5.0, 8.0), (6.0, 12.0)).unwrap();
        assert_eq!((r.xs, r.ys), (vec![0.0, 10.0, 6.0], vec![0.0, 0.0, 12.0]));
        assert!(apply(&g, Handle::Vertex(7), (0.0, 0.0), (1.0, 1.0)).is_none());
    }

    #[test]
    fn hit_testing_prefers_handles_over_the_body_and_is_none_outside() {
        let g = rect();
        let tol = (0.3, 0.3);
        assert_eq!(hit(&g, (6.1, 5.1), tol, 0.0), Some(Handle::Corner { right: true, top: true }));
        assert_eq!(hit(&g, (2.0, 3.1), tol, 0.0), Some(Handle::Edge(Side::Left)));
        assert_eq!(hit(&g, (4.0, 3.0), tol, 0.0), Some(Handle::Body));
        assert_eq!(hit(&g, (9.0, 9.0), tol, 0.0), None);
        // an inverted gate's body is everything outside it
        let mut inv = rect();
        inv.invert = true;
        assert_eq!(hit(&inv, (9.0, 9.0), tol, 0.0), Some(Handle::Body));
        // interval handles are whole lines: any y near the edge grabs it
        let iv = Gate::interval("X", 10.0, 20.0).unwrap();
        assert_eq!(hit(&iv, (20.1, -500.0), (0.3, 0.3), 0.0), Some(Handle::Edge(Side::Right)));
        // quadrants and index sets are not editable here
        let q = Gate::quadrant("X", "Y", sf_gating::Quad::Q1, 5.0, 5.0).unwrap();
        assert_eq!(hit(&q, (6.0, 6.0), tol, 0.0), None);
        assert!(apply(&q, Handle::Body, (0.0, 0.0), (1.0, 1.0)).is_none());
    }

    #[test]
    fn a_move_changes_membership_by_exactly_the_events_that_crossed_the_edge() {
        use sf_gating::EventView;
        let channels = vec!["X".to_string(), "Y".to_string()];
        // a 10 x 10 grid of events at integer positions
        let data: Vec<f32> = (0..10).flat_map(|i| (0..10).flat_map(move |j| [i as f32, j as f32])).collect();
        let view = EventView::new(&data, 2, &channels).unwrap();
        let g = Gate::rectangle("X", "Y", 1.5, 4.5, 1.5, 4.5).unwrap(); // 3 x 3 events
        let before = g.mask(&view).unwrap().count();
        assert_eq!(before, 9);
        let moved = apply(&g, Handle::Body, (3.0, 3.0), (4.0, 3.0)).unwrap(); // one column to the right
        assert_eq!(moved.mask(&view).unwrap().count(), 9);
        let grown = apply(&g, Handle::Edge(Side::Right), (4.5, 3.0), (5.5, 3.0)).unwrap(); // one more column
        assert_eq!(grown.mask(&view).unwrap().count(), 12);
    }
}
