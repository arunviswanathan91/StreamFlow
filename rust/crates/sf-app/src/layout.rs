//! Batch layouts: one plot per sample, the same population on the same axes, paginated into a grid of plots per
//! page. The axes are LOCKED across the batch (one shared view), so plot 1 and plot 50 can be compared.
//!
//! Text boxes may contain tokens, replaced for each plot or page:
//! `[File_Name]` (sample without `.fcs`), `[Population]`, `[Events]`, `[Percent_Parent]`, `[Page]`, `[Pages]`,
//! `[Count]` (plots in the batch). An unknown token is left as typed so a typo is visible.

use crate::density::{PlotKind, Range};
use crate::figure::{Align, Figure, Item, Text, VAlign};
use crate::plot_figure::{plot_figure, FmoLine, PlotGate, PlotSpec};
use crate::session::Session;
use crate::stats_table::pop_label;
use sf_core::scale::ScaleSpec;
use sf_gating::{Gate, GateKind, GateTree, NodeKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSetup {
    pub width: f32,
    pub height: f32,
    pub margin: f32,
}

impl PageSetup {
    pub fn letter(landscape: bool) -> PageSetup {
        PageSetup::sized(612.0, 792.0, landscape)
    }
    pub fn a4(landscape: bool) -> PageSetup {
        PageSetup::sized(595.28, 841.89, landscape)
    }
    fn sized(w: f32, h: f32, landscape: bool) -> PageSetup {
        if landscape { PageSetup { width: h, height: w, margin: 36.0 } } else { PageSetup { width: w, height: h, margin: 36.0 } }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutSpec {
    pub page: PageSetup,
    pub cols: usize,
    pub rows: usize,
    /// Printed at the top of every page.
    pub title: String,
    /// Under each plot's title line; lines separated by `\n`.
    pub caption: String,
    /// Bottom of every page.
    pub footer: String,
    /// Fill each column top to bottom before moving right (default is across rows).
    pub order_down: bool,
}

impl Default for LayoutSpec {
    fn default() -> Self {
        LayoutSpec { page: PageSetup::letter(true), cols: 3, rows: 2, title: "[Population]".into(), caption: "[File_Name]\n[Events] events · [Percent_Parent]% of parent".into(), footer: "Page [Page] of [Pages]".into(), order_down: false }
    }
}

/// Everything shared by the plots of a batch.
#[derive(Debug, Clone)]
pub struct PlotCommon {
    pub range: Range,
    pub kind: PlotKind,
    pub x_chan: String,
    pub y_chan: String,
    pub x_scale: ScaleSpec,
    pub y_scale: ScaleSpec,
    pub fmo: Vec<FmoLine>,
}

/// One sample's plot.
#[derive(Debug, Clone)]
pub struct CellData {
    pub sample: String,
    pub population: String,
    pub events: usize,
    pub parent_pct: f64,
    /// `None` when the sample lacks the population or a channel (see `note`).
    pub points: Option<Vec<(f64, f64)>>,
    pub gates: Vec<PlotGate>,
    pub note: Option<String>,
}

fn file_name(sample: &str) -> String {
    sample.trim_end_matches(".fcs").trim_end_matches(".FCS").to_string()
}

/// Replace the known tokens in `template`.
pub fn expand_tokens(template: &str, vars: &[(&str, String)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("[{k}]"), v);
    }
    out
}

fn cell_vars(c: &CellData, count: usize) -> Vec<(&'static str, String)> {
    vec![("File_Name", file_name(&c.sample)), ("Population", c.population.clone()), ("Events", c.events.to_string()), ("Percent_Parent", format!("{:.1}", c.parent_pct)), ("Count", count.to_string())]
}

/// Lay the cells out on pages.
pub fn build_pages(spec: &LayoutSpec, common: &PlotCommon, cells: &[CellData]) -> Result<Vec<Figure>, String> {
    if spec.cols == 0 || spec.rows == 0 {
        return Err("a layout needs at least one column and one row".into());
    }
    if cells.is_empty() {
        return Err("no samples to lay out".into());
    }
    let per_page = spec.cols * spec.rows;
    let n_pages = cells.len().div_ceil(per_page);
    let p = spec.page;
    let title_h = if spec.title.trim().is_empty() { 0.0 } else { 22.0 };
    let footer_h = if spec.footer.trim().is_empty() { 0.0 } else { 16.0 };
    let (gx, gy) = (p.margin, p.margin + title_h);
    let (gw, gh) = (p.width - 2.0 * p.margin, p.height - 2.0 * p.margin - title_h - footer_h);
    if gw < 60.0 || gh < 60.0 {
        return Err("the margins leave no room for plots on this page".into());
    }
    let gap = 6.0;
    let (cw, ch) = (gw / spec.cols as f32, gh / spec.rows as f32);
    if cw - gap < 90.0 || ch - gap < 90.0 {
        return Err(format!("{} x {} plots on this page would be smaller than 90 pt; use fewer columns or rows", spec.cols, spec.rows));
    }
    let mut pages = Vec::with_capacity(n_pages);
    for pg in 0..n_pages {
        let mut page = Figure::new(p.width, p.height);
        let page_vars = [("Page", (pg + 1).to_string()), ("Pages", n_pages.to_string()), ("Count", cells.len().to_string()), ("Population", cells[0].population.clone())];
        if title_h > 0.0 {
            page.push(Item::Text(Text::new(p.margin, p.margin, expand_tokens(&spec.title, &page_vars), 13.0).align(Align::Left, VAlign::Top).bold()));
        }
        if footer_h > 0.0 {
            page.push(Item::Text(Text::new(p.width / 2.0, p.height - p.margin / 2.0, expand_tokens(&spec.footer, &page_vars), 8.0).align(Align::Center, VAlign::Middle).color([105, 105, 105, 255])));
        }
        for (slot, cell) in cells[pg * per_page..].iter().take(per_page).enumerate() {
            let (col, row) = if spec.order_down { (slot / spec.rows, slot % spec.rows) } else { (slot % spec.cols, slot / spec.cols) };
            let (x, y) = (gx + col as f32 * cw, gy + row as f32 * ch);
            let captions: Vec<String> = expand_tokens(&spec.caption, &cell_vars(cell, cells.len())).lines().map(|l| l.to_string()).collect();
            let empty: Vec<(f64, f64)> = Vec::new();
            let fig = match &cell.points {
                Some(pts) => plot_figure(&PlotSpec { width: cw - gap, height: ch - gap, captions, points: pts, range: common.range, kind: common.kind, x_chan: common.x_chan.clone(), y_chan: common.y_chan.clone(), x_scale: common.x_scale.clone(), y_scale: common.y_scale.clone(), gates: cell.gates.clone(), fmo: common.fmo.clone(), footer: None }),
                None => {
                    let mut f = plot_figure(&PlotSpec { width: cw - gap, height: ch - gap, captions, points: &empty, range: common.range, kind: common.kind, x_chan: common.x_chan.clone(), y_chan: common.y_chan.clone(), x_scale: common.x_scale.clone(), y_scale: common.y_scale.clone(), gates: vec![], fmo: vec![], footer: None });
                    f.push(Item::Text(Text::new((cw - gap) / 2.0 + 20.0, (ch - gap) / 2.0, cell.note.clone().unwrap_or_else(|| "no data".into()), 8.0).align(Align::Center, VAlign::Middle).color([180, 60, 60, 255])));
                    f
                }
            };
            page.place(fig, x, y);
        }
        pages.push(page);
    }
    Ok(pages)
}

// ---- gathering the data ----------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum View {
    /// One view that fits every sample's population (the axes are locked across the batch).
    FitAll,
    Fixed(Range),
}

#[derive(Debug, Clone)]
pub struct BatchRequest {
    pub population: Vec<String>,
    pub x_chan: String,
    pub y_chan: String,
    pub x_scale: ScaleSpec,
    pub y_scale: ScaleSpec,
    pub kind: PlotKind,
    pub samples: Vec<usize>,
    pub view: View,
    pub show_gates: bool,
    pub show_fmo: bool,
}

/// Is a gate drawn on exactly these axes and scales?
fn on_axes(g: &Gate, req: &BatchRequest) -> bool {
    if matches!(g.kind, GateKind::Indexed(_)) || g.x_chan != req.x_chan || g.x_scale != req.x_scale {
        return false;
    }
    match &g.y_chan {
        Some(y) => req.kind != PlotKind::Histogram && *y == req.y_chan && g.y_scale == req.y_scale,
        None => true,
    }
}

/// Collect every requested sample's points, gates and percentages on shared axes.
pub fn collect(session: &mut Session, req: &BatchRequest) -> Result<(PlotCommon, Vec<CellData>), String> {
    if req.samples.is_empty() {
        return Err("tick at least one sample".into());
    }
    let refs: Vec<&str> = req.population.iter().map(String::as_str).collect();
    let pop = pop_label(&req.population);
    let mut cells = Vec::new();
    let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for &si in &req.samples {
        let name = session.sample(si).name.clone();
        let mut cell = CellData { sample: name, population: pop.clone(), events: 0, parent_pct: 0.0, points: None, gates: vec![], note: None };
        let node = if refs.is_empty() { Some(GateTree::ROOT) } else { session.tree(si).find(&refs) };
        let Some(node) = node else {
            cell.note = Some("population not in this sample".into());
            cells.push(cell);
            continue;
        };
        cell.events = session.count(si, node);
        let parent = session.tree(si).parent(node).ok().flatten().map(|p| session.count(si, p)).unwrap_or(session.sample(si).n_events).max(1);
        cell.parent_pct = 100.0 * cell.events as f64 / parent as f64;
        match session.population_points(si, node, &req.x_chan, &req.y_chan, &req.x_scale, &req.y_scale) {
            Ok(pts) => {
                for (a, b) in pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()) {
                    x0 = x0.min(*a);
                    x1 = x1.max(*a);
                    y0 = y0.min(*b);
                    y1 = y1.max(*b);
                }
                if req.show_gates {
                    let kids = session.tree(si).children(node).map(|c| c.to_vec()).unwrap_or_default();
                    for k in kids {
                        let (Ok(name), Ok(NodeKind::Gate(g))) = (session.tree(si).name(k).map(str::to_string), session.tree(si).kind(k).cloned()) else { continue };
                        if on_axes(&g, req) {
                            let pct = 100.0 * session.count(si, k) as f64 / cell.events.max(1) as f64;
                            cell.gates.push(PlotGate { gate: g, label: Some(format!("{name}\n{pct:.1}%")) });
                        }
                    }
                }
                cell.points = Some(pts);
            }
            Err(e) => cell.note = Some(e),
        }
        cells.push(cell);
    }
    let range = match &req.view {
        View::Fixed(r) => *r,
        View::FitAll => {
            if !(x1 > x0) || !(y1 > y0) {
                return Err("no sample has events on these axes".into());
            }
            let (px, py) = (0.01 * (x1 - x0), 0.01 * (y1 - y0));
            Range { x0: x0 - px, x1: x1 + px, y0: y0 - py, y1: y1 + py }
        }
    };
    let mut fmo = Vec::new();
    if req.show_fmo {
        if let (Some(f), Ok(t)) = (session.fmo(&req.x_chan), req.x_scale.build()) {
            fmo.push(FmoLine { on_x: true, pos: t.forward(f.level) });
        }
        if req.kind != PlotKind::Histogram {
            if let (Some(f), Ok(t)) = (session.fmo(&req.y_chan), req.y_scale.build()) {
                fmo.push(FmoLine { on_x: false, pos: t.forward(f.level) });
            }
        }
    }
    Ok((PlotCommon { range, kind: req.kind, x_chan: req.x_chan.clone(), y_chan: req.y_chan.clone(), x_scale: req.x_scale.clone(), y_scale: req.y_scale.clone(), fmo }, cells))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drawing::Shape;
    use crate::figure::{to_pdf, validate_pdf};
    use crate::sample::LoadedSample;

    fn common() -> PlotCommon {
        PlotCommon { range: Range { x0: 0.0, x1: 100.0, y0: 0.0, y1: 100.0 }, kind: PlotKind::Pseudocolor, x_chan: "X".into(), y_chan: "Y".into(), x_scale: ScaleSpec::Identity, y_scale: ScaleSpec::Identity, fmo: vec![] }
    }

    fn cell(i: usize) -> CellData {
        CellData { sample: format!("sample{i}.fcs"), population: "Pop".into(), events: 1000 + i, parent_pct: 12.34, points: Some(vec![(10.0, 10.0), (50.0, 50.0)]), gates: vec![], note: None }
    }

    #[test]
    fn tokens_are_replaced_and_unknown_ones_stay_visible() {
        let c = cell(7);
        let t = expand_tokens("[File_Name] has [Events] events, [Percent_Parent]% of [Population] (of [Count]) [Typo]", &cell_vars(&c, 40));
        assert_eq!(t, "sample7 has 1007 events, 12.3% of Pop (of 40) [Typo]");
    }

    #[test]
    fn pages_hold_rows_times_cols_plots_and_the_last_page_is_partial() {
        let spec = LayoutSpec { cols: 3, rows: 2, ..Default::default() };
        let cells: Vec<CellData> = (0..14).map(cell).collect();
        let pages = build_pages(&spec, &common(), &cells).unwrap();
        assert_eq!(pages.len(), 3, "14 plots at 6 per page");
        let count_samples = |p: &Figure| p.texts().iter().filter(|t| t.starts_with("sample")).count();
        assert_eq!((count_samples(&pages[0]), count_samples(&pages[1]), count_samples(&pages[2])), (6, 6, 2));
        // every sample appears exactly once across the batch, in order
        let names: Vec<String> = pages.iter().flat_map(|p| p.texts()).filter(|t| t.starts_with("sample")).collect();
        assert_eq!(names, (0..14).map(|i| format!("sample{i}")).collect::<Vec<_>>());
        assert!(pages[2].texts().contains(&"Page 3 of 3".to_string()) && pages[0].texts().contains(&"Pop".to_string()));
        assert!(pages[0].texts().contains(&"1000 events · 12.3% of parent".to_string()));
        assert_eq!(validate_pdf(&to_pdf(&pages, "t")), Ok(3));
    }

    #[test]
    fn order_across_and_down_place_the_second_plot_differently() {
        let cells: Vec<CellData> = (0..4).map(cell).collect();
        let pos = |down: bool| -> Vec<(f32, f32)> {
            let spec = LayoutSpec { cols: 2, rows: 2, order_down: down, ..Default::default() };
            let page = build_pages(&spec, &common(), &cells).unwrap().remove(0);
            ["sample0", "sample1", "sample2"].iter().map(|n| page.items.iter().find_map(|i| if let Item::Text(t) = i { (t.text == *n).then_some((t.x, t.y)) } else { None }).unwrap()).collect()
        };
        let across = pos(false);
        let down = pos(true);
        assert!(across[1].0 > across[0].0 && (across[1].1 - across[0].1).abs() < 0.01, "across: the second plot is to the right");
        assert!(down[1].1 > down[0].1 && (down[1].0 - down[0].0).abs() < 0.01, "down: the second plot is below");
        assert_eq!(across[2].1 > across[0].1, down[2].0 > down[0].0, "the third plot starts the next row / column");
    }

    #[test]
    fn bad_layouts_are_refused_with_a_reason() {
        let cells: Vec<CellData> = (0..2).map(cell).collect();
        assert!(build_pages(&LayoutSpec { cols: 0, ..Default::default() }, &common(), &cells).is_err());
        assert!(build_pages(&LayoutSpec::default(), &common(), &[]).unwrap_err().contains("no samples"));
        let e = build_pages(&LayoutSpec { cols: 12, rows: 8, ..Default::default() }, &common(), &cells).unwrap_err();
        assert!(e.contains("smaller than 90 pt"), "{e}");
        let tiny = PageSetup { width: 100.0, height: 100.0, margin: 36.0 };
        assert!(build_pages(&LayoutSpec { page: tiny, ..Default::default() }, &common(), &cells).is_err());
        // a plot without data is marked, not left blank
        let mut missing = cell(0);
        missing.points = None;
        missing.note = Some("population not in this sample".into());
        let page = build_pages(&LayoutSpec::default(), &common(), &[missing]).unwrap().remove(0);
        assert!(page.texts().contains(&"population not in this sample".to_string()));
    }

    #[test]
    fn page_sizes_and_orientation() {
        let l = PageSetup::letter(true);
        assert_eq!((l.width, l.height), (792.0, 612.0));
        let a = PageSetup::a4(false);
        assert!((a.width - 595.28).abs() < 0.01 && (a.height - 841.89).abs() < 0.01);
    }

    fn session_with_gates() -> Session {
        let mut s = Session::new();
        for (k, name) in ["a.fcs", "b.fcs", "c.fcs"].iter().enumerate() {
            let ev: Vec<f32> = (0..1000).flat_map(|i| [i as f32, ((i * 7) % 500) as f32 + k as f32 * 100.0]).collect();
            s.add_sample(LoadedSample::synthetic(name, &["X-A", "Y-A"], vec![2000.0, 2000.0], ev));
        }
        for si in 0..2 {
            let w = s.open_population(si, GateTree::ROOT).unwrap();
            let win = s.window_mut(w).unwrap();
            win.x_chan = "X-A".into();
            win.y_chan = "Y-A".into();
            win.x_scale = ScaleSpec::Identity;
            win.y_scale = ScaleSpec::Identity;
            s.commit_shape(w, &Shape::Rect { x0: -1.0, x1: 499.5, y0: -1.0, y1: 1e6 }, "Left").unwrap();
        }
        s
    }

    fn request(samples: Vec<usize>) -> BatchRequest {
        BatchRequest { population: vec![], x_chan: "X-A".into(), y_chan: "Y-A".into(), x_scale: ScaleSpec::Identity, y_scale: ScaleSpec::Identity, kind: PlotKind::Pseudocolor, samples, view: View::FitAll, show_gates: true, show_fmo: true }
    }

    #[test]
    fn collect_locks_one_view_over_all_samples_and_labels_gates_with_their_share() {
        let mut s = session_with_gates();
        let (common, cells) = collect(&mut s, &request(vec![0, 1, 2])).unwrap();
        // x spans 0..999, y spans 0..~700 (sample 2 is shifted by 200): the shared range covers all three
        assert!(common.range.x0 < 0.0 && common.range.x1 > 999.0);
        assert!(common.range.y1 > 699.0 && common.range.y1 < 710.0, "sample c reaches 699 (499 + 200 shift), plus 1% padding: {:?}", common.range);
        assert_eq!(cells.iter().map(|c| c.events).collect::<Vec<_>>(), vec![1000, 1000, 1000]);
        assert_eq!(cells[0].gates.len(), 1);
        assert_eq!(cells[0].gates[0].label.as_deref(), Some("Left\n50.0%"));
        assert!(cells[2].gates.is_empty(), "sample c has no gate");
        assert!((cells[0].parent_pct - 100.0).abs() < 1e-9);
        // a nested population that only some samples have is flagged on the others
        let mut r = request(vec![0, 1, 2]);
        r.population = vec!["Left".into()];
        let (_, cells) = collect(&mut s, &r).unwrap();
        assert_eq!(cells[0].events, 500);
        assert!((cells[0].parent_pct - 50.0).abs() < 1e-9);
        assert!(cells[2].points.is_none() && cells[2].note.as_deref() == Some("population not in this sample"));
        // a fixed view is used as given, FMO lines come through the plot's own scale
        s.window_mut(1).unwrap().x_chan = "X-A".into();
        s.set_fmo_from_window(1, 90.0).unwrap();
        let mut r = request(vec![0]);
        r.view = View::Fixed(Range { x0: 0.0, x1: 10.0, y0: 0.0, y1: 10.0 });
        let (common, _) = collect(&mut s, &r).unwrap();
        assert_eq!(common.range.x1, 10.0);
        assert!(common.fmo.iter().any(|f| f.on_x) && common.fmo.iter().any(|f| !f.on_x));
        let mut off = request(vec![0]);
        off.show_fmo = false;
        assert!(collect(&mut s, &off).unwrap().0.fmo.is_empty());
        assert!(collect(&mut s, &request(vec![])).is_err());
        // end to end: a PDF that the structural validator accepts
        let (common, cells) = collect(&mut s, &request(vec![0, 1, 2])).unwrap();
        let pages = build_pages(&LayoutSpec { cols: 2, rows: 2, ..Default::default() }, &common, &cells).unwrap();
        assert_eq!(validate_pdf(&to_pdf(&pages, "batch")), Ok(1));
    }
}
