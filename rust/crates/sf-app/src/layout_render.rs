//! Rendering a [`LayoutDoc`]: one page per batch unit (a sample, or a group of samples sharing a keyword value).
//!
//! * A tile's **control** layers show the same sample on every page; its **iterator** layers show the page's samples.
//! * A tile with [`ViewMode::Lock`] gets ONE view for the whole batch (the union of everything it will ever show), so
//!   page 1 and page 50 can be compared. [`ViewMode::Fixed`] uses the numbers given.
//! * Text and titles may hold tokens: `[File_Name]` (the page's first sample), `[Keyword]` (the group's value),
//!   `[Group_Size]`, `[Population]`, `[Events]`, `[Percent_Parent]`, `[Page]`, `[Pages]`.

use crate::density::{PlotKind, Range};
use crate::figure::{Align, Figure, Item, Text, VAlign};
use crate::layout::expand_tokens;
use crate::layout_doc::{Iterate, LayerRole, LayoutDoc, Tile, ViewMode, PALETTE};
use crate::plot_figure::{plot_figure, FmoLine, PlotGate, PlotLayer, PlotSpec};
use crate::session::{default_scale, Session};
use crate::stats_table::pop_label;
use sf_core::scale::ScaleSpec;
use sf_gating::{Gate, GateKind, GateTree, NodeKind};
use std::collections::HashMap;

/// What one page shows: the samples that replace the iterator layers.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub samples: Vec<usize>,
    /// The sample's name, or the keyword value of the group.
    pub label: String,
    pub keyword: Option<String>,
}

pub fn keyword_value(session: &Session, sample: usize, key: &str) -> Option<String> {
    let want = key.trim().trim_start_matches('$').to_uppercase();
    session.sample(sample).keywords.iter().find(|(k, _)| k.trim().trim_start_matches('$').to_uppercase() == want).map(|(_, v)| v.trim().to_string())
}

fn sample_index(session: &Session, name: &str) -> Option<usize> {
    session.samples().iter().position(|s| s.name == name)
}

/// The pages of a batch.
pub fn units(session: &Session, doc: &LayoutDoc) -> Result<Vec<Unit>, String> {
    let mut chosen: Vec<usize> = if doc.samples.is_empty() { (0..session.samples().len()).collect() } else { doc.samples.iter().filter_map(|n| sample_index(session, n)).collect() };
    chosen.dedup();
    if chosen.is_empty() {
        return Err("there are no samples to iterate over".into());
    }
    match &doc.iterate {
        Iterate::Sample => Ok(chosen.into_iter().map(|i| Unit { samples: vec![i], label: session.sample(i).name.trim_end_matches(".fcs").to_string(), keyword: None }).collect()),
        Iterate::Keyword(k) => {
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            for i in chosen {
                let v = keyword_value(session, i, k).unwrap_or_else(|| "(no value)".to_string());
                match groups.iter_mut().find(|(g, _)| *g == v) {
                    Some((_, m)) => m.push(i),
                    None => groups.push((v, vec![i])),
                }
            }
            Ok(groups.into_iter().map(|(v, m)| Unit { samples: m, label: v, keyword: Some(k.clone()) }).collect())
        }
    }
}

/// A layer once the page's samples are known.
struct Resolved {
    sample: usize,
    color: [u8; 4],
    control: bool,
}

fn palette(i: usize) -> [u8; 4] {
    let c = PALETTE[i % PALETTE.len()];
    [c[0], c[1], c[2], 255]
}

fn resolve(session: &Session, tile: &Tile, unit: Option<&Unit>, notes: &mut Vec<String>) -> Vec<Resolved> {
    let mut out: Vec<Resolved> = Vec::new();
    for l in &tile.layers {
        let push = |out: &mut Vec<Resolved>, s: usize, color: usize, control: bool| {
            if !out.iter().any(|r| r.sample == s) {
                out.push(Resolved { sample: s, color: palette(color), control });
            }
        };
        match (l.role, unit) {
            (LayerRole::Iterator, Some(u)) => {
                for (k, &s) in u.samples.iter().enumerate() {
                    push(&mut out, s, l.color + k, false);
                }
            }
            _ => match sample_index(session, &l.sample) {
                Some(s) => push(&mut out, s, l.color, l.role == LayerRole::Control),
                None => notes.push(format!("tile {}: sample '{}' is not loaded", tile.id, l.sample)),
            },
        }
    }
    out
}

struct Axes {
    x_chan: String,
    y_chan: String,
    x_scale: ScaleSpec,
    y_scale: ScaleSpec,
}

fn axes(session: &Session, tile: &Tile, first: Option<usize>) -> Axes {
    let s0 = session.sample(first.unwrap_or(0).min(session.samples().len().saturating_sub(1)));
    let pick = |want: &str, prefer: &str, alt: usize| -> String {
        if !want.is_empty() {
            return want.to_string();
        }
        s0.channels.iter().find(|c| c.eq_ignore_ascii_case(prefer)).or(s0.channels.get(alt)).or(s0.channels.first()).cloned().unwrap_or_default()
    };
    let (x_chan, y_chan) = (pick(&tile.x_chan, "FSC-A", 0), pick(&tile.y_chan, "SSC-A", 1));
    Axes { x_scale: tile.x_scale.clone().unwrap_or_else(|| default_scale(&x_chan, s0.range_of(&x_chan))), y_scale: tile.y_scale.clone().unwrap_or_else(|| default_scale(&y_chan, s0.range_of(&y_chan))), x_chan, y_chan }
}

fn node_of(session: &Session, sample: usize, population: &[String]) -> Option<usize> {
    if population.is_empty() {
        return Some(GateTree::ROOT);
    }
    let refs: Vec<&str> = population.iter().map(String::as_str).collect();
    session.tree(sample).find(&refs)
}

fn on_axes(g: &Gate, ax: &Axes, kind: PlotKind) -> bool {
    if matches!(g.kind, GateKind::Indexed(_)) || g.x_chan != ax.x_chan || g.x_scale != ax.x_scale {
        return false;
    }
    match &g.y_chan {
        Some(y) => kind != PlotKind::Histogram && *y == ax.y_chan && g.y_scale == ax.y_scale,
        None => true,
    }
}

/// The view each locked tile uses: the union of everything its layers show on any page of the batch.
pub fn locked_ranges(session: &mut Session, doc: &LayoutDoc, units: &[Unit]) -> HashMap<u32, Range> {
    let mut out = HashMap::new();
    for tile in doc.tiles.iter().filter(|t| t.view == ViewMode::Lock) {
        let mut samples: Vec<usize> = Vec::new();
        let mut sink = Vec::new();
        for u in units {
            for r in resolve(session, tile, Some(u), &mut sink) {
                if !samples.contains(&r.sample) {
                    samples.push(r.sample);
                }
            }
        }
        if let Some(r) = union_range(session, tile, &samples) {
            out.insert(tile.id, r);
        }
    }
    out
}

fn union_range(session: &mut Session, tile: &Tile, samples: &[usize]) -> Option<Range> {
    let ax = axes(session, tile, samples.first().copied());
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for &s in samples {
        let Some(node) = node_of(session, s, &tile.population) else { continue };
        let Ok(pts) = session.population_points(s, node, &ax.x_chan, &ax.y_chan, &ax.x_scale, &ax.y_scale) else { continue };
        for (a, b) in pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()) {
            x0 = x0.min(*a);
            x1 = x1.max(*a);
            y0 = y0.min(*b);
            y1 = y1.max(*b);
        }
    }
    if !(x1 > x0) {
        return None;
    }
    // a histogram has no meaningful y; any valid span will do
    if !(y1 > y0) {
        y0 = 0.0;
        y1 = 1.0;
    }
    let (px, py) = (0.01 * (x1 - x0), 0.01 * (y1 - y0));
    Some(Range { x0: x0 - px, x1: x1 + px, y0: y0 - py, y1: y1 + py })
}

/// One page. `unit` is `None` in the editor, which shows each layer's own sample. `locked` comes from
/// [`locked_ranges`]; a locked tile missing from it falls back to the union of what this page shows.
pub fn render_page(session: &mut Session, doc: &LayoutDoc, unit: Option<&Unit>, locked: &HashMap<u32, Range>, page: usize, pages: usize) -> (Figure, Vec<String>) {
    let p = doc.page();
    let mut fig = Figure::new(p.width, p.height);
    let mut notes = Vec::new();
    let first_sample = unit.map(|u| u.samples[0]).or_else(|| doc.tiles.iter().flat_map(|t| t.layers.iter()).find_map(|l| sample_index(session, &l.sample)));
    for tile in &doc.tiles {
        let layers = resolve(session, tile, unit, &mut notes);
        let ax = axes(session, tile, layers.first().map(|r| r.sample).or(first_sample));
        let mut plot_layers: Vec<PlotLayer> = Vec::new();
        let mut gates: Vec<PlotGate> = Vec::new();
        let (mut events, mut parent_pct, mut first_name) = (0usize, 0.0f64, String::new());
        let mut used: Vec<usize> = Vec::new();
        for r in &layers {
            let name = session.sample(r.sample).name.clone();
            let Some(node) = node_of(session, r.sample, &tile.population) else {
                notes.push(format!("tile {}: '{}' has no population {}", tile.id, name, pop_label(&tile.population)));
                continue;
            };
            let pts = match session.population_points(r.sample, node, &ax.x_chan, &ax.y_chan, &ax.x_scale, &ax.y_scale) {
                Ok(p) => p,
                Err(e) => {
                    notes.push(format!("tile {}: {name}: {e}", tile.id));
                    continue;
                }
            };
            if plot_layers.is_empty() {
                first_name = name.clone();
                events = session.count(r.sample, node);
                let parent = session.tree(r.sample).parent(node).ok().flatten().map(|q| session.count(r.sample, q)).unwrap_or(session.sample(r.sample).n_events).max(1);
                parent_pct = 100.0 * events as f64 / parent as f64;
                if tile.show_gates {
                    for k in session.tree(r.sample).children(node).map(|c| c.to_vec()).unwrap_or_default() {
                        let (Ok(gname), Ok(NodeKind::Gate(g))) = (session.tree(r.sample).name(k).map(str::to_string), session.tree(r.sample).kind(k).cloned()) else { continue };
                        if on_axes(&g, &ax, tile.kind) {
                            let pct = 100.0 * session.count(r.sample, k) as f64 / events.max(1) as f64;
                            gates.push(PlotGate { gate: g, label: Some(format!("{gname}\n{pct:.1}%")) });
                        }
                    }
                }
            }
            used.push(r.sample);
            plot_layers.push(PlotLayer { points: pts, color: r.color, label: name.trim_end_matches(".fcs").to_string(), control: r.control });
        }
        let range = match &tile.view {
            ViewMode::Fixed(r) => Some(*r),
            ViewMode::Lock => locked.get(&tile.id).copied().or_else(|| union_range(session, tile, &used)),
        };
        let vars = vec![
            ("File_Name", first_name.trim_end_matches(".fcs").to_string()),
            ("Population", pop_label(&tile.population)),
            ("Events", events.to_string()),
            ("Percent_Parent", format!("{parent_pct:.1}")),
            ("Keyword", unit.and_then(|u| u.keyword.as_ref().map(|_| u.label.clone())).unwrap_or_default()),
            ("Group_Size", unit.map_or(1, |u| u.samples.len()).to_string()),
            ("Page", page.to_string()),
            ("Pages", pages.to_string()),
        ];
        let captions: Vec<String> = expand_tokens(&tile.title, &vars).lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
        let mut fmo = Vec::new();
        if tile.show_fmo {
            if let (Some(f), Ok(t)) = (session.fmo(&ax.x_chan), ax.x_scale.build()) {
                fmo.push(FmoLine { on_x: true, pos: t.forward(f.level) });
            }
            if tile.kind != PlotKind::Histogram {
                if let (Some(f), Ok(t)) = (session.fmo(&ax.y_chan), ax.y_scale.build()) {
                    fmo.push(FmoLine { on_x: false, pos: t.forward(f.level) });
                }
            }
        }
        let empty = plot_layers.is_empty();
        let spec = PlotSpec {
            width: tile.w,
            height: tile.h,
            captions,
            points: &[],
            layers: plot_layers,
            legend: tile.legend,
            range: range.unwrap_or(Range { x0: 0.0, x1: 0.0, y0: 0.0, y1: 0.0 }),
            kind: tile.kind,
            x_chan: ax.x_chan.clone(),
            y_chan: ax.y_chan.clone(),
            x_scale: ax.x_scale.clone(),
            y_scale: ax.y_scale.clone(),
            gates,
            fmo,
            footer: if empty { None } else { Some(format!("{events} events · {parent_pct:.1}% of parent")) },
        };
        let mut f = plot_figure(&spec);
        if empty {
            f.push(Item::Text(Text::new(tile.w / 2.0 + 18.0, tile.h / 2.0, if tile.layers.is_empty() { "drop a sample here" } else { "nothing to draw" }, 8.0).align(Align::Center, VAlign::Middle).color([150, 150, 150, 255])));
        }
        fig.place(f, tile.x, tile.y);
    }
    let page_vars = vec![
        ("File_Name", first_sample.map(|s| session.sample(s).name.trim_end_matches(".fcs").to_string()).unwrap_or_default()),
        ("Keyword", unit.and_then(|u| u.keyword.as_ref().map(|_| u.label.clone())).unwrap_or_default()),
        ("Group_Size", unit.map_or(1, |u| u.samples.len()).to_string()),
        ("Population", doc.tiles.first().map(|t| pop_label(&t.population)).unwrap_or_default()),
        ("Page", page.to_string()),
        ("Pages", pages.to_string()),
    ];
    for t in &doc.texts {
        let mut text = Text::new(t.x, t.y, expand_tokens(&t.text, &page_vars), t.size).align(Align::Left, VAlign::Top);
        if t.bold {
            text = text.bold();
        }
        if t.italic {
            text = text.italic();
        }
        fig.push(Item::Text(text));
    }
    (fig, notes)
}

/// Every page of a batch, and what could not be drawn.
pub fn render_batch(session: &mut Session, doc: &LayoutDoc) -> Result<(Vec<Figure>, Vec<String>), String> {
    if doc.tiles.is_empty() && doc.texts.is_empty() {
        return Err("the layout is empty: drop a sample onto the canvas first".into());
    }
    let us = units(session, doc)?;
    let locked = locked_ranges(session, doc, &us);
    let n = us.len();
    let (mut pages, mut notes) = (Vec::with_capacity(n), Vec::new());
    for (i, u) in us.iter().enumerate() {
        let (f, mut nt) = render_page(session, doc, Some(u), &locked, i + 1, n);
        nt.iter_mut().for_each(|s| *s = format!("page {}: {s}", i + 1));
        pages.push(f);
        notes.extend(nt);
    }
    Ok((pages, notes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::figure::{to_pdf, validate_pdf};
    use crate::layout_doc::{Layer, LayoutDoc};
    use crate::sample::LoadedSample;

    /// Four samples; sample k has CD3 shifted by 100*k. Samples a,b have group "x", c,d have group "y".
    fn session() -> Session {
        let mut s = Session::new();
        for (k, (name, grp)) in [("a.fcs", "x"), ("b.fcs", "x"), ("c.fcs", "y"), ("d.fcs", "y")].iter().enumerate() {
            let ev: Vec<f32> = (0..1000).flat_map(|i| [i as f32, 50.0 + (i % 100) as f32 + 100.0 * k as f32]).collect();
            let mut smp = LoadedSample::synthetic(name, &["FSC-A", "CD3-A"], vec![2000.0, 2000.0], ev);
            smp.keywords.insert("$GRP".into(), grp.to_string());
            s.add_sample(smp);
        }
        s
    }

    fn doc_with_tile(kind: PlotKind) -> LayoutDoc {
        let mut d = LayoutDoc::new("t");
        let id = d.add_tile((300.0, 250.0), 240.0, 200.0);
        let t = d.tile_mut(id).unwrap();
        t.x_chan = "CD3-A".into();
        t.y_chan = "FSC-A".into();
        t.x_scale = Some(ScaleSpec::Identity);
        t.y_scale = Some(ScaleSpec::Identity);
        t.kind = kind;
        t.add_layer("a.fcs", LayerRole::Iterator);
        d.add_text((20.0, 20.0), "[File_Name] / [Keyword] / [Group_Size] / [Page] of [Pages]");
        d
    }

    fn texts(f: &Figure) -> Vec<String> {
        f.texts()
    }

    #[test]
    fn iterate_by_sample_makes_a_page_per_sample_and_by_keyword_a_page_per_group() {
        let s = session();
        let mut d = doc_with_tile(PlotKind::Histogram);
        let u = units(&s, &d).unwrap();
        assert_eq!(u.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d"]);
        d.iterate = Iterate::Keyword("GRP".into()); // the $ is optional, case does not matter
        let u = units(&s, &d).unwrap();
        assert_eq!(u.iter().map(|x| (x.label.as_str(), x.samples.clone())).collect::<Vec<_>>(), vec![("x", vec![0, 1]), ("y", vec![2, 3])]);
        d.iterate = Iterate::Keyword("MISSING".into());
        assert_eq!(units(&s, &d).unwrap().len(), 1, "samples without the keyword form one '(no value)' group");
        // a subset of samples
        d.iterate = Iterate::Sample;
        d.samples = vec!["c.fcs".into(), "nope.fcs".into()];
        assert_eq!(units(&s, &d).unwrap().len(), 1);
        d.samples = vec!["nope.fcs".into()];
        assert!(units(&s, &d).is_err());
    }

    #[test]
    fn tokens_fill_in_per_page_and_iterator_layers_swap_while_controls_stay() {
        let mut s = session();
        let mut d = doc_with_tile(PlotKind::Histogram);
        d.tile_mut(1).unwrap().add_layer("d.fcs", LayerRole::Control);
        let (pages, notes) = render_batch(&mut s, &d).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(pages.len(), 4);
        assert!(texts(&pages[0]).contains(&"a /  / 1 / 1 of 4".to_string()), "{:?}", texts(&pages[0]));
        assert!(texts(&pages[2]).contains(&"c /  / 1 / 3 of 4".to_string()));
        // page 3 shows c (iterator) and d (control); page 1 shows a and d. The legend names both, d in italics
        let legend = |f: &Figure| -> Vec<(String, bool)> { f.items.iter().filter_map(|i| if let Item::Text(t) = i { ((t.size - 6.6).abs() < 0.01 && ["a", "b", "c", "d"].contains(&t.text.as_str())).then(|| (t.text.clone(), t.italic)) } else { None }).collect() };
        assert_eq!(legend(&pages[0]), vec![("a".into(), false), ("d".into(), true)]);
        assert_eq!(legend(&pages[2]), vec![("c".into(), false), ("d".into(), true)]);
        // when the iterator sample IS the control, it appears once (as the iterator), not twice
        let curves = pages[3].items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items.iter().filter(|x| matches!(x, Item::Poly { .. })).count()) } else { None }).unwrap();
        assert_eq!(curves, 1, "one curve, not the same sample twice (and so no legend)");
        assert_eq!(validate_pdf(&to_pdf(&pages, "b")), Ok(4));
    }

    #[test]
    fn a_keyword_page_overlays_every_sample_of_its_group_with_its_own_colour() {
        let mut s = session();
        let mut d = doc_with_tile(PlotKind::Histogram);
        d.iterate = Iterate::Keyword("GRP".into());
        let (pages, _) = render_batch(&mut s, &d).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(texts(&pages[0]).contains(&"a / x / 2 / 1 of 2".to_string()), "{:?}", texts(&pages[0]));
        let clip = |f: &Figure| f.items.iter().find_map(|i| if let Item::Clip { items, .. } = i { Some(items.clone()) } else { None }).unwrap();
        let colours = |f: &Figure| -> Vec<[u8; 4]> { clip(f).iter().filter_map(|i| if let Item::Poly { stroke: Some(st), .. } = i { Some(st.color) } else { None }).collect() };
        let c0 = colours(&pages[0]);
        assert_eq!(c0.len(), 2, "two samples, two curves");
        assert_ne!(c0[0], c0[1]);
        assert_eq!(c0[0], palette(0));
        assert_eq!(c0[1], palette(1), "consecutive palette colours follow the layer's");
    }

    #[test]
    fn locked_tiles_share_one_view_over_the_whole_batch_and_fixed_tiles_use_theirs() {
        let mut s = session();
        let d = doc_with_tile(PlotKind::Pseudocolor);
        let us = units(&s, &d).unwrap();
        let locked = locked_ranges(&mut s, &d, &us);
        let r = locked[&1];
        // CD3 spans 50..149 (a) up to 350..449 (d): the lock covers all four samples
        assert!(r.x0 < 50.0 && r.x1 > 449.0 && r.x1 < 460.0, "{r:?}");
        let (pages, _) = render_batch(&mut s, &d).unwrap();
        // every page draws the same axis labels (same view), not each sample's own
        let ticks = |f: &Figure| f.texts().into_iter().filter(|t| t.parse::<f64>().is_ok()).collect::<Vec<_>>();
        assert_eq!(ticks(&pages[0]), ticks(&pages[3]));
        // a fixed view is used exactly
        let mut d2 = d.clone();
        d2.tile_mut(1).unwrap().view = ViewMode::Fixed(Range { x0: 0.0, x1: 1000.0, y0: 0.0, y1: 1000.0 });
        assert!(locked_ranges(&mut s, &d2, &us).is_empty());
        let (p2, _) = render_batch(&mut s, &d2).unwrap();
        assert!(ticks(&p2[0]).contains(&"800".to_string()) || ticks(&p2[0]).iter().any(|t| t.starts_with("8")), "{:?}", ticks(&p2[0]));
    }

    #[test]
    fn problems_are_reported_per_page_not_hidden() {
        let mut s = session();
        let mut d = doc_with_tile(PlotKind::Histogram);
        d.tile_mut(1).unwrap().layers.push(Layer { sample: "ghost.fcs".into(), role: LayerRole::Control, color: 3 });
        d.tile_mut(1).unwrap().population = vec!["Nope".into()];
        let (pages, notes) = render_batch(&mut s, &d).unwrap();
        assert_eq!(pages.len(), 4);
        assert!(notes.iter().any(|n| n.contains("sample 'ghost.fcs' is not loaded")), "{notes:?}");
        assert!(notes.iter().any(|n| n.starts_with("page 1:") && n.contains("has no population Nope")), "{notes:?}");
        assert!(pages[0].texts().contains(&"nothing to draw".to_string()));
        // an empty tile invites a drop; an empty document is refused
        let mut e = LayoutDoc::new("e");
        e.add_tile((200.0, 200.0), 200.0, 200.0);
        let (f, _) = render_page(&mut s, &e, None, &HashMap::new(), 1, 1);
        assert!(f.texts().contains(&"drop a sample here".to_string()));
        assert!(render_batch(&mut s, &LayoutDoc::new("none")).unwrap_err().contains("empty"));
    }

    #[test]
    fn gates_of_the_first_layer_are_drawn_with_their_share() {
        let mut s = session();
        let w = s.open_population(0, GateTree::ROOT).unwrap();
        let win = s.window_mut(w).unwrap();
        win.x_chan = "CD3-A".into();
        win.y_chan = "FSC-A".into();
        win.x_scale = ScaleSpec::Identity;
        win.y_scale = ScaleSpec::Identity;
        s.commit_shape(w, &Shape::Rect { x0: 40.0, x1: 99.5, y0: -1.0, y1: 1e6 }, "Low").unwrap();
        let mut d = doc_with_tile(PlotKind::Pseudocolor);
        let (pages, _) = render_batch(&mut s, &d).unwrap();
        assert!(pages[0].texts().contains(&"Low".to_string()), "{:?}", pages[0].texts());
        assert!(!pages[1].texts().contains(&"Low".to_string()), "sample b has no such gate");
        d.tile_mut(1).unwrap().show_gates = false;
        let (pages, _) = render_batch(&mut s, &d).unwrap();
        assert!(!pages[0].texts().contains(&"Low".to_string()));
    }
}
