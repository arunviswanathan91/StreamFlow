//! Headless UI tests: real egui frames and real pointer / keyboard events over the real layout.

use super::*;
use crate::drawing::{Shape, Tool};
use crate::sample::LoadedSample;
use eframe::egui::{Pos2, Rect};
use sf_core::rng::Rng;
use sf_core::scale::ScaleSpec;
use sf_gating::{Gate, GateKind, GateTree, NodeKind};
use workstation::Tab;
use crate::embed;

fn app_with_sample() -> App {
    let mut app = App::new();
    let mut rng = Rng::new(3);
    let n = 3000;
    let events = (0..n).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 100_000.0) as f32, (rng.next_f64() * 20_000.0 - 500.0) as f32, (rng.next_f64() * 20_000.0 - 500.0) as f32]).collect();
    app.session.add_sample(LoadedSample::synthetic("t.fcs", &["FSC-A", "SSC-A", "CD3-A", "CD4-A"], vec![262_144.0; 4], events));
    app
}

thread_local! {
    static CLOCK: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
}

fn screen() -> Rect {
    Rect::from_min_size(Pos2::ZERO, egui::vec2(1500.0, 900.0))
}

fn frame(app: &mut App, ctx: &egui::Context) -> egui::FullOutput {
    let input = egui::RawInput { screen_rect: Some(screen()), ..Default::default() };
    ctx.run(input, |c| app.show(c))
}

/// One frame with events; the clock advances so egui can recognise double clicks.
fn pointer(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let t = CLOCK.with(|c| {
        c.set(c.get() + 0.04);
        c.get()
    });
    let input = egui::RawInput { screen_rect: Some(screen()), time: Some(t), events, ..Default::default() };
    let _ = ctx.run(input, |c| app.show(c));
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn key(key: egui::Key, ctrl: bool, shift: bool) -> egui::Event {
    let m = egui::Modifiers { ctrl, command: ctrl, shift, ..Default::default() };
    egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: m }
}

fn settle(app: &mut App, ctx: &egui::Context) {
    for _ in 0..3 {
        frame(app, ctx);
    }
}

/// Open the root population and make the tab ready, returning (window id, plot rect).
fn ready_tab(app: &mut App, ctx: &egui::Context) -> (u64, Rect) {
    let w = app.open_tab(0, GateTree::ROOT).unwrap();
    settle(app, ctx);
    let plot = app.ws.graphs[&w].plot_rect.expect("plot drawn");
    (w, plot)
}

fn children(app: &App) -> Vec<usize> {
    app.session.tree(0).children(GateTree::ROOT).unwrap().to_vec()
}

#[test]
fn mapper_round_trips_and_flips_y() {
    let m = Mapper { rect: Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(110.0, 220.0)), range: crate::density::Range { x0: 0.0, x1: 10.0, y0: -5.0, y1: 5.0 } };
    assert_eq!(m.to_screen((0.0, -5.0)), Pos2::new(10.0, 220.0), "data bottom-left is the screen bottom-left");
    assert_eq!(m.to_screen((10.0, 5.0)), Pos2::new(110.0, 20.0));
    for p in [(3.3, 1.2), (0.0, 0.0), (9.9, -4.9)] {
        let back = m.to_data(m.to_screen(p));
        assert!((back.0 - p.0).abs() < 1e-4 && (back.1 - p.1).abs() < 1e-4, "{p:?} -> {back:?}");
    }
}

#[test]
fn every_module_renders_with_and_without_data() {
    let ctx = egui::Context::default();
    let mut empty = App::new();
    let mut loaded = app_with_sample();
    loaded.open_tab(0, GateTree::ROOT).unwrap();
    for (_, items) in Module::GROUPS {
        for &m in items.iter() {
            empty.set_module(m);
            loaded.set_module(m);
            for _ in 0..2 {
                frame(&mut empty, &ctx);
                let out = frame(&mut loaded, &ctx);
                assert!(!out.shapes.is_empty(), "{m:?} drew nothing");
            }
        }
    }
    // each module has a unique label and icon-bearing nav entry
    let all: Vec<Module> = Module::GROUPS.iter().flat_map(|(_, i)| i.iter().copied()).collect();
    let labels: std::collections::HashSet<&str> = all.iter().map(|m| m.label()).collect();
    assert_eq!((all.len(), labels.len()), (18, 18));
}

#[test]
fn the_navigation_rail_collapses_and_expands_without_losing_the_module() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    app.set_module(Module::Statistics);
    settle(&mut app, &ctx);
    app.nav_expanded = false;
    for _ in 0..12 {
        frame(&mut app, &ctx);
    }
    assert_eq!(app.module(), Module::Statistics);
    // the Ctrl+B shortcut toggles it back
    pointer(&mut app, &ctx, vec![key(egui::Key::B, true, false)]);
    assert!(app.nav_expanded);
    for _ in 0..12 {
        frame(&mut app, &ctx);
    }
}

#[test]
fn opening_a_population_creates_a_tab_with_a_density_texture_and_gates_are_drawn() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    assert_eq!(app.ws.tab, Tab::Graph(w));
    assert!(app.ws.graphs[&w].tex.is_some(), "density texture built");
    assert!(app.ws.graphs[&w].shown > 2000, "most of the 3000 events are inside the auto range");
    app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 100_000.0, y0: 0.0, y1: 50_000.0 }, "G").unwrap();
    let before = app.ws.graphs[&w].key;
    settle(&mut app, &ctx);
    assert_ne!(app.ws.graphs[&w].key, before, "gating invalidates the cached texture");
    // scaled fluorescence axes, then dot plot and histogram
    {
        let win = app.session.window_mut(w).unwrap();
        win.x_chan = "CD3-A".into();
        win.y_chan = "CD4-A".into();
        win.x_scale = ScaleSpec::Logicle { t: 262_144.0, w: 0.5, m: 4.5, a: 0.0 };
        win.y_scale = ScaleSpec::Arcsinh { cofactor: 150.0 };
    }
    app.ws.graphs.get_mut(&w).unwrap().refit = true;
    settle(&mut app, &ctx);
    assert!(app.ws.graphs[&w].range.unwrap().is_valid());
    for kind in crate::density::PlotKind::ALL {
        app.session.window_mut(w).unwrap().kind = kind;
        app.ws.graphs.get_mut(&w).unwrap().refit = true;
        settle(&mut app, &ctx);
    }
}

#[test]
fn a_deep_gating_chain_renders_in_the_tree_panel_without_recursion() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let mut w = app.session.open_population(0, GateTree::ROOT).unwrap();
    for i in 0..400 {
        let node = app.session.commit_shape(w, &Shape::Rect { x0: -1e9, x1: 1e9, y0: -1e9, y1: 1e9 }, &format!("L{i}")).unwrap();
        app.session.close_window(w);
        w = app.session.open_population(0, node).unwrap();
    }
    settle(&mut app, &ctx);
    assert_eq!(app.session.tree(0).len(), 401);
    let first = app.session.tree(0).children(GateTree::ROOT).unwrap()[0];
    app.ws.collapsed.insert((0, first));
    frame(&mut app, &ctx);
    // the breadcrumb of the deepest population lists every ancestor
    assert_eq!(app.session.breadcrumb(0, app.session.window(w).unwrap().node).len(), 401);
}

#[test]
fn deleting_a_population_with_an_open_tab_does_not_break_the_next_frame() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let w = app.session.open_population(0, GateTree::ROOT).unwrap();
    let g = app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, "G").unwrap();
    app.open_tab(0, g).unwrap();
    settle(&mut app, &ctx);
    app.session.delete(0, g).unwrap();
    settle(&mut app, &ctx);
    assert_eq!(app.session.windows.len(), 1);
    assert_eq!(app.ws.tab, Tab::Graph(w), "focus falls back to a tab that still exists");
}

/// Real egui pointer events over the real layout: drag with the Rectangle tool, then click vertices and a
/// closing double click for a polygon, a click for a quadrant, a drag for an interval and an ellipse.
#[test]
fn every_gate_tool_creates_its_gate_through_the_real_event_path() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot) = ready_tab(&mut app, &ctx);
    let range = app.ws.graphs[&w].range.unwrap();
    let map = Mapper { rect: plot, range };
    let at = |fx: f32, fy: f32| Pos2::new(plot.left() + fx * plot.width(), plot.top() + fy * plot.height());
    let set_tool = |app: &mut App, t: Tool| app.session.window_mut(w).unwrap().tool = t;

    // rectangle
    set_tool(&mut app, Tool::Rectangle);
    settle(&mut app, &ctx);
    let (a, b) = (at(0.25, 0.75), at(0.60, 0.30));
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(a)]);
    pointer(&mut app, &ctx, vec![button(a, true)]);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0))]);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    pointer(&mut app, &ctx, vec![button(b, false)]);
    frame(&mut app, &ctx);
    let kids = children(&app);
    assert_eq!(kids.len(), 1, "one rectangle gate was created; status: {}", app.status);
    let g = match app.session.tree(0).kind(kids[0]).unwrap() {
        NodeKind::Gate(g) => g.clone(),
        k => panic!("{k:?}"),
    };
    let (da, db) = (map.to_data(a), map.to_data(b));
    let tol = (range.x1 - range.x0) * 0.02;
    assert!((g.xs.iter().cloned().fold(f64::INFINITY, f64::min) - da.0.min(db.0)).abs() < tol, "x0 {:?} vs {:?}", g.xs, (da, db));
    assert!((g.ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - da.1.max(db.1)).abs() < (range.y1 - range.y0) * 0.02);
    assert!(app.session.count(0, kids[0]) > 0 && app.session.count(0, kids[0]) < 3000);

    // polygon: three clicks, then a closing double click on the last spot
    set_tool(&mut app, Tool::Polygon);
    frame(&mut app, &ctx);
    for p in [at(0.1, 0.9), at(0.9, 0.9), at(0.5, 0.4)] {
        pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(p)]);
        pointer(&mut app, &ctx, vec![button(p, true)]);
        pointer(&mut app, &ctx, vec![button(p, false)]);
    }
    assert!(app.session.window(w).unwrap().drawing.is_active(), "vertices are being collected");
    let last = at(0.5, 0.2);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(last)]);
    for _ in 0..2 {
        pointer(&mut app, &ctx, vec![button(last, true)]);
        pointer(&mut app, &ctx, vec![button(last, false)]);
    }
    frame(&mut app, &ctx);
    let kids = children(&app);
    assert_eq!(kids.len(), 2, "polygon created; status: {}", app.status);
    assert!(matches!(app.session.tree(0).kind(kids[1]).unwrap(), NodeKind::Gate(Gate { kind: GateKind::Polygon, .. })));

    // interval and ellipse: drags
    for (tool, kind_is) in [(Tool::Interval, GateKind::Interval), (Tool::Ellipse, GateKind::Ellipse)] {
        set_tool(&mut app, tool);
        frame(&mut app, &ctx);
        let (a, b) = (at(0.3, 0.5), at(0.5, 0.7));
        pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(a)]);
        pointer(&mut app, &ctx, vec![button(a, true)]);
        pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
        pointer(&mut app, &ctx, vec![button(b, false)]);
        frame(&mut app, &ctx);
        let last = *children(&app).last().unwrap();
        assert!(matches!(app.session.tree(0).kind(last).unwrap(), NodeKind::Gate(g) if g.kind == kind_is), "{tool:?} did not create a {kind_is:?}; status: {}", app.status);
    }

    // quadrant: a single click makes four populations that partition the parent
    let before = children(&app).len();
    set_tool(&mut app, Tool::Quadrant);
    frame(&mut app, &ctx);
    let c = at(0.5, 0.5);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(c)]);
    pointer(&mut app, &ctx, vec![button(c, true)]);
    pointer(&mut app, &ctx, vec![button(c, false)]);
    frame(&mut app, &ctx);
    let kids = children(&app);
    assert_eq!(kids.len(), before + 4, "status: {}", app.status);
    let quads: Vec<usize> = kids[before..].to_vec();
    assert_eq!(quads.iter().map(|&q| app.session.count(0, q)).sum::<usize>(), 3000);
    assert!(app.session.log.is_empty(), "{:?}", app.session.log);
}

#[test]
fn keyboard_shortcuts_select_tools_undo_redo_and_delete_a_gate() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    pointer(&mut app, &ctx, vec![key(egui::Key::R, false, false)]);
    assert_eq!(app.session.window(w).unwrap().tool, Tool::Rectangle);
    pointer(&mut app, &ctx, vec![key(egui::Key::Q, false, false)]);
    assert_eq!(app.session.window(w).unwrap().tool, Tool::Quadrant);
    pointer(&mut app, &ctx, vec![key(egui::Key::N, false, false)]);
    assert_eq!(app.session.window(w).unwrap().tool, Tool::Pointer);

    let g = app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, "A").unwrap();
    settle(&mut app, &ctx);
    pointer(&mut app, &ctx, vec![key(egui::Key::Z, true, false)]);
    assert!(children(&app).is_empty(), "Ctrl+Z removed the gate");
    pointer(&mut app, &ctx, vec![key(egui::Key::Y, true, false)]);
    assert_eq!(children(&app).len(), 1, "Ctrl+Y brought it back");
    pointer(&mut app, &ctx, vec![key(egui::Key::Z, true, true)]);
    assert_eq!(children(&app).len(), 1, "Ctrl+Shift+Z is redo (nothing left to redo), it must not undo");
    pointer(&mut app, &ctx, vec![key(egui::Key::Z, true, false)]);
    assert!(children(&app).is_empty());
    pointer(&mut app, &ctx, vec![key(egui::Key::Z, true, true)]);
    assert_eq!(children(&app).len(), 1, "Ctrl+Shift+Z redoes what Ctrl+Z undid");
    pointer(&mut app, &ctx, vec![key(egui::Key::Z, true, false)]);
    let _ = g;
    // Delete removes the selected child gate
    let g2 = app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, "B").unwrap();
    app.session.selected = Some((0, g2));
    settle(&mut app, &ctx);
    pointer(&mut app, &ctx, vec![key(egui::Key::Delete, false, false)]);
    assert!(children(&app).is_empty(), "Delete removed the selected gate; status: {}", app.status);
    assert!(app.session.can_undo(0));
}

#[test]
fn arrow_keys_hop_between_samples_keeping_the_population_path() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let mut rng = Rng::new(9);
    let events = (0..2000).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 100_000.0) as f32, 1.0, 2.0]).collect();
    app.session.add_sample(LoadedSample::synthetic("u.fcs", &["FSC-A", "SSC-A", "CD3-A", "CD4-A"], vec![262_144.0; 4], events));
    let (w, _) = ready_tab(&mut app, &ctx);
    pointer(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false, false)]);
    assert_eq!(app.session.window(w).unwrap().sample, 1);
    assert_eq!(app.ws.tree_sample, 1, "the populations panel follows");
    pointer(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false, false)]);
    assert_eq!(app.session.window(w).unwrap().sample, 0);
    pointer(&mut app, &ctx, vec![key(egui::Key::ArrowLeft, false, false)]);
    assert_eq!(app.session.window(w).unwrap().sample, 0, "no sample before the first");
}

#[test]
fn statistics_compensation_export_and_overview_screens_render_on_loaded_data() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, "P").unwrap();
    // Overview with a selected population
    app.ws.tab = Tab::Overview;
    settle(&mut app, &ctx);
    assert!(!app.ws.graphs.is_empty());
    for m in [Module::Statistics, Module::Compensation, Module::Export, Module::AnalysisLog, Module::DimReduction] {
        app.set_module(m);
        settle(&mut app, &ctx);
    }
    // the compensation screen starts from identity over the area channels and can be applied programmatically
    let info = app.session.sample(0).clone();
    assert_eq!(info.channels.len(), 4);
}

/// The real FlowJo workspace, imported through the app: 39 samples' gating trees on biex / linear axes with
/// FlowJo's compensation, checked against the counts FlowJo itself saved.
#[test]
fn importing_the_real_flowjo_workspace_reproduces_flowjos_counts() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/workspace files/ARUN COCULTURE 29102024"));
    let wsp = dir.join("31-Oct-2024.wsp");
    let Ok(text) = std::fs::read_to_string(&wsp) else {
        eprintln!("SKIPPED: fixture workspace not found");
        return;
    };
    let mut session = Session::new();
    let rep = session.import_wsp(&text, &dir).unwrap();
    eprintln!("{}", rep.summary());
    assert!(rep.imported.len() >= 30, "only {} samples imported", rep.imported.len());
    assert!(rep.populations > 150 && rep.compared > 150, "{} populations, {} compared", rep.populations, rep.compared);
    assert!(rep.mean_diff_pct < 1.0, "mean difference to FlowJo's counts {:.3}%", rep.mean_diff_pct);
    // the trees came with the scales and the compensation, and every population opens in a tab at any depth
    let i = session.samples().iter().position(|s| s.comp.is_some()).expect("a compensated sample");
    assert!(session.sample(i).channel_index("Comp-PE-Cy7-A").is_some());
    let deepest = session.tree(i).self_and_descendants(GateTree::ROOT).unwrap().into_iter().max_by_key(|&n| session.tree(i).depth(n).unwrap()).unwrap();
    assert!(session.tree(i).depth(deepest).unwrap() >= 4, "the CD8- Live chain is 4+ levels deep");
    let w = session.open_population(i, deepest).unwrap();
    assert!(session.window_points(w).is_ok());
    assert!(session.windows.len() == 1);
}

#[test]
fn the_app_shows_an_import_summary_and_renders_after_it() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/workspace files/ARUN COCULTURE 29102024"));
    let wsp = dir.join("31-Oct-2024.wsp");
    if !wsp.exists() {
        eprintln!("SKIPPED: fixture workspace not found");
        return;
    }
    let ctx = egui::Context::default();
    let mut app = App::new();
    app.import_flowjo_path(&wsp);
    assert!(app.status.starts_with("Imported "), "{}", app.status);
    assert!(app.status.contains("agree with FlowJo's saved counts"), "{}", app.status);
    settle(&mut app, &ctx);
    // open the first sample's deepest population from the tree and render it
    let si = app.session.samples().iter().position(|s| s.comp.is_some()).unwrap();
    let nodes = app.session.tree(si).self_and_descendants(GateTree::ROOT).unwrap();
    let deepest = *nodes.iter().max_by_key(|&&n| app.session.tree(si).depth(n).unwrap()).unwrap();
    app.open_tab(si, deepest).unwrap();
    settle(&mut app, &ctx);
    assert!(app.ws.graphs.values().any(|g| g.tex.is_some()));
}

#[test]
fn the_statistics_module_computes_a_table_for_the_selected_samples() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, "P").unwrap();
    app.set_module(Module::Statistics);
    settle(&mut app, &ctx);
    app.stats.channel = "CD3-A".into();
    let rows = app.compute_statistics();
    assert_eq!(rows, 2, "All Events and P");
    let t = app.stats.table.as_ref().unwrap();
    assert!(t.metrics.contains(&crate::stats_table::Metric::Median));
    let csv = crate::stats_table::to_csv(t);
    assert!(csv.lines().count() == 3 && csv.starts_with("Sample,Population,Count"), "{csv}");
    settle(&mut app, &ctx); // the table renders
    // nothing selected -> a clear message, not an empty table
    app.stats.selected.iter_mut().for_each(|s| *s = false);
    assert_eq!(app.compute_statistics(), 0);
    assert!(app.status.contains("at least one sample"));
}

// ---- discovery, spectral and compensation screens ----------------------------------------------------------

/// Two samples of four phenotypes over four markers (raw units ~ 180 x the display value).
fn marker_app() -> App {
    let centres = [[5.0, 5.0, 0.0, 0.0], [5.0, 0.0, 5.0, 0.0], [5.0, 5.0, 0.0, 5.0], [0.0, 0.0, 0.0, 0.0]];
    let mut app = App::new();
    for (k, name) in ["a.fcs", "b.fcs"].iter().enumerate() {
        let mut rng = Rng::new(40 + k as u64);
        let mut ev = Vec::new();
        for i in 0..1500 {
            let c = i % 4;
            for m in 0..4 {
                let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
                let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
                ev.push(((centres[c][m] + 0.5 * z) * 150.0 * 1.2 + 40.0) as f32);
            }
        }
        app.session.add_sample(LoadedSample::synthetic(name, &["CD3-A", "CD4-A", "CD8-A", "FoxP3-A"], vec![1e5; 4], ev));
    }
    app
}

fn wait_for(app: &mut App, ctx: &egui::Context, what: &str, done: impl Fn(&App) -> bool) {
    for _ in 0..600 {
        frame(app, ctx);
        if done(app) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("timed out waiting for {what}; status: {}", app.status);
}

#[test]
fn dim_reduction_runs_in_the_background_colours_clusters_and_approves_populations() {
    let ctx = egui::Context::default();
    let mut app = marker_app();
    app.set_module(Module::DimReduction);
    settle(&mut app, &ctx);
    assert_eq!(app.disc.features.iter().filter(|(_, on)| *on).count(), 4, "all four markers are chosen by default");
    app.disc.per_sample = 600;
    app.disc_run_embedding();
    assert!(app.disc.is_busy(), "the embedding runs on a worker thread");
    frame(&mut app, &ctx); // the interface keeps rendering while it runs
    wait_for(&mut app, &ctx, "the embedding", |a| a.disc.emb.is_some());
    assert_eq!(app.disc.emb.as_ref().unwrap().input.n(), 1200);
    for color in [discover_view::ColorBy::Sample, discover_view::ColorBy::Marker(0), discover_view::ColorBy::Density] {
        app.disc.color = color;
        settle(&mut app, &ctx);
    }
    // clusters
    app.disc.k = 4;
    app.disc_run_clustering();
    wait_for(&mut app, &ctx, "the clustering", |a| a.disc.clus.is_some());
    let names = app.disc.names.clone();
    assert_eq!(names.len(), 4);
    app.disc.color = discover_view::ColorBy::Cluster;
    app.disc.picked = vec![true; 4];
    settle(&mut app, &ctx);
    // approve into whole-file populations
    app.disc_approve_clusters(embed::Scope::FullFile);
    assert!(app.status.contains("whole files"), "{}", app.status);
    let kids = app.session.tree(0).children(GateTree::ROOT).unwrap().len();
    assert_eq!(kids, 4);
    let covered: usize = app.session.tree(0).children(GateTree::ROOT).unwrap().to_vec().into_iter().map(|n| app.session.count(0, n)).sum();
    assert_eq!(covered, 1500, "every event of the file is assigned to a cluster population");
    assert!(app.session.undo(0));
    assert!(app.session.tree(0).children(GateTree::ROOT).unwrap().is_empty());
    // nothing picked -> a clear message
    app.disc.picked = vec![false; 4];
    app.disc_approve_clusters(embed::Scope::Embedded);
    assert!(app.status.contains("Tick the clusters"));
}

#[test]
fn rules_name_and_approve_and_a_drawn_region_becomes_a_population() {
    let ctx = egui::Context::default();
    let mut app = marker_app();
    app.set_module(Module::DimReduction);
    settle(&mut app, &ctx);
    app.disc.per_sample = 500;
    app.disc_run_embedding();
    wait_for(&mut app, &ctx, "the embedding", |a| a.disc.emb.is_some());
    app.disc.rules.push(discover_view::RuleUi { name: "Treg".into(), clauses: vec![("CD3-A".into(), true), ("CD4-A".into(), true), ("FoxP3-A".into(), true)] });
    app.disc.rules.push(discover_view::RuleUi { name: "CD8 T".into(), clauses: vec![("CD3-A".into(), true), ("CD8-A".into(), true)] });
    app.disc.color = discover_view::ColorBy::Rule(0);
    settle(&mut app, &ctx);
    app.disc_approve_rules(embed::Scope::FullFile);
    let treg = app.session.count(0, app.session.tree(0).find(&["Treg"]).unwrap());
    assert!((treg as f64 - 375.0).abs() < 25.0, "a quarter of 1500 events are Treg: {treg}");
    // drag a rectangle over the whole embedding with the real event path -> everything pooled becomes a region
    app.disc.tool = Tool::Rectangle;
    settle(&mut app, &ctx);
    let plot = app.disc.plot_rect.expect("embedding drawn");
    let (a, b) = (Pos2::new(plot.left() + 4.0, plot.top() + 4.0), Pos2::new(plot.right() - 4.0, plot.bottom() - 4.0));
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(a)]);
    pointer(&mut app, &ctx, vec![button(a, true)]);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(b)]);
    pointer(&mut app, &ctx, vec![button(b, false)]);
    frame(&mut app, &ctx);
    assert!(app.status.contains("Region 1"), "{}", app.status);
    let region = app.session.count(0, app.session.tree(0).find(&["Region 1"]).unwrap());
    assert_eq!(region, 500, "all pooled events of sample 0 fall inside a full-plot rectangle");
}

#[test]
fn clustering_screen_shows_the_heat_map_and_pools_on_its_own() {
    let ctx = egui::Context::default();
    let mut app = marker_app();
    app.set_module(Module::Clustering);
    settle(&mut app, &ctx);
    app.disc.per_sample = 500;
    app.disc.cluster_method = embed::ClusterMethod::PhenoGraph;
    app.disc_run_clustering(); // pools first, then clusters
    wait_for(&mut app, &ctx, "the clustering", |a| a.disc.clus.is_some());
    assert!(app.disc.input.is_some() && app.disc.emb.is_none(), "no embedding was needed");
    settle(&mut app, &ctx);
    let res = app.disc.clus.as_ref().unwrap();
    assert_eq!((res.requested, res.actual), (embed::ClusterMethod::PhenoGraph, embed::ClusterMethod::PhenoGraph));
    assert!(res.n_clusters >= 4);
    // PhenoGraph cannot place new events, and the screen says why instead of approving something wrong
    app.disc.picked = vec![true; res.n_clusters];
    app.disc_approve_clusters(embed::Scope::FullFile);
    assert!(app.status.contains("FlowSOM"), "{}", app.status);
}

#[test]
fn compensation_is_computed_from_the_real_controls_and_applied_through_the_screen() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
    if !dir.join("fitc alone.fcs").exists() {
        eprintln!("SKIPPED: fixtures missing");
        return;
    }
    let ctx = egui::Context::default();
    let mut app = App::new();
    let files: Vec<std::path::PathBuf> = ["fitc alone", "pe alone", "apc alone", "AF700 alone", "bv421 alone", "bv711 alone", "multistain-bv421-pe-fitc-apc-bv711-AF700"].iter().map(|n| dir.join(format!("{n}.fcs"))).collect();
    app.load_files(&files);
    assert_eq!(app.session.samples().len(), 7);
    app.set_module(Module::Compensation);
    settle(&mut app, &ctx);
    assert!(app.compensation_from_controls(), "{}", app.status);
    settle(&mut app, &ctx);
    // apply to every sample and look at the compensated channels through the session
    let m = app.comp.matrix_for_test().expect("matrix computed");
    assert_eq!(m.detectors.len(), 6);
    let failed = app.session.set_compensation_all(&m, sf_core::compensation::Mode::Standard);
    assert!(failed.is_empty(), "{failed:?}");
    assert!(app.session.sample(6).channel_index("Comp-APC-A").is_some());
    settle(&mut app, &ctx);
}

#[test]
fn spectral_screen_builds_signatures_and_unmixes_real_controls() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
    if !dir.join("fitc alone.fcs").exists() {
        eprintln!("SKIPPED: fixtures missing");
        return;
    }
    let ctx = egui::Context::default();
    let mut app = App::new();
    let files: Vec<std::path::PathBuf> = ["fitc alone", "pe alone", "apc alone", "AF700 alone", "bv421 alone", "bv711 alone", "unstained", "multistain-bv421-pe-fitc-apc-bv711-AF700"].iter().map(|n| dir.join(format!("{n}.fcs"))).collect();
    app.load_files(&files);
    app.set_module(Module::Spectral);
    settle(&mut app, &ctx); // guesses the roles from the file names
    assert!(matches!(app.spec.roles[0], spectral_view::Assign::Single(ref l) if l == "FITC"));
    assert!(app.spec.roles[6] == spectral_view::Assign::Unstained);
    assert!(app.spec.roles[7] == spectral_view::Assign::None, "the stained sample is not a control");
    app.spec.af_k = 0;
    app.spectral_build().unwrap();
    settle(&mut app, &ctx);
    let m = app.spec.model.as_ref().unwrap();
    assert_eq!((m.f(), m.d()), (6, 6));
    assert!(m.qc.condition.is_finite());
    app.spectral_unmix(&[7]);
    settle(&mut app, &ctx);
    assert!(app.session.sample(7).channel_index("Unmix-FITC").is_some() && app.session.sample(7).channel_index("Unmix-Residual").is_some());
    assert!(app.spec.log[0].contains("median residual"), "{:?}", app.spec.log);
    // the unmixed channels are ordinary channels: they open on a plot axis and Dim. Reduction picks them up
    let w = app.session.open_population(7, GateTree::ROOT).unwrap();
    app.session.window_mut(w).unwrap().x_chan = "Unmix-FITC".into();
    app.session.window_mut(w).unwrap().y_chan = "Unmix-PE".into();
    assert!(app.session.window_points(w).unwrap().len() > 1000);
    assert!(discover_view::default_features(app.session.sample(7)).iter().all(|c| c.starts_with("Unmix-")));
    // errors are shown, not swallowed
    app.spec.roles = vec![spectral_view::Assign::None; 8];
    assert!(app.spectral_build().unwrap_err().contains("at least two"));
}

// ---- cell cycle, proliferation, apoptosis screens ---------------------------------------------------------

#[test]
fn analysis_screens_run_on_the_real_annexin_and_dna_files_and_show_honest_results() {
    let base = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs"));
    if !base.join("dual stain/annexin alone.fcs").exists() {
        eprintln!("SKIPPED: fixtures missing");
        return;
    }
    let ctx = egui::Context::default();
    let mut app = App::new();
    app.load_files(&[base.join("dual stain/annexin alone.fcs"), base.join("single stain/single-stain-pe.fcs")]);
    assert_eq!(app.session.samples().len(), 2);

    // apoptosis on the Annexin file: channels guessed (FL1 = Annexin, FL2 = viability), numbers equal the engine's
    app.set_module(Module::Apoptosis);
    settle(&mut app, &ctx);
    assert_eq!((app.ana.annexin_channel.as_str(), app.ana.pi_channel.as_str()), ("FL1-A", "FL2-A"));
    app.run_apoptosis();
    let (_, r, pts) = app.ana.apop.as_ref().expect(&app.status);
    assert!((r.quadrants.live - 78.32).abs() < 0.011 && (r.quadrants.necrotic - 14.84).abs() < 0.011, "{:?}", r.quadrants);
    assert!(pts.len() <= 5000 && pts.len() >= 2500);
    settle(&mut app, &ctx); // the plot and tables draw
    // the same channel twice is refused with a message, not run
    app.ana.pi_channel = app.ana.annexin_channel.clone();
    app.run_apoptosis();
    assert!(app.status.contains("two different channels"), "{}", app.status);

    // cell cycle on the PI channel of the second file; the poor fit is reported, not hidden
    app.set_module(Module::CellCycle);
    app.ana.sample = 1;
    settle(&mut app, &ctx);
    assert_eq!(app.ana.cycle_channel, "PI-A");
    app.run_cell_cycle();
    let (label, r) = app.ana.cycle.as_ref().expect(&app.status);
    assert!(label.contains("single-stain-pe.fcs"));
    assert!((r.g1_pct + r.s_pct + r.g2_pct - 100.0).abs() < 1e-9);
    assert!(r.r2 < 0.8, "this PE tube is not a DNA histogram, so the screen must warn (R2 {})", r.r2);
    settle(&mut app, &ctx);
    // a population that does not exist in the sample is an error, not a silent fall back to all events
    app.ana.restrict = vec!["Nope".into()];
    app.run_cell_cycle();
    assert!(app.status.contains("does not exist"), "{}", app.status);
    assert!(app.ana.cycle.is_none());
    app.ana.restrict.clear();

    // proliferation on a channel without dilution peaks must say so or return a result, never panic
    app.set_module(Module::Proliferation);
    settle(&mut app, &ctx);
    app.ana.prolif_channel = "PI-A".into();
    app.run_proliferation();
    settle(&mut app, &ctx);
    assert!(app.ana.prolif.is_some() || !app.status.is_empty());
}
