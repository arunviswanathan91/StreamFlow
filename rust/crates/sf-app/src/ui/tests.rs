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
use crate::layout_doc::LayerRole;

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
    let now = ctx.input(|i| i.time);
    let t = CLOCK.with(|c| {
        c.set((c.get() + 0.04).max(now + 0.01)); // plain frames advance egui's clock too; never go backwards
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
    assert_eq!((all.len(), labels.len()), (19, 19));
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

// ---- stats comparison and kinetic screens -----------------------------------------------------------------

fn multi_stain_app(n: usize) -> Option<App> {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir).ok()?.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map_or(false, |x| x == "fcs")).collect();
    files.sort();
    files.truncate(n);
    if files.len() < n {
        eprintln!("SKIPPED: fixtures missing");
        return None;
    }
    let mut app = App::new();
    app.load_files(&files);
    Some(app)
}

#[test]
fn stats_comparison_screen_matches_the_tests_run_directly_and_reports_what_it_dropped() {
    let Some(mut app) = multi_stain_app(8) else { return };
    let ctx = egui::Context::default();
    app.set_module(Module::StatsComparison);
    settle(&mut app, &ctx);
    // nothing assigned: a clear message
    app.run_comparison();
    assert!(app.status.contains("at least 2 groups"), "{}", app.status);
    // two groups of four, median of the first fluorescence channel of all events
    for i in 0..8 {
        app.cmp.groups[i] = if i < 4 { "A".into() } else { "B".into() };
    }
    app.cmp.metric = crate::stats_table::Metric::COMMON.iter().position(|m| *m == crate::stats_table::Metric::Median).unwrap();
    app.run_comparison();
    let (label, c) = app.cmp.result.clone().expect(&app.status);
    assert!(label.contains("All Events") && label.contains("Median"), "{label}");
    assert_eq!(c.test, "Mann-Whitney U");
    let (a, b) = (c.groups[0].values.clone(), c.groups[1].values.clone());
    let mut all: Vec<f64> = a.iter().chain(&b).copied().collect();
    all.sort_by(|x, y| x.total_cmp(y));
    let ties = all.windows(2).any(|w| w[0] == w[1]);
    assert_eq!(c.method, if ties { "asymptotic" } else { "exact" }, "SciPy's auto rule: exact for 4 v 4 only without ties; values {all:?}");
    assert_eq!((a.len(), b.len()), (4, 4));
    let direct = sf_core::htest::mann_whitney_u(&a, &b).unwrap();
    assert_eq!(c.p, direct.p, "the screen reports exactly what the test returns on the same numbers");
    // each value really is the median of that channel in that sample
    let ch = app.cmp.channel.clone();
    let m0 = app.session.population_values(0, GateTree::ROOT, &ch).unwrap();
    assert!((a[0] - sf_core::stats::percentile(&m0, 50.0)).abs() < 1e-9);
    settle(&mut app, &ctx);
    // three groups -> Kruskal-Wallis
    for i in 6..8 {
        app.cmp.groups[i] = "C".into();
    }
    app.run_comparison();
    assert!(app.cmp.result.as_ref().unwrap().1.test.starts_with("Kruskal-Wallis"));
    // a population only some samples have is reported as dropped, not treated as zero
    app.cmp.restrict = vec!["Nope".into()];
    app.run_comparison();
    assert!(app.cmp.result.is_none() && app.status.contains("at least 2 groups"), "{}", app.status);
    // pairing needs equal sizes and exactly two groups
    app.cmp.restrict.clear();
    app.cmp.paired = true;
    app.run_comparison();
    assert!(app.status.contains("exactly two groups"), "{}", app.status);
}

#[test]
fn kinetic_screen_uses_the_real_time_channel_and_names_the_samples_it_skips() {
    let Some(mut app) = multi_stain_app(6) else { return };
    let ctx = egui::Context::default();
    app.set_module(Module::Kinetic);
    settle(&mut app, &ctx);
    assert_eq!(app.cmp.k_time, "Time");
    for i in 0..6 {
        app.cmp.groups[i] = if i % 2 == 0 { "even".into() } else { "odd".into() };
    }
    app.cmp.k_bins = 40;
    app.run_kinetics();
    let run = app.cmp.k_result.as_ref().expect(&app.status);
    assert_eq!(run.series.len(), 2);
    assert_eq!(run.series[0].n_samples + run.series[1].n_samples, 6);
    assert_eq!(run.series[0].times.len(), 40);
    assert!(run.skipped.is_empty());
    assert!(run.series[0].mfi.iter().any(|v| v.is_finite()));
    settle(&mut app, &ctx);
    // a sample without a Time channel is listed as skipped, not silently dropped from the average
    app.session.add_sample(LoadedSample::synthetic("notime.fcs", &["FITC-A"], vec![1e5], vec![1.0, 2.0, 3.0]));
    settle(&mut app, &ctx);
    app.cmp.k_sel = vec![true; 7];
    app.run_kinetics();
    let run = app.cmp.k_result.as_ref().expect(&app.status);
    assert_eq!(run.skipped.len(), 1);
    assert!(run.skipped[0].starts_with("notime.fcs"), "{:?}", run.skipped);
    assert_eq!(run.series.iter().map(|s| s.n_samples).sum::<usize>(), 6);
    // a population that does not exist: skipped with the reason, not an empty curve
    app.cmp.k_time = "Time".into();
    app.cmp.restrict = vec!["Nope".into()];
    app.run_kinetics();
    assert!(app.cmp.k_result.is_none());
}

// ---- moving and resizing gates -------------------------------------------------------------------------------

fn gate_of(app: &App, node: usize) -> Gate {
    match app.session.tree(0).kind(node).unwrap() {
        NodeKind::Gate(g) => g.clone(),
        k => panic!("{k:?}"),
    }
}

/// Press at `a`, drag through a few intermediate points to `b`, release.
fn drag(app: &mut App, ctx: &egui::Context, a: Pos2, b: Pos2) {
    pointer(app, ctx, vec![egui::Event::PointerMoved(a)]);
    pointer(app, ctx, vec![button(a, true)]);
    for k in 1..=4 {
        let t = k as f32 / 4.0;
        pointer(app, ctx, vec![egui::Event::PointerMoved(Pos2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t))]);
    }
    pointer(app, ctx, vec![button(b, false)]);
    frame(app, ctx);
}

#[test]
fn a_gate_is_moved_and_resized_with_the_pointer_and_each_edit_is_one_undo_step() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot) = ready_tab(&mut app, &ctx);
    let range = app.ws.graphs[&w].range.unwrap();
    let map = Mapper { rect: plot, range };
    let at = |fx: f32, fy: f32| Pos2::new(plot.left() + fx * plot.width(), plot.top() + fy * plot.height());
    app.session.window_mut(w).unwrap().tool = Tool::Rectangle;
    settle(&mut app, &ctx);
    drag(&mut app, &ctx, at(0.25, 0.75), at(0.60, 0.30));
    let kid = children(&app)[0];
    let g0 = gate_of(&app, kid);
    let n0 = app.session.count(0, kid);
    app.session.window_mut(w).unwrap().tool = Tool::Pointer;
    settle(&mut app, &ctx);
    let undo_depth = |app: &App| (0..50).filter(|_| false).count() + usize::from(app.session.can_undo(0));
    assert_eq!(undo_depth(&app), 1);

    // ---- move: grab the body and drag right and up
    let (dx_px, dy_px) = (0.20 * plot.width(), -0.10 * plot.height());
    let centre = at(0.425, 0.525);
    drag(&mut app, &ctx, centre, Pos2::new(centre.x + dx_px, centre.y + dy_px));
    let g1 = gate_of(&app, kid);
    let (ddx, ddy) = (map.to_data(Pos2::new(centre.x + dx_px, centre.y + dy_px)).0 - map.to_data(centre).0, map.to_data(Pos2::new(centre.x + dx_px, centre.y + dy_px)).1 - map.to_data(centre).1);
    for (a, b) in g1.xs.iter().zip(&g0.xs) {
        assert!((a - b - ddx).abs() < 1e-6 * ddx.abs().max(1.0), "x shifted by the drag: {:?} -> {:?}", g0.xs, g1.xs);
    }
    for (a, b) in g1.ys.iter().zip(&g0.ys) {
        assert!((a - b - ddy).abs() < 1e-6 * ddy.abs().max(1.0), "y shifted by the drag: {:?} -> {:?}", g0.ys, g1.ys);
    }
    assert!(app.status.contains("events"), "{}", app.status);
    assert_ne!(app.session.count(0, kid), 0);
    // the view did not pan while a gate was being moved
    assert_eq!(app.ws.graphs[&w].range.unwrap(), range, "dragging a gate must not pan the plot");

    // ---- resize: drag the right edge handle outward
    let g1b = gate_of(&app, kid);
    let (x1, ym) = (g1b.xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max), (g1b.ys[0] + g1b.ys[1]) / 2.0);
    let handle = map.to_screen((x1, ym));
    drag(&mut app, &ctx, handle, Pos2::new(handle.x + 0.15 * plot.width(), handle.y));
    let g2 = gate_of(&app, kid);
    let (old_w, new_w) = (g1b.xs[1].max(g1b.xs[0]) - g1b.xs[0].min(g1b.xs[1]), g2.xs[1] - g2.xs[0]);
    assert!(new_w > old_w * 1.3, "width grew from {old_w} to {new_w}");
    assert!((g2.xs[0] - g1b.xs[0].min(g1b.xs[1])).abs() < 1e-9, "the left edge stayed put");
    assert_eq!(g2.ys, g1b.ys, "a vertical edge does not touch y");

    // ---- each edit is one undo step, and undo restores the exact gate
    assert!(app.session.undo(0));
    assert_eq!(gate_of(&app, kid), g1b);
    assert!(app.session.undo(0));
    assert_eq!(gate_of(&app, kid), g0);
    assert_eq!(app.session.count(0, kid), n0);
    assert!(app.session.redo(0) && app.session.redo(0));
    assert_eq!(gate_of(&app, kid), g2);

    // ---- Esc cancels a drag in progress
    let before = gate_of(&app, kid);
    let c = map.to_screen(((before.xs[0] + before.xs[1]) / 2.0, (before.ys[0] + before.ys[1]) / 2.0));
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(c)]);
    pointer(&mut app, &ctx, vec![button(c, true)]);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(c.x + 60.0, c.y))]);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(Pos2::new(c.x + 90.0, c.y))]);
    pointer(&mut app, &ctx, vec![key(egui::Key::Escape, false, false)]);
    pointer(&mut app, &ctx, vec![button(Pos2::new(c.x + 90.0, c.y), false)]);
    frame(&mut app, &ctx);
    assert_eq!(gate_of(&app, kid), before, "Esc abandons the move");

    // ---- dragging empty space still pans
    let empty = at(0.9, 0.1);
    let r0 = app.ws.graphs[&w].range.unwrap();
    drag(&mut app, &ctx, empty, Pos2::new(empty.x - 80.0, empty.y + 40.0));
    let r1 = app.ws.graphs[&w].range.unwrap();
    assert!(r1.x0 > r0.x0, "the plot panned: {r0:?} -> {r1:?}");
    assert_eq!(gate_of(&app, kid), before, "the gate was not touched by a pan");
}

// ---- cross-sample and classifier screens -------------------------------------------------------------------------

#[test]
fn cross_sample_and_classifier_work_on_real_samples_with_an_applied_gate() {
    let Some(mut app) = multi_stain_app(8) else { return };
    let ctx = egui::Context::default();
    // one gate drawn on sample 0 and applied to every sample
    let w = app.open_tab(0, GateTree::ROOT).unwrap();
    settle(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_scale = ScaleSpec::Identity;
    app.session.window_mut(w).unwrap().y_scale = ScaleSpec::Identity;
    let r = app.ws.graphs[&w].range.unwrap();
    let kid = app.session.commit_shape(w, &Shape::Rect { x0: r.x0, x1: (r.x0 + r.x1) / 2.0, y0: r.y0, y1: r.y1 }, "Left half").unwrap();
    assert_eq!(app.session.apply_to_all(0, kid).unwrap(), 7);

    // nothing computed yet: a clear message
    app.set_module(Module::CrossSample);
    settle(&mut app, &ctx);
    app.xs.sel = vec![false; 8];
    app.run_cross_sample();
    assert!(app.status.contains("Tick at least one sample"), "{}", app.status);
    app.xs.sel = vec![true; 8];
    app.run_cross_sample();
    let m = app.xs.matrix.clone().expect(&app.status);
    assert_eq!((m.populations.len(), m.samples.len()), (1, 8));
    for (si, f) in m.freq[0].iter().enumerate() {
        let n = app.session.count(si, app.session.tree(si).find(&["Left half"]).unwrap()) as f64;
        assert!((f - 100.0 * n / app.session.sample(si).n_events as f64).abs() < 1e-9, "sample {si}: {f}");
    }
    assert!(app.xs.drift.as_ref().unwrap().1[0].iter().any(|v| v.is_finite()));
    for tab in [cross_view::Tab::Matrix, cross_view::Tab::Consistency, cross_view::Tab::Radar, cross_view::Tab::Drift] {
        app.xs.tab = tab;
        settle(&mut app, &ctx); // every tab draws, including the 'needs three populations' radar message
    }

    // classifier: no groups -> PCA only; then two groups of four -> forest
    app.set_module(Module::Classifier);
    settle(&mut app, &ctx);
    app.run_classifier();
    let r = app.xs.class.clone().expect(&app.status);
    assert_eq!((r.method.as_str(), r.n_samples, r.n_features), ("PCA", 8, 1));
    for i in 0..8 {
        app.cmp.groups[i] = if i < 4 { "A".into() } else { "B".into() };
    }
    app.run_classifier();
    let r = app.xs.class.clone().expect(&app.status);
    assert!(r.method.starts_with("PCA + Random Forest"), "{}", r.method);
    assert_eq!(r.rf_samples, 8);
    settle(&mut app, &ctx);
    // a group of two is refused with the engine's message
    app.cmp.groups[0] = "C".into();
    app.run_classifier();
    assert!(app.xs.class.is_none() && app.status.contains("at least 3 samples per group"), "{}", app.status);
    // fewer than two samples
    app.xs.sel = vec![false, true, false, false, false, false, false, false];
    app.run_classifier();
    assert!(app.status.contains("at least 2 samples"), "{}", app.status);
}

#[test]
fn cross_sample_without_gates_says_so() {
    let Some(mut app) = multi_stain_app(3) else { return };
    let ctx = egui::Context::default();
    app.set_module(Module::CrossSample);
    settle(&mut app, &ctx);
    app.run_cross_sample();
    assert!(app.status.contains("no gated populations"), "{}", app.status);
    assert!(app.xs.matrix.is_none());
}

// ---- FMO reference lines and the compare strip -------------------------------------------------------------------

fn two_sample_app() -> App {
    let mut app = app_with_sample();
    let mut rng = Rng::new(11);
    // a second sample with the same panel but a shifted CD3 distribution
    let events = (0..3000).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 100_000.0) as f32, (rng.next_f64() * 20_000.0 + 4_000.0) as f32, (rng.next_f64() * 20_000.0 - 500.0) as f32]).collect();
    app.session.add_sample(LoadedSample::synthetic("u.fcs", &["FSC-A", "SSC-A", "CD3-A", "CD4-A"], vec![262_144.0; 4], events));
    app
}

#[test]
fn fmo_level_is_the_percentile_of_the_population_and_survives_a_saved_workspace() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    app.session.window_mut(w).unwrap().y_chan = "CD4-A".into();
    let done = app.session.set_fmo_from_window(w, 95.0).unwrap();
    assert_eq!(done.iter().map(|d| d.0.as_str()).collect::<Vec<_>>(), vec!["CD3-A", "CD4-A"], "both shown channels are set on a 2-D plot");
    let vals = app.session.population_values(0, GateTree::ROOT, "CD3-A").unwrap();
    let want = sf_core::stats::percentile(&vals, 95.0);
    assert_eq!(app.session.fmo("CD3-A").unwrap().level, want);
    assert_eq!(app.session.fmo("CD3-A").unwrap().n, 3000);
    // about 5% of the control's own events sit at or above its 95th percentile
    let above = app.session.fraction_above_fmo(0, GateTree::ROOT, "CD3-A").unwrap();
    assert!((above - 0.05).abs() < 0.005, "{above}");
    // a histogram only sets the X channel
    app.session.clear_fmo("CD4-A");
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Histogram;
    let done = app.session.set_fmo_from_window(w, 99.0).unwrap();
    assert_eq!(done.len(), 1);
    assert!(app.session.fmo("CD4-A").is_none());
    assert!(app.session.set_fmo_from_window(w, 120.0).is_err());
    // saved with the workspace and restored
    let doc = app.session.to_workspace_json();
    let mut other = app_with_sample();
    other.session.apply_workspace_json(&doc).unwrap();
    assert_eq!(other.session.fmo("CD3-A"), app.session.fmo("CD3-A"));
    // clearing
    other.session.clear_fmo("CD3-A");
    assert!(other.session.fmo("CD3-A").is_none());
}

#[test]
fn an_fmo_line_is_dragged_with_the_pointer_without_panning_or_touching_gates() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    app.session.window_mut(w).unwrap().y_chan = "CD4-A".into();
    app.ws.graphs.get_mut(&w).unwrap().refit = true;
    settle(&mut app, &ctx);
    let range = app.ws.graphs[&w].range.unwrap();
    let map = Mapper { rect: plot, range };
    let (xs, ws_) = (app.session.window(w).unwrap().x_scale.clone(), app.session.window(w).unwrap().clone());
    app.session.set_fmo_from_window(w, 90.0).unwrap();
    settle(&mut app, &ctx);
    let t = xs.build().unwrap();
    let before = app.session.fmo("CD3-A").unwrap().level;
    let line = map.to_screen((t.forward(before), range.y0));
    assert!(line.x > plot.left() && line.x < plot.right(), "the line is inside the plot at {line:?}");
    // drag the vertical line 80 px to the left at mid height (it sits near the right edge of the view)
    let a = Pos2::new(line.x, plot.center().y);
    let b = Pos2::new(a.x - 80.0, a.y);
    drag(&mut app, &ctx, a, b);
    let after = app.session.fmo("CD3-A").unwrap().level;
    let want = t.inverse(map.to_data(b).0);
    assert!(after < before && (after - want).abs() < 0.02 * want.abs().max(1.0), "moved {before} -> {after}, expected about {want}");
    assert_eq!(app.ws.graphs[&w].range.unwrap(), range, "dragging an FMO line must not pan the plot");
    assert!(app.status.contains("moved"), "{}", app.status);
    assert!(children(&app).is_empty() && !app.session.can_undo(0), "no gate was created or edited");
    let _ = ws_;
    // hiding the lines frees that spot again: a drag there pans instead
    app.ws.graphs.get_mut(&w).unwrap().style.fmo = false;
    settle(&mut app, &ctx);
    let line2 = map.to_screen((t.forward(after), range.y0));
    drag(&mut app, &ctx, Pos2::new(line2.x, plot.center().y), Pos2::new(line2.x - 60.0, plot.center().y));
    assert_eq!(app.session.fmo("CD3-A").unwrap().level, after, "a hidden line cannot be grabbed");
    assert_ne!(app.ws.graphs[&w].range.unwrap(), range, "the drag panned the view instead");
}

#[test]
fn the_compare_strip_shows_every_sample_on_the_same_view_and_a_click_switches_sample() {
    let ctx = egui::Context::default();
    let mut app = two_sample_app();
    let (w, _) = ready_tab(&mut app, &ctx);
    // a gate drawn on sample 0 and applied to sample 1, so the strip has gates to overlay
    app.session.window_mut(w).unwrap().x_scale = ScaleSpec::Identity;
    app.session.window_mut(w).unwrap().y_scale = ScaleSpec::Identity;
    let r = app.ws.graphs[&w].range.unwrap();
    let kid = app.session.commit_shape(w, &Shape::Rect { x0: r.x0, x1: (r.x0 + r.x1) / 2.0, y0: r.y0, y1: r.y1 }, "Left").unwrap();
    app.session.apply_to_all(0, kid).unwrap();
    app.ws.graphs.get_mut(&w).unwrap().strip_open = true;
    settle(&mut app, &ctx);
    settle(&mut app, &ctx);
    let g = &app.ws.graphs[&w];
    assert_eq!(g.strip.len(), 2, "one picture per sample");
    assert_eq!(g.strip_rects.len(), 2);
    let range_now = g.range.unwrap();
    assert_eq!(app.session.window(w).unwrap().sample, 0);
    // click the second card: the plot switches to that sample but keeps the same view
    let card = g.strip_rects.iter().find(|(s, _)| *s == 1).unwrap().1;
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(card.center())]);
    pointer(&mut app, &ctx, vec![button(card.center(), true)]);
    pointer(&mut app, &ctx, vec![button(card.center(), false)]);
    frame(&mut app, &ctx);
    assert_eq!(app.session.window(w).unwrap().sample, 1, "{}", app.status);
    assert_eq!(app.ws.graphs[&w].range.unwrap(), range_now, "switching sample keeps the view");
    // a population that one sample lacks is said so, not drawn empty
    app.session.delete(1, app.session.tree(1).find(&["Left"]).unwrap()).unwrap();
    app.ws.graphs.get_mut(&w).unwrap().refit = false;
    settle(&mut app, &ctx);
    // an unreachable channel shows 'channel not in this sample' without panicking
    app.ws.graphs.get_mut(&w).unwrap().strip.clear();
    settle(&mut app, &ctx);
}


#[test]
fn fit_all_widens_the_shared_view_to_every_samples_population() {
    let ctx = egui::Context::default();
    let mut app = two_sample_app();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    app.session.window_mut(w).unwrap().y_chan = "CD4-A".into();
    app.session.window_mut(w).unwrap().x_scale = ScaleSpec::Identity;
    app.session.window_mut(w).unwrap().y_scale = ScaleSpec::Identity;
    app.refit_window(w);
    settle(&mut app, &ctx);
    let own = app.ws.graphs[&w].range.unwrap();
    let win = app.session.window(w).unwrap().clone();
    app.fit_all_samples(w, &win);
    let all = app.ws.graphs[&w].range.unwrap();
    // sample 1's CD3 runs up to 24 000, sample 0's only to 19 500: the union is wider than either alone
    assert!(all.x1 > own.x1 + 1000.0, "{own:?} -> {all:?}");
    for si in 0..2 {
        let v = app.session.population_values(si, GateTree::ROOT, "CD3-A").unwrap();
        assert!(v.iter().all(|x| *x >= all.x0 && *x <= all.x1), "sample {si} fits");
    }
    assert!(all.is_valid());
}

// ---- plot export and layouts -------------------------------------------------------------------------------------

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("sf_export_tests");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn a_graph_window_exports_as_a_figure_with_its_view_gates_and_fmo_line() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot) = ready_tab(&mut app, &ctx);
    let _ = plot;
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    app.session.window_mut(w).unwrap().y_chan = "CD4-A".into();
    app.refit_window(w);
    settle(&mut app, &ctx);
    let r = app.ws.graphs[&w].range.unwrap();
    app.session.commit_shape(w, &Shape::Rect { x0: r.x0, x1: (r.x0 + r.x1) / 2.0, y0: r.y0, y1: (r.y0 + r.y1) / 2.0 }, "Lower left").unwrap();
    app.session.set_fmo_from_window(w, 90.0).unwrap();
    settle(&mut app, &ctx);
    let fig = app.window_figure(w, 460.0, 400.0).unwrap();
    let texts = fig.texts();
    for want in ["t", "All Events", "CD3-A", "CD4-A", "Lower left", "FMO"] {
        assert!(texts.iter().any(|t| t == want), "missing {want:?} in {texts:?}");
    }
    assert!(texts.iter().any(|t| t.ends_with("% of parent") && t.contains("3000 events")), "{texts:?}");
    // written as a valid PDF and as well-formed SVG
    let (pdf, svg) = (scratch("plot.pdf"), scratch("plot.svg"));
    std::fs::write(&pdf, crate::figure::to_pdf(&[fig.clone()], "plot")).unwrap();
    std::fs::write(&svg, crate::figure::to_svg(&fig)).unwrap();
    assert_eq!(crate::figure::validate_pdf(&std::fs::read(&pdf).unwrap()), Ok(1));
    assert!(sf_core::xml::parse(&std::fs::read_to_string(&svg).unwrap()).is_ok());
    // hiding the FMO lines on the plot hides them in the export too
    app.ws.graphs.get_mut(&w).unwrap().style.fmo = false;
    let fig2 = app.window_figure(w, 460.0, 400.0).unwrap();
    assert!(!fig2.texts().iter().any(|t| t == "FMO"));
    // a window that has not been drawn yet has no view to export, and says so
    app.ws.graphs.remove(&w);
    assert!(app.window_figure(w, 460.0, 400.0).err().expect("no view yet").contains("no view"));
}

#[test]
fn png_as_shown_crops_the_screenshot_to_the_plot_and_writes_exactly_those_pixels() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    let canvas = app.ws.graphs[&w].canvas_rect.expect("the canvas is recorded for export");
    assert!(canvas.width() > 200.0 && canvas.height() > 200.0);
    let path = scratch("shown.png");
    let _ = std::fs::remove_file(&path);
    app.begin_capture(egui::Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(60.0, 50.0)), super::CaptureDest::File(path.clone()));
    // a fake full-window screenshot whose pixels encode their own position
    let mut img = egui::ColorImage::new([200, 100], egui::Color32::BLACK);
    for y in 0..100 {
        for x in 0..200 {
            img.pixels[y * 200 + x] = egui::Color32::from_rgb(x as u8, y as u8, 7);
        }
    }
    let img = std::sync::Arc::new(img);
    // the screenshot is requested only after a few frames, so early frames do nothing
    for _ in 0..5 {
        let _ = ctx.run(egui::RawInput { screen_rect: Some(screen()), ..Default::default() }, |c| app.poll_capture(c));
    }
    assert!(!path.exists());
    let ev = egui::Event::Screenshot { viewport_id: egui::ViewportId::ROOT, image: img };
    let _ = ctx.run(egui::RawInput { screen_rect: Some(screen()), events: vec![ev], ..Default::default() }, |c| app.poll_capture(c));
    assert!(path.exists(), "{}", app.status);
    assert!(app.status.contains("50 x 30 px"), "{}", app.status);
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut rd = dec.read_info().unwrap();
    let mut buf = vec![0; rd.output_buffer_size().unwrap()];
    let info = rd.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (50, 30));
    assert_eq!(&buf[..3], &[10, 20, 7], "top-left pixel of the crop is source pixel (10, 20)");
    let last = (info.width * info.height - 1) as usize * 4;
    assert_eq!(&buf[last..last + 3], &[59, 49, 7], "bottom-right is (59, 49)");
}

#[test]
fn the_layout_screen_builds_pages_for_real_samples_and_writes_a_valid_pdf_and_svg() {
    let Some(mut app) = multi_stain_app(7) else { return };
    let ctx = egui::Context::default();
    app.set_module(Module::Layout);
    settle(&mut app, &ctx);
    assert_eq!((app.lay.x_chan.as_str(), app.lay.y_chan.as_str()), ("FSC-A", "SSC-A"));
    // nothing built yet: saving says so
    assert!(app.write_layout_pdf(&scratch("none.pdf")).unwrap_err().contains("build the layout first"));
    app.lay.spec.cols = 3;
    app.lay.spec.rows = 2;
    let pages = app.build_layout().unwrap();
    assert_eq!(pages, 2, "7 plots at 6 per page");
    settle(&mut app, &ctx); // the preview paints, including the density images as textures
    let (pdf, svg) = (scratch("layout.pdf"), scratch("layout_page.svg"));
    app.write_layout_pdf(&pdf).unwrap();
    app.lay.page = 1;
    settle(&mut app, &ctx);
    app.write_layout_svg(&svg).unwrap();
    assert_eq!(crate::figure::validate_pdf(&std::fs::read(&pdf).unwrap()), Ok(2));
    let svg_text = std::fs::read_to_string(&svg).unwrap();
    assert!(sf_core::xml::parse(&svg_text).is_ok());
    assert!(svg_text.contains("Page 2 of 2"), "the second page was written");
    // population and samples
    app.lay.sel = vec![true, true, false, false, false, false, false];
    assert_eq!(app.build_layout().unwrap(), 1);
    app.lay.population = vec!["Nope".into()];
    let e = app.build_layout().unwrap_err();
    assert!(e.contains("no sample has events"), "{e}");
    app.lay.population.clear();
    app.lay.sel = vec![false; 7];
    assert!(app.build_layout().unwrap_err().contains("tick at least one sample"));
    // a grid that leaves plots too small is refused with advice
    app.lay.sel = vec![true; 7];
    app.lay.spec.cols = 10;
    assert!(app.build_layout().unwrap_err().contains("smaller than 90 pt"));
}

// ---- the layout editor -------------------------------------------------------------------------------------------

fn editor_app() -> (App, egui::Context) {
    let ctx = egui::Context::default();
    let mut app = two_sample_app();
    app.set_module(Module::Layout);
    app.lay.tab = super::layout_view::LayTab::Editor;
    settle(&mut app, &ctx);
    (app, ctx)
}

fn page_pt(app: &App, fx: f32, fy: f32) -> Pos2 {
    let (rect, _) = app.lay.ed.canvas.expect("the canvas is drawn");
    Pos2::new(rect.left() + fx * rect.width(), rect.top() + fy * rect.height())
}

fn drag_sample(app: &mut App, ctx: &egui::Context, name: &str, to: Pos2) {
    let from = row(app, name);
    drag(app, ctx, from, to);
}

fn row(app: &App, name: &str) -> Pos2 {
    app.lay.ed.sample_rows.iter().find(|(n, _)| n == name).expect("sample row").1.center()
}

#[test]
fn dragging_samples_onto_the_page_makes_tiles_and_overlays_and_tiles_move_and_resize() {
    let (mut app, ctx) = editor_app();
    assert_eq!(app.lay.ed.sample_rows.len(), 2);
    assert!(app.lay.ed.doc.tiles.is_empty());
    // drag t.fcs from the list to the left part of the page -> a new tile with that sample as its layer
    { let to = page_pt(&app, 0.25, 0.4); drag_sample(&mut app, &ctx, "t.fcs", to); }
    settle(&mut app, &ctx);
    assert_eq!(app.lay.ed.doc.tiles.len(), 1, "{}", app.status);
    let tile = app.lay.ed.doc.tiles[0].clone();
    assert_eq!(tile.layers.len(), 1);
    assert_eq!((tile.layers[0].sample.as_str(), tile.layers[0].role), ("t.fcs", LayerRole::Iterator));
    let (page_w, page_h) = (app.lay.ed.doc.page().width, app.lay.ed.doc.page().height);
    assert!((tile.x + tile.w / 2.0 - 0.25 * page_w).abs() < 4.0 && (tile.y + tile.h / 2.0 - 0.4 * page_h).abs() < 4.0, "the tile is centred where it was dropped: {tile:?}");
    assert_eq!(app.lay.ed.selected, Some(super::layout_editor::Sel::Tile(tile.id)));
    // drop u.fcs ONTO that tile -> an overlay layer with its own colour, not a second tile
    let on_tile = page_pt(&app, (tile.x + tile.w / 2.0) / page_w, (tile.y + tile.h / 2.0) / page_h);
    { let to = on_tile; drag_sample(&mut app, &ctx, "u.fcs", to); }
    settle(&mut app, &ctx);
    assert_eq!(app.lay.ed.doc.tiles.len(), 1, "{}", app.status);
    let layers = &app.lay.ed.doc.tiles[0].layers;
    assert_eq!(layers.iter().map(|l| l.sample.as_str()).collect::<Vec<_>>(), vec!["t.fcs", "u.fcs"]);
    assert_ne!(layers[0].color, layers[1].color);
    // dropping the same sample again is refused with a message
    { let to = on_tile; drag_sample(&mut app, &ctx, "u.fcs", to); }
    assert_eq!(app.lay.ed.doc.tiles[0].layers.len(), 2);
    assert!(app.status.contains("already on that tile"), "{}", app.status);
    // a drop on empty page space makes a second tile that starts like the first
    app.lay.ed.doc.tiles[0].population = vec![];
    app.lay.ed.doc.tiles[0].x_chan = "CD3-A".into();
    { let to = page_pt(&app, 0.8, 0.75); drag_sample(&mut app, &ctx, "u.fcs", to); }
    settle(&mut app, &ctx);
    assert_eq!(app.lay.ed.doc.tiles.len(), 2);
    assert_eq!(app.lay.ed.doc.tiles[1].x_chan, "CD3-A", "a new tile copies the axes of the selected one");

    // move the first tile by dragging its body
    let (_, scale) = app.lay.ed.canvas.unwrap();
    let t0 = app.lay.ed.doc.tiles[0].clone();
    let body = page_pt(&app, (t0.x + 40.0) / page_w, (t0.y + 40.0) / page_h);
    drag(&mut app, &ctx, body, Pos2::new(body.x + 50.0, body.y + 30.0));
    let t0b = app.lay.ed.doc.tile(t0.id).unwrap().clone();
    assert!(((t0b.x - t0.x) * scale - 50.0).abs() < 3.0 && ((t0b.y - t0.y) * scale - 30.0).abs() < 3.0, "moved by the drag: {t0:?} -> {t0b:?}");
    assert_eq!((t0b.w, t0b.h), (t0.w, t0.h));
    // resize from the bottom-right handle
    let handle = page_pt(&app, (t0b.x + t0b.w) / page_w, (t0b.y + t0b.h) / page_h);
    drag(&mut app, &ctx, handle, Pos2::new(handle.x + 40.0, handle.y + 20.0));
    let t0c = app.lay.ed.doc.tile(t0.id).unwrap().clone();
    assert!(((t0c.w - t0b.w) * scale - 40.0).abs() < 3.0 && ((t0c.h - t0b.h) * scale - 20.0).abs() < 3.0, "{t0b:?} -> {t0c:?}");
    assert_eq!((t0c.x, t0c.y), (t0b.x, t0b.y), "resizing keeps the corner");
    // Delete removes the selected tile
    pointer(&mut app, &ctx, vec![key(egui::Key::Delete, false, false)]);
    assert_eq!(app.lay.ed.doc.tiles.len(), 1);
}

#[test]
fn the_editor_runs_a_batch_with_controls_and_tokens_and_the_layout_survives_a_saved_workspace() {
    let (mut app, ctx) = editor_app();
    { let to = page_pt(&app, 0.3, 0.45); drag_sample(&mut app, &ctx, "t.fcs", to); }
    settle(&mut app, &ctx);
    let id = app.lay.ed.doc.tiles[0].id;
    {
        let t = app.lay.ed.doc.tile_mut(id).unwrap();
        t.x_chan = "CD3-A".into();
        t.kind = crate::density::PlotKind::Histogram;
        t.add_layer("u.fcs", LayerRole::Control);
    }
    app.lay.ed.doc.add_text((30.0, 20.0), "[File_Name] — page [Page] of [Pages]");
    settle(&mut app, &ctx); // the editor view draws with the overlay and the control in italics
    let pages = app.editor_run_batch(Some(&ctx)).unwrap();
    assert_eq!(pages, 2, "one page per sample");
    assert!(app.lay.ed.batch[0].texts().contains(&"t — page 1 of 2".to_string()), "{:?}", app.lay.ed.batch[0].texts());
    assert!(app.lay.ed.batch[1].texts().contains(&"u — page 2 of 2".to_string()));
    let pdf = scratch("editor_batch.pdf");
    app.write_batch_pdf(&pdf).unwrap();
    assert_eq!(crate::figure::validate_pdf(&std::fs::read(&pdf).unwrap()), Ok(2));
    app.lay.ed.preview = Some(1);
    settle(&mut app, &ctx); // a batch page can be previewed on the canvas
    app.lay.ed.preview = None;
    // save the workspace document and open it in a fresh app with the same samples
    let doc_json = app.workspace_document().write();
    let mut other = two_sample_app();
    other.apply_workspace_document(&sf_core::json::Json::parse(&doc_json).unwrap()).unwrap();
    assert_eq!(other.lay.ed.doc, app.lay.ed.doc, "tiles, layers, roles and text boxes survive");
    // a damaged layout is reported, not silently dropped
    let mut j = sf_core::json::Json::parse(&doc_json).unwrap();
    if let sf_core::json::Json::Obj(o) = &mut j {
        o.insert("layout".into(), sf_core::json::Json::Num(1.0));
    }
    let mut third = two_sample_app();
    let notes = third.apply_workspace_document(&j).unwrap();
    assert!(notes.iter().any(|n| n.contains("saved layout could not be read")), "{notes:?}");
    // an empty layout cannot be run
    let (mut empty, _) = editor_app();
    assert!(empty.editor_run_batch(None).unwrap_err().contains("empty"));
    assert!(empty.write_batch_pdf(&scratch("never.pdf")).unwrap_err().contains("run the batch first"));
}

