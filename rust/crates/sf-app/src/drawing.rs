//! Gate drawing as a small state machine over display-space points (no GUI types), so the behaviour is
//! testable. The egui layer feeds it mouse events already converted to display coordinates.

use sf_core::scale::ScaleSpec;
use sf_gating::Gate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Pointer,
    Rectangle,
    Polygon,
    Ellipse,
    Interval,
    Quadrant,
}

impl Tool {
    pub const ALL: [Tool; 6] = [Tool::Pointer, Tool::Polygon, Tool::Rectangle, Tool::Ellipse, Tool::Interval, Tool::Quadrant];
    pub fn label(self) -> &'static str {
        match self {
            Tool::Pointer => "Pointer",
            Tool::Rectangle => "Rectangle",
            Tool::Polygon => "Polygon",
            Tool::Ellipse => "Ellipse",
            Tool::Interval => "Interval",
            Tool::Quadrant => "Quadrant",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Drawing {
    /// Anchor + current corner for drag tools; the vertices placed so far for polygons.
    pts: Vec<(f64, f64)>,
    /// Current mouse position (for rubber-banding).
    hover: Option<(f64, f64)>,
    dragging: bool,
}

/// What the user has drawn, in display space.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    Rect { x0: f64, x1: f64, y0: f64, y1: f64 },
    Ellipse { cx: f64, cy: f64, rx: f64, ry: f64 },
    Interval { x0: f64, x1: f64 },
    Polygon(Vec<(f64, f64)>),
    /// A cross-hair: four populations around the centre.
    Quadrant { cx: f64, cy: f64 },
}

impl Drawing {
    pub fn is_active(&self) -> bool {
        !self.pts.is_empty()
    }

    pub fn cancel(&mut self) {
        *self = Drawing::default();
    }

    pub fn points(&self) -> &[(f64, f64)] {
        &self.pts
    }

    pub fn hover(&self) -> Option<(f64, f64)> {
        self.hover
    }

    pub fn move_to(&mut self, p: (f64, f64)) {
        self.hover = Some(p);
        if self.dragging && self.pts.len() >= 2 {
            self.pts[1] = p;
        }
    }

    /// Mouse down. Drag tools start a drag; the polygon tool adds a vertex.
    pub fn press(&mut self, tool: Tool, p: (f64, f64)) {
        match tool {
            Tool::Pointer => {}
            Tool::Polygon => {
                self.pts.push(p);
                self.dragging = false;
            }
            _ => {
                self.pts = vec![p, p];
                self.dragging = true;
            }
        }
        self.hover = Some(p);
    }

    /// Mouse up for drag tools; returns the finished shape if the drag was not degenerate.
    pub fn release(&mut self, tool: Tool, p: (f64, f64)) -> Option<Shape> {
        if !self.dragging {
            return None;
        }
        self.pts[1] = p;
        let (a, b) = (self.pts[0], self.pts[1]);
        let shape = match tool {
            Tool::Rectangle => Some(Shape::Rect { x0: a.0.min(b.0), x1: a.0.max(b.0), y0: a.1.min(b.1), y1: a.1.max(b.1) }),
            Tool::Ellipse => Some(Shape::Ellipse { cx: (a.0 + b.0) / 2.0, cy: (a.1 + b.1) / 2.0, rx: (a.0 - b.0).abs() / 2.0, ry: (a.1 - b.1).abs() / 2.0 }),
            Tool::Interval => Some(Shape::Interval { x0: a.0.min(b.0), x1: a.0.max(b.0) }),
            Tool::Quadrant => Some(Shape::Quadrant { cx: b.0, cy: b.1 }),
            _ => None,
        };
        self.cancel();
        shape.filter(|s| match s {
            Shape::Rect { x0, x1, y0, y1 } => x1 > x0 && y1 > y0,
            Shape::Ellipse { rx, ry, .. } => *rx > 0.0 && *ry > 0.0,
            Shape::Interval { x0, x1 } => x1 > x0,
            Shape::Polygon(_) | Shape::Quadrant { .. } => true,
        })
    }

    /// Finish a polygon (double-click or Enter). Needs at least three distinct vertices.
    pub fn finish_polygon(&mut self) -> Option<Shape> {
        let mut v: Vec<(f64, f64)> = Vec::new();
        for &p in &self.pts {
            if v.last().map_or(true, |l| *l != p) {
                v.push(p); // a double-click adds the same vertex twice
            }
        }
        if v.len() > 1 && v.first() == v.last() {
            v.pop();
        }
        self.cancel();
        (v.len() >= 3).then_some(Shape::Polygon(v))
    }
}

/// Turn a drawn shape into a gate on the given axes; coordinates stay in display space and the gate
/// records the scales it was drawn on.
pub fn to_gate(shape: &Shape, x_chan: &str, y_chan: &str, xs: &ScaleSpec, ys: &ScaleSpec) -> Result<Gate, String> {
    let g = match shape {
        Shape::Rect { x0, x1, y0, y1 } => Gate::rectangle(x_chan, y_chan, *x0, *x1, *y0, *y1),
        Shape::Ellipse { cx, cy, rx, ry } => Gate::ellipse(x_chan, y_chan, *cx, *cy, *rx, *ry, 0.0),
        Shape::Interval { x0, x1 } => Gate::interval(x_chan, *x0, *x1),
        Shape::Polygon(v) => Gate::polygon(x_chan, y_chan, v),
        Shape::Quadrant { .. } => return Err("a quadrant makes four populations; use the quadrant command".into()),
    }
    .map_err(|e| e.to_string())?;
    // an interval has no y axis
    let ys = if matches!(shape, Shape::Interval { .. }) { ScaleSpec::Identity } else { ys.clone() };
    Ok(g.with_scales(xs.clone(), ys))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_a_rectangle_in_any_direction_gives_the_same_box() {
        for (a, b) in [((1.0, 2.0), (5.0, 8.0)), ((5.0, 8.0), (1.0, 2.0)), ((1.0, 8.0), (5.0, 2.0))] {
            let mut d = Drawing::default();
            d.press(Tool::Rectangle, a);
            assert!(d.is_active());
            d.move_to((3.0, 3.0));
            let s = d.release(Tool::Rectangle, b).unwrap();
            assert_eq!(s, Shape::Rect { x0: 1.0, x1: 5.0, y0: 2.0, y1: 8.0 });
            assert!(!d.is_active(), "state resets after a gate is made");
        }
    }

    #[test]
    fn a_click_without_a_drag_makes_no_gate() {
        let mut d = Drawing::default();
        d.press(Tool::Rectangle, (1.0, 1.0));
        assert_eq!(d.release(Tool::Rectangle, (1.0, 1.0)), None);
        d.press(Tool::Interval, (2.0, 0.0));
        assert_eq!(d.release(Tool::Interval, (2.0, 9.0)), None, "an interval needs x extent");
        d.press(Tool::Ellipse, (0.0, 0.0));
        assert_eq!(d.release(Tool::Ellipse, (4.0, 0.0)), None, "a flat ellipse has no area");
        assert_eq!(d.release(Tool::Pointer, (0.0, 0.0)), None);
    }

    #[test]
    fn ellipse_and_interval_come_from_the_drag_box() {
        let mut d = Drawing::default();
        d.press(Tool::Ellipse, (0.0, 0.0));
        assert_eq!(d.release(Tool::Ellipse, (4.0, 2.0)), Some(Shape::Ellipse { cx: 2.0, cy: 1.0, rx: 2.0, ry: 1.0 }));
        d.press(Tool::Interval, (3.0, 0.0));
        assert_eq!(d.release(Tool::Interval, (1.0, 7.0)), Some(Shape::Interval { x0: 1.0, x1: 3.0 }));
    }

    #[test]
    fn polygons_collect_vertices_ignore_double_click_duplicates_and_need_three() {
        let mut d = Drawing::default();
        for p in [(0.0, 0.0), (4.0, 0.0), (4.0, 3.0), (4.0, 3.0)] {
            d.press(Tool::Polygon, p);
        }
        assert_eq!(d.points().len(), 4);
        assert_eq!(d.finish_polygon(), Some(Shape::Polygon(vec![(0.0, 0.0), (4.0, 0.0), (4.0, 3.0)])));
        d.press(Tool::Polygon, (0.0, 0.0));
        d.press(Tool::Polygon, (1.0, 1.0));
        assert_eq!(d.finish_polygon(), None, "two points are not a polygon");
        assert!(!d.is_active());
        // closing onto the first vertex does not double it
        for p in [(0.0, 0.0), (2.0, 0.0), (1.0, 2.0), (0.0, 0.0)] {
            d.press(Tool::Polygon, p);
        }
        assert_eq!(d.finish_polygon().map(|s| if let Shape::Polygon(v) = s { v.len() } else { 0 }), Some(3));
    }

    #[test]
    fn gates_record_the_scales_they_were_drawn_on() {
        let sc = ScaleSpec::Arcsinh { cofactor: 150.0 };
        let g = to_gate(&Shape::Rect { x0: 0.0, x1: 1.0, y0: 0.0, y1: 1.0 }, "X", "Y", &sc, &ScaleSpec::Identity).unwrap();
        assert_eq!((g.x_scale.clone(), g.y_scale.clone()), (sc.clone(), ScaleSpec::Identity));
        let i = to_gate(&Shape::Interval { x0: 0.0, x1: 1.0 }, "X", "Y", &sc, &sc).unwrap();
        assert!(i.y_scale.is_identity() && i.y_chan.is_none(), "an interval has no y axis");
        assert!(to_gate(&Shape::Polygon(vec![(0.0, 0.0), (1.0, 1.0)]), "X", "Y", &sc, &sc).is_err());
    }
}
