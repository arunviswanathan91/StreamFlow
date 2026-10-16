//! A cytometry plot as a [`Figure`]: density image or histogram, axes with the same ticks as the screen, gates,
//! FMO reference lines and caption text. The figure can be written as SVG or PDF, or placed on a layout page.

use crate::axis::ticks;
use crate::density::{bin, hist_path, hist_ticks, histogram_curve, render_kind, unify_curves, HistCurve, HistStyle, PlotKind, PlotStyle, Range};
use crate::figure::{text_width, Align, Figure, Item, Rgba, Stroke, Text, VAlign, WHITE};
use sf_core::scale::ScaleSpec;
use sf_gating::{Gate, GateKind};

const INK: Rgba = [30, 30, 30, 255];
const MUTED: Rgba = [105, 105, 105, 255];
const GRID: Rgba = [230, 230, 230, 255];
const AXIS: Rgba = [120, 120, 120, 255];
const GATE: Rgba = [214, 39, 40, 255];
const GATE_TEXT: Rgba = [140, 20, 20, 255];
const FMO: Rgba = [142, 68, 173, 255];

/// A gate to draw, with the text shown beside it (e.g. its name and percentage).
#[derive(Debug, Clone)]
pub struct PlotGate {
    pub gate: Gate,
    pub label: Option<String>,
}

/// One sample drawn on the plot. With several layers the first is the base and the others are overlays.
#[derive(Debug, Clone)]
pub struct PlotLayer {
    pub points: Vec<(f64, f64)>,
    pub color: Rgba,
    pub label: String,
    /// A control layer is the same on every page; its legend entry is italic.
    pub control: bool,
}

/// A dashed FMO reference line at a display-space position on the X (`true`) or Y axis.
#[derive(Debug, Clone, Copy)]
pub struct FmoLine {
    pub on_x: bool,
    pub pos: f64,
}

pub struct PlotSpec<'a> {
    pub width: f32,
    pub height: f32,
    /// Lines of text above the plot (sample, population, ...).
    pub captions: Vec<String>,
    pub points: &'a [(f64, f64)],
    /// When not empty these are drawn instead of `points`.
    pub layers: Vec<PlotLayer>,
    /// List the layers in a legend (only when there is more than one).
    pub legend: bool,
    /// Histogram, contour and density options.
    pub style: PlotStyle,
    pub range: Range,
    pub kind: PlotKind,
    pub x_chan: String,
    pub y_chan: String,
    pub x_scale: ScaleSpec,
    pub y_scale: ScaleSpec,
    pub gates: Vec<PlotGate>,
    pub fmo: Vec<FmoLine>,
    /// Small text under the x-axis label (e.g. the event count).
    pub footer: Option<String>,
}

/// Pixel-per-point supersampling of the density image: crisp when printed, but bounded.
const SUPERSAMPLE: f32 = 2.0;
const MAX_CELLS: usize = 520;

struct Frame {
    l: f32,
    t: f32,
    w: f32,
    h: f32,
    range: Range,
}

impl Frame {
    fn x(&self, v: f64) -> f32 {
        self.l + ((v - self.range.x0) / (self.range.x1 - self.range.x0)) as f32 * self.w
    }
    fn y(&self, v: f64) -> f32 {
        self.t + self.h - ((v - self.range.y0) / (self.range.y1 - self.range.y0)) as f32 * self.h
    }
}

fn line(a: (f32, f32), b: (f32, f32), color: Rgba, width: f32) -> Item {
    Item::Line { a, b, stroke: Stroke::new(color, width) }
}

/// Which major tick labels to print when they would overlap: `(major, position, centre on the page, half extent,
/// label)` per tick. Zero is placed first, then the labels furthest from zero (1k and 100k matter more than 10 and
/// 100 on a small plot), and a label is dropped when it would touch one already placed. Ticks are still drawn.
fn declutter(ticks: &[(bool, f64, f32, f32, &str)]) -> Vec<bool> {
    let mut order: Vec<usize> = (0..ticks.len()).filter(|&i| ticks[i].0).collect();
    order.sort_by(|&a, &b| {
        let key = |i: usize| if ticks[i].4 == "0" { f64::INFINITY } else { ticks[i].1.abs() };
        key(b).total_cmp(&key(a))
    });
    let mut placed: Vec<(f32, f32)> = Vec::new();
    let mut show = vec![false; ticks.len()];
    for i in order {
        let (_, _, c, half, _) = ticks[i];
        let (lo, hi) = (c - half - 2.0, c + half + 2.0);
        if placed.iter().all(|&(a, b)| hi <= a || lo >= b) {
            placed.push((lo, hi));
            show[i] = true;
        }
    }
    show
}

pub fn plot_figure(spec: &PlotSpec) -> Figure {
    let mut fig = Figure::new(spec.width, spec.height);
    let hist = spec.kind == PlotKind::Histogram;
    let caption_h = spec.captions.len() as f32 * 10.5;
    let footer_h = if spec.footer.is_some() { 11.0 } else { 0.0 };
    let (l, r, b) = (46.0, 10.0, 34.0 + footer_h);
    let top = 4.0 + caption_h + if caption_h > 0.0 { 4.0 } else { 0.0 };
    let fr = Frame { l, t: top, w: (spec.width - l - r).max(20.0), h: (spec.height - top - b).max(20.0), range: spec.range };
    // adjunct histograms take a strip above and a strip to the right of the plot
    const ADJ: f32 = 34.0;
    let adjunct = spec.style.adjunct && !hist && spec.range.is_valid();
    let fr = if adjunct { Frame { l: fr.l, t: fr.t + ADJ + 3.0, w: (fr.w - ADJ - 3.0).max(20.0), h: (fr.h - ADJ - 3.0).max(20.0), range: fr.range } } else { fr };
    for (i, c) in spec.captions.iter().enumerate() {
        let mut t = Text::new(4.0, 4.0 + i as f32 * 10.5, c.clone(), if i == 0 { 8.0 } else { 7.2 }).align(Align::Left, VAlign::Top).color(if i == 0 { INK } else { MUTED });
        if i == 0 {
            t = t.bold();
        }
        fig.push(Item::Text(t));
    }
    fig.push(Item::Rect { x: fr.l, y: fr.t, w: fr.w, h: fr.h, fill: Some(WHITE), stroke: None });
    if !spec.range.is_valid() {
        fig.push(Item::Text(Text::new(fr.l + fr.w / 2.0, fr.t + fr.h / 2.0, "no events on these axes", 8.0).align(Align::Center, VAlign::Middle).color(MUTED)));
        fig.push(Item::Rect { x: fr.l, y: fr.t, w: fr.w, h: fr.h, fill: None, stroke: Some(Stroke::new(AXIS, 0.6)) });
        return fig;
    }

    // ---- the layers, and the histogram curves they make
    let style = &spec.style;
    let single = [PlotLayer { points: Vec::new(), color: [0, 100, 160, 255], label: String::new(), control: false }];
    let layers: Vec<(&[(f64, f64)], Rgba)> = if spec.layers.is_empty() { vec![(spec.points, single[0].color)] } else { spec.layers.iter().map(|l| (l.points.as_slice(), l.color)).collect() };
    let bins = (fr.w.round() as usize).clamp(32, 400);
    let mut curves: Vec<HistCurve> = if hist { layers.iter().map(|(p, _)| histogram_curve(p.iter().map(|q| q.0), spec.range.x0, spec.range.x1, bins, style)).collect() } else { vec![] };
    unify_curves(&mut curves);

    // ---- grid and ticks (under the data)
    let mut axes: Vec<Item> = Vec::new();
    let x_ticks = ticks(&spec.x_scale, spec.range.x0, spec.range.x1);
    let x_show = declutter(&x_ticks.iter().map(|t| (t.major, t.pos, fr.x(t.pos), text_width(&t.label, 6.8, false) / 2.0, t.label.as_str())).collect::<Vec<_>>());
    for (t, show) in x_ticks.into_iter().zip(x_show) {
        let x = fr.x(t.pos);
        if t.major {
            if style.grid {
                fig.push(line((x, fr.t), (x, fr.t + fr.h), GRID, 0.4));
            }
            axes.push(line((x, fr.t + fr.h), (x, fr.t + fr.h + 3.0), AXIS, 0.6));
            if show {
                axes.push(Item::Text(Text::new(x, fr.t + fr.h + 4.0, t.label, 6.8).align(Align::Center, VAlign::Top).color(INK)));
            }
        } else {
            axes.push(line((x, fr.t + fr.h), (x, fr.t + fr.h + 1.6), AXIS, 0.5));
        }
    }
    if !hist {
        let y_ticks = ticks(&spec.y_scale, spec.range.y0, spec.range.y1);
        let y_show = declutter(&y_ticks.iter().map(|t| (t.major, t.pos, fr.y(t.pos), 4.2, t.label.as_str())).collect::<Vec<_>>());
        for (t, show) in y_ticks.into_iter().zip(y_show) {
            let y = fr.y(t.pos);
            if t.major {
                if style.grid {
                    fig.push(line((fr.l, y), (fr.l + fr.w, y), GRID, 0.4));
                }
                axes.push(line((fr.l - 3.0, y), (fr.l, y), AXIS, 0.6));
                if show {
                    axes.push(Item::Text(Text::new(fr.l - 4.5, y, t.label, 6.8).align(Align::Right, VAlign::Middle).color(INK)));
                }
            } else {
                axes.push(line((fr.l - 1.6, y), (fr.l, y), AXIS, 0.5));
            }
        }
    } else if let Some(c) = curves.first() {
        // the histogram's own Y axis, in the unit the style chose
        for (frac, label) in hist_ticks(c) {
            let y = fr.t + fr.h - frac as f32 * fr.h;
            if style.grid {
                fig.push(line((fr.l, y), (fr.l + fr.w, y), GRID, 0.4));
            }
            axes.push(line((fr.l - 3.0, y), (fr.l, y), AXIS, 0.6));
            axes.push(Item::Text(Text::new(fr.l - 4.5, y, label, 6.8).align(Align::Right, VAlign::Middle).color(INK)));
        }
    }

    // ---- data
    let mut clipped: Vec<Item> = Vec::new();
    if hist {
        let base = fr.t + fr.h;
        let many = layers.len() > 1;
        for (c, (_, color)) in curves.iter().zip(&layers) {
            let path: Vec<(f32, f32)> = hist_path(c, style.hist_style).into_iter().map(|(xf, h)| (fr.l + xf as f32 * fr.w, base - h as f32 * fr.h)).collect();
            if path.is_empty() || c.total <= 0.0 {
                continue;
            }
            let (fill, edge) = if many { ([color[0], color[1], color[2], 45], *color) } else { ([style.hist_fill[0], style.hist_fill[1], style.hist_fill[2], style.hist_fill_alpha], [style.hist_line[0], style.hist_line[1], style.hist_line[2], 255]) };
            if style.hist_style != HistStyle::Line {
                let mut poly = vec![(path[0].0, base)];
                poly.extend(&path);
                poly.push((path[path.len() - 1].0, base));
                clipped.push(Item::Poly { pts: poly, closed: true, fill: Some(fill), stroke: None });
            }
            clipped.push(Item::Poly { pts: path, closed: false, fill: None, stroke: Some(Stroke::new(edge, if many { 1.0 } else { 0.8 })) });
        }
    } else {
        let gw = ((fr.w * SUPERSAMPLE) as usize).clamp(16, MAX_CELLS);
        let gh = ((fr.h * SUPERSAMPLE) as usize).clamp(16, MAX_CELLS);
        let grid = bin(layers[0].0.iter().copied(), spec.range, gw, gh);
        let px = render_kind(&grid, spec.kind, style);
        // composite on white: the figure's image is opaque RGB
        let mut rgb = Vec::with_capacity(gw * gh * 3);
        for p in &px {
            let a = p[3] as u32;
            for c in 0..3 {
                rgb.push(((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8);
            }
        }
        // overlays: the occupied cells of each further layer, in its own colour, on top of the base
        for (pts_l, color) in layers.iter().skip(1) {
            let g = bin(pts_l.iter().copied(), spec.range, gw, gh);
            for (i, p) in crate::density::render_dots(&g, [color[0], color[1], color[2]]).iter().enumerate() {
                if p[3] > 0 {
                    rgb[i * 3..i * 3 + 3].copy_from_slice(&p[..3]);
                }
            }
        }
        clipped.push(Item::Image { x: fr.l, y: fr.t, w: fr.w, h: fr.h, px_w: gw as u32, px_h: gh as u32, rgb });
    }

    // ---- gates and FMO lines
    for pg in &spec.gates {
        let (items, anchor) = gate_items(&pg.gate, &fr, hist);
        clipped.extend(items);
        if let (Some(label), Some((ax, ay))) = (&pg.label, anchor) {
            for (i, ln) in label.lines().enumerate() {
                clipped.push(Item::Text(Text::new(ax + 2.0, ay - 2.0 - (label.lines().count() - 1 - i) as f32 * 7.6, ln.to_string(), 6.6).align(Align::Left, VAlign::Bottom).color(GATE_TEXT)));
            }
        }
    }
    for f in &spec.fmo {
        let st = Stroke::dashed(FMO, 0.9, 4.0, 2.5);
        if f.on_x {
            let x = fr.x(f.pos);
            if x >= fr.l && x <= fr.l + fr.w {
                clipped.push(Item::Line { a: (x, fr.t), b: (x, fr.t + fr.h), stroke: st });
                clipped.push(Item::Text(Text::new(x + 2.0, fr.t + 2.0, "FMO", 6.2).align(Align::Left, VAlign::Top).color(FMO)));
            }
        } else if !hist {
            let y = fr.y(f.pos);
            if y >= fr.t && y <= fr.t + fr.h {
                clipped.push(Item::Line { a: (fr.l, y), b: (fr.l + fr.w, y), stroke: st });
                clipped.push(Item::Text(Text::new(fr.l + 2.0, y - 1.5, "FMO", 6.2).align(Align::Left, VAlign::Bottom).color(FMO)));
            }
        }
    }
    fig.push(Item::Clip { x: fr.l, y: fr.t, w: fr.w, h: fr.h, items: clipped });
    for a in axes {
        fig.push(a);
    }
    fig.push(Item::Rect { x: fr.l, y: fr.t, w: fr.w, h: fr.h, fill: None, stroke: Some(Stroke::new(AXIS, 0.6)) });
    if spec.legend && spec.layers.len() > 1 {
        let widest = spec.layers.iter().map(|l| text_width(&l.label, 6.6, false)).fold(0.0f32, f32::max);
        let (bw, bh) = (widest + 18.0, spec.layers.len() as f32 * 9.0 + 4.0);
        let (bx, by) = (fr.l + fr.w - bw - 4.0, fr.t + 4.0);
        fig.push(Item::Rect { x: bx, y: by, w: bw, h: bh, fill: Some([255, 255, 255, 215]), stroke: Some(Stroke::new([190, 190, 190, 255], 0.4)) });
        for (i, l) in spec.layers.iter().enumerate() {
            let y = by + 3.0 + i as f32 * 9.0;
            fig.push(Item::Rect { x: bx + 3.0, y: y + 0.5, w: 7.0, h: 5.0, fill: Some(l.color), stroke: None });
            let mut t = Text::new(bx + 13.0, y + 3.0, l.label.clone(), 6.6).align(Align::Left, VAlign::Middle).color(INK);
            if l.control {
                t = t.italic();
            }
            fig.push(Item::Text(t));
        }
    }
    fig.push(Item::Text(Text::new(fr.l + fr.w / 2.0, fr.t + fr.h + 15.0, spec.x_chan.clone(), 8.0).align(Align::Center, VAlign::Top).color(INK)));
    if hist {
        let label = curves.first().map_or("count", |c| c.y_label);
        fig.push(Item::Text(Text::new(10.0, fr.t + fr.h / 2.0, label, 8.0).align(Align::Center, VAlign::Middle).color(MUTED).vertical()));
    } else {
        fig.push(Item::Text(Text::new(10.0, fr.t + fr.h / 2.0, spec.y_chan.clone(), 8.0).align(Align::Center, VAlign::Middle).color(INK).vertical()));
    }
    if adjunct {
        // the X and Y distributions of the first layer, along the top and the right-hand edge
        let adj_style = PlotStyle { hist_y: crate::density::HistY::Mode, ..style.clone() };
        let (fill, edge) = ([style.hist_fill[0], style.hist_fill[1], style.hist_fill[2], style.hist_fill_alpha], [style.hist_line[0], style.hist_line[1], style.hist_line[2], 255]);
        let xs = histogram_curve(layers[0].0.iter().map(|q| q.0), spec.range.x0, spec.range.x1, bins, &adj_style);
        let ys = histogram_curve(layers[0].0.iter().map(|q| q.1), spec.range.y0, spec.range.y1, (fr.h.round() as usize).clamp(32, 400), &adj_style);
        let (top_base, right_base) = (fr.t - 3.0, fr.l + fr.w + 3.0);
        if xs.total > 0.0 {
            let path: Vec<(f32, f32)> = hist_path(&xs, style.hist_style).into_iter().map(|(xf, h)| (fr.l + xf as f32 * fr.w, top_base - h as f32 * ADJ)).collect();
            let mut poly = vec![(path[0].0, top_base)];
            poly.extend(&path);
            poly.push((path[path.len() - 1].0, top_base));
            fig.push(Item::Poly { pts: poly, closed: true, fill: Some(fill), stroke: None });
            fig.push(Item::Poly { pts: path, closed: false, fill: None, stroke: Some(Stroke::new(edge, 0.7)) });
        }
        if ys.total > 0.0 {
            let path: Vec<(f32, f32)> = hist_path(&ys, style.hist_style).into_iter().map(|(yf, h)| (right_base + h as f32 * ADJ, fr.t + fr.h - yf as f32 * fr.h)).collect();
            let mut poly = vec![(right_base, path[0].1)];
            poly.extend(&path);
            poly.push((right_base, path[path.len() - 1].1));
            fig.push(Item::Poly { pts: poly, closed: true, fill: Some(fill), stroke: None });
            fig.push(Item::Poly { pts: path, closed: false, fill: None, stroke: Some(Stroke::new(edge, 0.7)) });
        }
    }
    if let Some(f) = &spec.footer {
        fig.push(Item::Text(Text::new(fr.l + fr.w / 2.0, spec.height - 3.0, f.clone(), 7.0).align(Align::Center, VAlign::Bottom).color(MUTED)));
    }
    fig
}

/// The items of one gate in plot coordinates, and where its label goes.
fn gate_items(g: &Gate, fr: &Frame, hist: bool) -> (Vec<Item>, Option<(f32, f32)>) {
    let lo = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = |v: &[f64]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let tint: Rgba = [GATE[0], GATE[1], GATE[2], 28];
    let st = || Some(Stroke::new(GATE, 1.0));
    match &g.kind {
        GateKind::Rectangle => {
            let (x0, x1, y0, y1) = (fr.x(lo(&g.xs)), fr.x(hi(&g.xs)), fr.y(hi(&g.ys)), fr.y(lo(&g.ys)));
            (vec![Item::Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0, fill: None, stroke: st() }], Some((x0, y0)))
        }
        GateKind::Interval => {
            let (x0, x1) = (fr.x(lo(&g.xs)), fr.x(hi(&g.xs)));
            let mut v = Vec::new();
            if hist {
                v.push(Item::Rect { x: x0, y: fr.t, w: x1 - x0, h: fr.h, fill: Some(tint), stroke: None });
            }
            v.push(line((x0, fr.t), (x0, fr.t + fr.h), GATE, 1.0));
            v.push(line((x1, fr.t), (x1, fr.t + fr.h), GATE, 1.0));
            (v, Some((x0, fr.t + 9.0)))
        }
        GateKind::Ellipse => {
            let (cx, cy) = ((lo(&g.xs) + hi(&g.xs)) / 2.0, (lo(&g.ys) + hi(&g.ys)) / 2.0);
            let (rx, ry) = ((hi(&g.xs) - lo(&g.xs)) / 2.0, (hi(&g.ys) - lo(&g.ys)) / 2.0);
            let (c, s) = (g.angle.cos(), g.angle.sin());
            let pts: Vec<(f32, f32)> = (0..72)
                .map(|k| {
                    let t = k as f64 * std::f64::consts::TAU / 72.0;
                    let (ex, ey) = (rx * t.cos(), ry * t.sin());
                    (fr.x(cx + ex * c - ey * s), fr.y(cy + ex * s + ey * c))
                })
                .collect();
            let anchor = pts.iter().cloned().fold(None, |a: Option<(f32, f32)>, p| if a.map_or(true, |q| p.1 < q.1) { Some(p) } else { a });
            (vec![Item::Poly { pts, closed: true, fill: None, stroke: st() }], anchor)
        }
        GateKind::Polygon => {
            let pts: Vec<(f32, f32)> = g.xs.iter().zip(&g.ys).map(|(&x, &y)| (fr.x(x), fr.y(y))).collect();
            let anchor = pts.iter().cloned().fold(None, |a: Option<(f32, f32)>, p| if a.map_or(true, |q| p.1 < q.1) { Some(p) } else { a });
            (vec![Item::Poly { pts, closed: true, fill: None, stroke: st() }], anchor)
        }
        GateKind::Quadrant(_) => {
            let (x, y) = (fr.x(g.xs[0]), fr.y(g.ys[0]));
            (vec![line((x, fr.t), (x, fr.t + fr.h), GATE, 1.0), line((fr.l, y), (fr.l + fr.w, y), GATE, 1.0)], None)
        }
        GateKind::Indexed(_) => (vec![], None),
    }
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub fn demo_spec(pts: &[(f64, f64)]) -> PlotSpec<'_> {
        PlotSpec {
            width: 260.0,
            height: 220.0,
            captions: vec!["sample.fcs".into(), "Lymphocytes".into()],
            points: pts,
            layers: vec![],
            legend: false,
            style: PlotStyle::default(),
            range: Range { x0: 0.0, x1: 1000.0, y0: 0.0, y1: 500.0 },
            kind: PlotKind::Pseudocolor,
            x_chan: "FSC-A".into(),
            y_chan: "SSC-A".into(),
            x_scale: ScaleSpec::Identity,
            y_scale: ScaleSpec::Identity,
            gates: vec![],
            fmo: vec![],
            footer: Some("3,000 events".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure::{to_pdf, to_svg, validate_pdf};
    use sf_core::rng::Rng;

    fn points(n: usize) -> Vec<(f64, f64)> {
        let mut rng = Rng::new(5);
        (0..n).map(|_| (rng.next_f64() * 1000.0, rng.next_f64() * 500.0)).collect()
    }

    fn spec<'a>(pts: &'a [(f64, f64)], kind: PlotKind) -> PlotSpec<'a> {
        PlotSpec {
            width: 260.0,
            height: 220.0,
            captions: vec!["sample.fcs".into(), "Lymphocytes ▸ CD3+".into()],
            points: pts,
            layers: vec![],
            legend: true,
            style: PlotStyle::default(),
            range: Range { x0: 0.0, x1: 1000.0, y0: 0.0, y1: 500.0 },
            kind,
            x_chan: "FSC-A".into(),
            y_chan: "SSC-A".into(),
            x_scale: ScaleSpec::Identity,
            y_scale: ScaleSpec::Identity,
            gates: vec![],
            fmo: vec![],
            footer: Some("12,345 events".into()),
        }
    }

    fn images(f: &Figure) -> Vec<(u32, u32)> {
        fn walk(items: &[Item], out: &mut Vec<(u32, u32)>) {
            for i in items {
                match i {
                    Item::Image { px_w, px_h, .. } => out.push((*px_w, *px_h)),
                    Item::Clip { items, .. } => walk(items, out),
                    _ => {}
                }
            }
        }
        let mut v = Vec::new();
        walk(&f.items, &mut v);
        v
    }

    #[test]
    fn a_density_plot_has_the_image_axes_labels_and_captions() {
        let pts = points(20_000);
        let f = plot_figure(&spec(&pts, PlotKind::Pseudocolor));
        let texts = f.texts();
        for want in ["sample.fcs", "Lymphocytes ▸ CD3+", "FSC-A", "SSC-A", "12,345 events", "0", "200", "400", "600", "800"] {
            assert!(texts.iter().any(|t| t == want), "missing text {want:?} in {texts:?}");
        }
        // the density image covers the plot at 2 cells per point, and is opaque RGB the right size
        let im = images(&f);
        assert_eq!(im.len(), 1);
        assert_eq!(im[0], (((260.0 - 46.0 - 10.0) * 2.0) as u32, ((220.0 - (4.0 + 21.0 + 4.0) - 45.0) * 2.0) as u32));
        // it writes as valid SVG and PDF
        assert!(sf_core::xml::parse(&to_svg(&f)).is_ok());
        assert_eq!(validate_pdf(&to_pdf(&[f], "t")), Ok(1));
    }

    #[test]
    fn every_gate_kind_is_drawn_labelled_and_clipped_to_the_plot() {
        let pts = points(500);
        let mut s = spec(&pts, PlotKind::Pseudocolor);
        let rect = Gate::rectangle("FSC-A", "SSC-A", 100.0, 300.0, 50.0, 200.0).unwrap();
        let ell = Gate::ellipse("FSC-A", "SSC-A", 600.0, 250.0, 150.0, 80.0, 0.5).unwrap();
        let poly = Gate::polygon("FSC-A", "SSC-A", &[(700.0, 300.0), (900.0, 300.0), (800.0, 450.0)]).unwrap();
        let quad = Gate::quadrant("FSC-A", "SSC-A", sf_gating::Quad::Q1, 500.0, 250.0).unwrap();
        s.gates = vec![PlotGate { gate: rect, label: Some("Rect\n12.5%".into()) }, PlotGate { gate: ell, label: Some("Ell".into()) }, PlotGate { gate: poly, label: None }, PlotGate { gate: quad, label: None }, PlotGate { gate: Gate::indexed(vec![1, 2]), label: Some("skipped".into()) }];
        let f = plot_figure(&s);
        let t = f.texts();
        assert!(t.contains(&"Rect".to_string()) && t.contains(&"12.5%".to_string()) && t.contains(&"Ell".to_string()));
        assert!(!t.contains(&"skipped".to_string()), "an index-set gate has no geometry to draw");
        // 72-vertex ellipse polygon and a 3-vertex triangle exist inside the clip
        let clip = f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items) } else { None }).unwrap();
        let polys: Vec<usize> = clip.iter().filter_map(|i| if let Item::Poly { pts, .. } = i { Some(pts.len()) } else { None }).collect();
        assert!(polys.contains(&72) && polys.contains(&3), "{polys:?}");
        // the rectangle is where the data says: x 100..300 of 0..1000 across the 204pt-wide plot
        let r = clip.iter().find_map(|i| if let Item::Rect { x, w, .. } = i { Some((*x, *w)) } else { None }).unwrap();
        assert!((r.0 - (46.0 + 0.1 * 204.0)).abs() < 0.01 && (r.1 - 0.2 * 204.0).abs() < 0.01, "{r:?}");
        assert_eq!(validate_pdf(&to_pdf(&[f], "g")), Ok(1));
    }

    #[test]
    fn histograms_and_dot_plots_and_fmo_lines() {
        let pts = points(5000);
        let mut s = spec(&pts, PlotKind::Histogram);
        s.fmo = vec![FmoLine { on_x: true, pos: 400.0 }, FmoLine { on_x: false, pos: 100.0 }];
        s.gates = vec![PlotGate { gate: Gate::interval("FSC-A", 200.0, 500.0).unwrap(), label: Some("Band".into()) }];
        let f = plot_figure(&s);
        assert!(images(&f).is_empty(), "a histogram is vector");
        assert!(f.texts().contains(&"Normalized to mode".to_string()) && !f.texts().contains(&"SSC-A".to_string()), "{:?}", f.texts());
        assert!(f.texts().contains(&"100".to_string()) && f.texts().contains(&"0".to_string()), "a histogram has a real Y axis");
        let clip = f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items) } else { None }).unwrap();
        let dashed = clip.iter().filter(|i| matches!(i, Item::Line { stroke, .. } if stroke.dash.is_some())).count();
        assert_eq!(dashed, 1, "only the X-channel FMO line shows on a histogram");
        assert!(clip.iter().any(|i| matches!(i, Item::Poly { fill: Some(_), pts, .. } if pts.len() > 50)));
        let dot = plot_figure(&spec(&pts, PlotKind::Dot));
        assert_eq!(images(&dot).len(), 1);
        // an FMO line outside the view is not drawn
        let mut off = spec(&pts, PlotKind::Pseudocolor);
        off.fmo = vec![FmoLine { on_x: true, pos: 5000.0 }];
        let f = plot_figure(&off);
        let clip = f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items) } else { None }).unwrap();
        assert!(!clip.iter().any(|i| matches!(i, Item::Line { stroke, .. } if stroke.dash.is_some())));
    }

    #[test]
    fn an_empty_or_invalid_view_says_so_instead_of_drawing_nonsense() {
        let pts: Vec<(f64, f64)> = vec![];
        let mut s = spec(&pts, PlotKind::Pseudocolor);
        s.range = Range { x0: 5.0, x1: 5.0, y0: 0.0, y1: 1.0 };
        let f = plot_figure(&s);
        assert!(f.texts().contains(&"no events on these axes".to_string()));
        assert_eq!(validate_pdf(&to_pdf(&[f], "e")), Ok(1));
        // a valid view with no points still draws axes and a (blank white) image
        let f2 = plot_figure(&spec(&pts, PlotKind::Pseudocolor));
        assert!(f2.texts().contains(&"FSC-A".to_string()));
    }

    fn layer(pts: Vec<(f64, f64)>, color: Rgba, label: &str, control: bool) -> PlotLayer {
        PlotLayer { points: pts, color, label: label.into(), control }
    }

    #[test]
    fn histogram_overlays_are_one_coloured_outline_per_layer_each_scaled_to_its_own_peak() {
        let pts = points(10);
        let mut s = spec(&pts, PlotKind::Histogram);
        let wide: Vec<(f64, f64)> = (0..4000).map(|i| ((i % 1000) as f64, 0.0)).collect();
        let narrow: Vec<(f64, f64)> = (0..300).map(|i| (400.0 + (i % 50) as f64, 0.0)).collect();
        s.layers = vec![layer(wide, [31, 119, 180, 255], "sample A", false), layer(narrow, [214, 39, 40, 255], "control", true)];
        let f = plot_figure(&s);
        let clip = f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items) } else { None }).unwrap();
        let polys: Vec<(&Vec<(f32, f32)>, Rgba)> = clip.iter().filter_map(|i| if let Item::Poly { pts, stroke: Some(st), .. } = i { Some((pts, st.color)) } else { None }).collect();
        assert_eq!(polys.len(), 2);
        assert_eq!((polys[0].1, polys[1].1), ([31, 119, 180, 255], [214, 39, 40, 255]));
        // both curves reach the same height even though one layer has 13x fewer events
        let top = |p: &Vec<(f32, f32)>| p.iter().map(|q| q.1).fold(f32::INFINITY, f32::min);
        let (ta, tb) = (top(polys[0].0), top(polys[1].0));
        assert!((ta - tb).abs() < 1.0, "{ta} vs {tb}");
        // the legend lists both layers; the control is italic
        let texts: Vec<&Text> = f.items.iter().filter_map(|i| if let Item::Text(t) = i { Some(t) } else { None }).collect();
        assert!(texts.iter().any(|t| t.text == "sample A" && !t.italic) && texts.iter().any(|t| t.text == "control" && t.italic));
        // no legend for a single layer, or when switched off
        let mut one = spec(&pts, PlotKind::Histogram);
        one.layers = vec![layer(vec![(1.0, 1.0)], [0, 0, 0, 255], "only", false)];
        assert!(!plot_figure(&one).texts().contains(&"only".to_string()));
        s.legend = false;
        assert!(!plot_figure(&s).texts().contains(&"control".to_string()));
        assert_eq!(validate_pdf(&to_pdf(&[plot_figure(&{ let mut x = spec(&pts, PlotKind::Histogram); x.layers = s.layers.clone(); x })], "o")), Ok(1));
    }

    #[test]
    fn two_dimensional_overlays_paint_the_further_layers_in_their_colour_over_the_base() {
        let base = points(20_000);
        let pts = points(10);
        let mut s = spec(&pts, PlotKind::Pseudocolor);
        let cluster: Vec<(f64, f64)> = (0..200).map(|i| (800.0 + (i % 10) as f64, 400.0 + (i / 10) as f64)).collect();
        s.layers = vec![layer(base, [0, 0, 0, 255], "base", false), layer(cluster, [255, 0, 0, 255], "overlay", false)];
        let f = plot_figure(&s);
        let im = f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { items.iter().find_map(|x| if let Item::Image { px_w, px_h, rgb, .. } = x { Some((*px_w as usize, *px_h as usize, rgb.clone())) } else { None }) } else { None }).unwrap();
        let reds = im.2.chunks(3).filter(|p| p == &[255, 0, 0]).count();
        assert!(reds > 0 && reds < im.0 * im.1 / 20, "the overlay cluster is a small patch of pure red: {reds}");
        // the patch sits where the data says (x 800..810 of 0..1000 is the right-hand fifth, y 400..420 the top)
        let first = im.2.chunks(3).position(|p| p == &[255, 0, 0]).unwrap();
        let (col, row) = (first % im.0, first / im.0);
        assert!(col as f32 / im.0 as f32 > 0.75 && (row as f32 / im.1 as f32) < 0.25, "({col}, {row}) of {} x {}", im.0, im.1);
    }

    fn clip_items(f: &Figure) -> Vec<Item> {
        f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items.clone()) } else { None }).unwrap()
    }

    fn peaked(n: usize) -> Vec<(f64, f64)> {
        (0..n).map(|i| (500.0 + ((i * 37) % 200) as f64 - 100.0, 250.0 + ((i * 11) % 100) as f64 - 50.0)).collect()
    }

    #[test]
    fn the_histogram_y_axis_follows_the_chosen_unit_and_the_style_draws_fill_bars_or_line() {
        let pts = peaked(4000);
        for (y, label, top) in [(crate::density::HistY::Count, "Count", None), (crate::density::HistY::PercentMax, "% of max", Some("100")), (crate::density::HistY::UnitArea, "Unit area", None), (crate::density::HistY::Cdf, "Cumulative %", Some("100"))] {
            let mut s = spec(&pts, PlotKind::Histogram);
            s.style.hist_y = y;
            let f = plot_figure(&s);
            assert!(f.texts().contains(&label.to_string()), "{y:?}: {:?}", f.texts());
            if let Some(t) = top {
                assert!(f.texts().contains(&t.to_string()), "{y:?}");
            }
            assert_eq!(validate_pdf(&to_pdf(&[f], "h")), Ok(1));
        }
        // fill: a filled polygon plus an outline; line only: just the outline; bars: steps (two points per bin)
        let polys = |st: HistStyle| -> Vec<(usize, bool)> {
            let mut s = spec(&pts, PlotKind::Histogram);
            s.style.hist_style = st;
            clip_items(&plot_figure(&s)).iter().filter_map(|i| if let Item::Poly { pts, fill, .. } = i { Some((pts.len(), fill.is_some())) } else { None }).collect()
        };
        let filled = polys(HistStyle::Filled);
        assert_eq!(filled.iter().filter(|(_, f)| *f).count(), 1);
        assert_eq!(filled.len(), 2, "fill and outline");
        assert_eq!(polys(HistStyle::Line), vec![(filled[1].0, false)], "line only has no fill");
        let bars = polys(HistStyle::Bars);
        assert!(bars[1].0 > 2 * filled[1].0 - 4, "a step per bin has about twice the points: {bars:?} vs {filled:?}");
        // the style's colours are used
        let mut s = spec(&pts, PlotKind::Histogram);
        s.style.hist_fill = [10, 200, 10];
        s.style.hist_fill_alpha = 77;
        s.style.hist_line = [200, 10, 10];
        let items = clip_items(&plot_figure(&s));
        assert!(items.iter().any(|i| matches!(i, Item::Poly { fill: Some([10, 200, 10, 77]), .. })));
        assert!(items.iter().any(|i| matches!(i, Item::Poly { stroke: Some(st), .. } if st.color == [200, 10, 10, 255])));
    }

    #[test]
    fn smoothing_options_change_the_exported_curve() {
        let pts = peaked(600);
        let rough = |smooth: bool, bw: f64| -> f32 {
            let mut s = spec(&pts, PlotKind::Histogram);
            s.style.hist_smooth = smooth;
            s.style.hist_bandwidth = bw;
            let items = clip_items(&plot_figure(&s));
            let line = items.iter().find_map(|i| if let Item::Poly { pts, fill: None, .. } = i { Some(pts.clone()) } else { None }).unwrap();
            line.windows(2).map(|w| (w[1].1 - w[0].1).abs()).sum()
        };
        let (off, light, heavy) = (rough(false, 0.5), rough(true, 0.2), rough(true, 0.9));
        assert!(light < off && heavy < light, "{off} {light} {heavy}");
    }

    #[test]
    fn contour_density_and_zebra_export_as_images_and_contour_lines_carry_the_chosen_colour() {
        let pts = points(30_000);
        for k in [PlotKind::Contour, PlotKind::Density, PlotKind::Zebra] {
            let f = plot_figure(&spec(&pts, k));
            assert_eq!(images(&f).len(), 1, "{k:?}");
            assert_eq!(validate_pdf(&to_pdf(&[f], "c")), Ok(1));
        }
        let mut s = spec(&pts, PlotKind::Contour);
        s.style.contour_color = [250, 20, 20];
        let f = plot_figure(&s);
        let img = clip_items(&f).into_iter().find_map(|i| if let Item::Image { rgb, .. } = i { Some(rgb) } else { None }).unwrap();
        let non_white = img.chunks(3).filter(|p| p != &[255, 255, 255]).count();
        let red = img.chunks(3).filter(|p| p == &[250, 20, 20]).count();
        assert!(non_white > 100 && red == non_white, "a contour image is white with lines in the chosen colour: {red}/{non_white}");
        // more levels, more line
        let lines = |n: usize| {
            let mut s = spec(&pts, PlotKind::Contour);
            s.style.contour_levels = n;
            clip_items(&plot_figure(&s)).into_iter().find_map(|i| if let Item::Image { rgb, .. } = i { Some(rgb.chunks(3).filter(|p| p != &[255, 255, 255]).count()) } else { None }).unwrap()
        };
        assert!(lines(20) > lines(5));
    }

    #[test]
    fn adjunct_histograms_add_a_strip_above_and_to_the_right_and_shrink_the_plot() {
        let pts = peaked(3000);
        let plain = plot_figure(&spec(&pts, PlotKind::Pseudocolor));
        let mut s = spec(&pts, PlotKind::Pseudocolor);
        s.style.adjunct = true;
        let adj = plot_figure(&s);
        let plot_w = |f: &Figure| f.items.iter().find_map(|i| if let Item::Rect { w, stroke: Some(_), fill: None, .. } = i { Some(*w) } else { None }).unwrap();
        assert!(plot_w(&adj) < plot_w(&plain) - 30.0, "the plot gives up room for the strips");
        let top_level_polys = |f: &Figure| f.items.iter().filter(|i| matches!(i, Item::Poly { .. })).count();
        assert_eq!(top_level_polys(&plain), 0);
        assert_eq!(top_level_polys(&adj), 4, "a fill and an outline for each of X and Y");
        assert_eq!(validate_pdf(&to_pdf(&[adj], "a")), Ok(1));
        // a histogram has no adjuncts
        let mut h = spec(&pts, PlotKind::Histogram);
        h.style.adjunct = true;
        assert_eq!(top_level_polys(&plot_figure(&h)), 0);
    }

    #[test]
    fn crowded_tick_labels_keep_zero_and_the_larger_magnitudes() {
        // five labels 12 pt apart, each 10 pt wide plus a 4 pt gap: neighbours collide, labels two steps apart do not
        let t: Vec<(bool, f64, f32, f32, &str)> = vec![(true, -2.0, 0.0, 5.0, "-100"), (true, -1.0, 12.0, 5.0, "-10"), (true, 0.0, 24.0, 5.0, "0"), (true, 1.0, 36.0, 5.0, "10"), (true, 2.0, 48.0, 5.0, "100")];
        let show = declutter(&t);
        assert!(show[2], "zero is always kept");
        assert_eq!(show.iter().filter(|s| **s).count(), 3, "{show:?}");
        assert!(show[0] && show[4], "the outer, larger labels win over the ones next to zero: {show:?}");
        // well spaced labels all print; minor ticks never carry a label
        let wide: Vec<(bool, f64, f32, f32, &str)> = (0..5).map(|i| (true, i as f64, i as f32 * 40.0, 8.0, "1k")).collect();
        assert!(declutter(&wide).iter().all(|s| *s));
        assert_eq!(declutter(&[(false, 1.0, 0.0, 3.0, "")]), vec![false]);
    }

    #[test]
    fn no_two_printed_labels_of_a_real_scaled_axis_overlap() {
        let pts = points(100);
        let mut s = spec(&pts, PlotKind::Pseudocolor);
        s.width = 170.0;
        s.x_scale = crate::session::default_scale("CD3-A", 262_144.0);
        let t = s.x_scale.build().unwrap();
        s.range = Range { x0: t.forward(-5_000.0), x1: t.forward(262_144.0), y0: 0.0, y1: 500.0 };
        let f = plot_figure(&s);
        let mut boxes: Vec<(f32, f32)> = f.items.iter().filter_map(|i| match i {
            Item::Text(t) if t.size < 7.0 && t.align == Align::Center && t.valign == VAlign::Top => {
                let w = text_width(&t.text, t.size, false);
                Some((t.x - w / 2.0, t.x + w / 2.0))
            }
            _ => None,
        }).collect();
        boxes.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(boxes.len() >= 3, "{boxes:?}");
        for w in boxes.windows(2) {
            assert!(w[0].1 <= w[1].0 + 1e-3, "labels overlap: {w:?}");
        }
    }

    #[test]
    fn scaled_axes_get_decade_labels() {
        let pts = points(100);
        let mut s = spec(&pts, PlotKind::Pseudocolor);
        s.x_scale = crate::session::default_scale("CD3-A", 262_144.0);
        let t = s.x_scale.build().unwrap();
        s.range = Range { x0: t.forward(-500.0), x1: t.forward(200_000.0), y0: 0.0, y1: 500.0 };
        let f = plot_figure(&s);
        assert!(f.texts().iter().any(|t| t == "1k" || t == "10k" || t == "100"), "{:?}", f.texts());
    }
}
