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
    assert_eq!(app.session.window(w).unwrap().tool, Tool::Pointer, "after a gate is drawn the pointer is back, so a double click opens it");
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
    assert_eq!(app.session.window(w).unwrap().tool, Tool::Pointer, "and after quadrants too");
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
    // the unmixed plot opens on request, and the non-negative method never reports a negative amount
    app.open_unmixed_plot(7, true).unwrap();
    settle(&mut app, &ctx);
    app.spec.method = sf_core::spectral::Method::Nnls;
    app.spectral_unmix(&[7]);
    settle(&mut app, &ctx);
    let smp = app.session.sample(7);
    let (d, c) = (smp.channels.len(), smp.channel_index("Unmix-FITC").unwrap());
    assert!((0..smp.n_events).all(|r| smp.events[r * d + c] >= 0.0), "NNLS: no negative FITC");
    assert!(app.spec.log[0].contains("median residual"), "{:?}", app.spec.log);
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
    let left = app.session.tree(1).find(&["Left"]).unwrap();
    app.session.release_gate(1, left).ok();
    app.session.delete(1, left).unwrap();
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


// ---- plot kinds and their options, on screen and in exports ------------------------------------------------------

#[test]
fn every_plot_kind_draws_on_screen_and_exports_and_the_histogram_unit_follows_the_option() {
    use crate::density::{HistStyle, HistY, PlotKind};
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    app.session.window_mut(w).unwrap().y_chan = "CD4-A".into();
    app.refit_window(w);
    settle(&mut app, &ctx);
    for kind in PlotKind::ALL {
        app.session.window_mut(w).unwrap().kind = kind;
        settle(&mut app, &ctx);
        let g = &app.ws.graphs[&w];
        assert_eq!(g.curve.is_some(), kind == PlotKind::Histogram, "{kind:?}: only a histogram has a curve");
        assert_eq!(g.tex.is_some(), kind != PlotKind::Histogram, "{kind:?}: every 2-D kind has an image");
        let fig = app.window_figure(w, 460.0, 400.0).unwrap();
        assert_eq!(crate::figure::validate_pdf(&crate::figure::to_pdf(&[fig], "k")), Ok(1), "{kind:?}");
    }
    // the histogram's Y unit is what the option says, on screen and in the export
    app.session.window_mut(w).unwrap().kind = PlotKind::Histogram;
    for y in HistY::ALL {
        app.ws.graphs.get_mut(&w).unwrap().plot.hist_y = y;
        settle(&mut app, &ctx);
        assert_eq!(app.ws.graphs[&w].curve.as_ref().unwrap().y_label, y.label(), "on screen");
        assert!(app.window_figure(w, 460.0, 400.0).unwrap().texts().contains(&y.label().to_string()), "in the export: {y:?}");
    }
    // count: the axis top is a clean number at or above the tallest bin of the events shown
    app.ws.graphs.get_mut(&w).unwrap().plot.hist_y = HistY::Count;
    app.ws.graphs.get_mut(&w).unwrap().plot.hist_smooth = false;
    settle(&mut app, &ctx);
    let c = app.ws.graphs[&w].curve.clone().unwrap();
    let peak = c.heights.iter().cloned().fold(0.0, f64::max) * c.axis_max;
    assert!(peak >= 1.0 && c.axis_max >= peak && (c.total as usize) <= 3000, "peak {peak}, axis {}, total {}", c.axis_max, c.total);
    // the style changes what is drawn: bars have a different outline from the filled curve
    let outline = |app: &mut App, st: HistStyle| -> usize {
        app.ws.graphs.get_mut(&w).unwrap().plot.hist_style = st;
        let fig = app.window_figure(w, 460.0, 400.0).unwrap();
        fig.items.iter().find_map(|i| if let crate::figure::Item::Clip { items, .. } = i { items.iter().find_map(|x| if let crate::figure::Item::Poly { pts, fill: None, .. } = x { Some(pts.len()) } else { None }) } else { None }).unwrap()
    };
    let (filled, bars) = (outline(&mut app, HistStyle::Filled), outline(&mut app, HistStyle::Bars));
    assert!(bars > filled, "{bars} vs {filled}");
    // contour options reach the export
    app.session.window_mut(w).unwrap().kind = PlotKind::Contour;
    let lines = |app: &mut App, n: usize| -> usize {
        app.ws.graphs.get_mut(&w).unwrap().plot.contour_levels = n;
        let fig = app.window_figure(w, 460.0, 400.0).unwrap();
        fig.items.iter().find_map(|i| if let crate::figure::Item::Clip { items, .. } = i { items.iter().find_map(|x| if let crate::figure::Item::Image { rgb, .. } = x { Some(rgb.chunks(3).filter(|p| *p != [255, 255, 255]).count()) } else { None }) } else { None }).unwrap()
    };
    assert!(lines(&mut app, 25) > lines(&mut app, 4));
}

#[test]
fn a_layout_tile_carries_its_plot_kind_and_style_into_the_batch_pdf() {
    use crate::density::{HistY, PlotKind, PlotStyle};
    let (mut app, ctx) = editor_app();
    { let to = page_pt(&app, 0.3, 0.45); drag_sample(&mut app, &ctx, "t.fcs", to); }
    settle(&mut app, &ctx);
    let id = app.lay.ed.doc.tiles[0].id;
    {
        let t = app.lay.ed.doc.tile_mut(id).unwrap();
        t.x_chan = "CD3-A".into();
        t.y_chan = "CD4-A".into();
        t.kind = PlotKind::Contour;
        t.style = PlotStyle { contour_levels: 12, contour_color: [250, 20, 20], adjunct: true, ..PlotStyle::default() };
    }
    settle(&mut app, &ctx); // the editor draws the tile and its properties with the style controls
    app.editor_run_batch(Some(&ctx)).unwrap();
    let page = &app.lay.ed.batch[0];
    let clip = page.items.iter().find_map(|i| if let crate::figure::Item::Clip { items, .. } = i { Some(items.clone()) } else { None }).unwrap();
    let rgb = clip.into_iter().find_map(|i| if let crate::figure::Item::Image { rgb, .. } = i { Some(rgb) } else { None }).expect("a contour image");
    assert!(rgb.chunks(3).any(|p| p == [250, 20, 20]), "the contour is drawn in the tile's colour");
    assert!(page.items.iter().filter(|i| matches!(i, crate::figure::Item::Poly { .. })).count() >= 4, "adjunct histograms are on the page");
    // switch the same tile to a histogram with a count axis
    {
        let t = app.lay.ed.doc.tile_mut(id).unwrap();
        t.kind = PlotKind::Histogram;
        t.style.hist_y = HistY::Count;
    }
    app.editor_run_batch(None).unwrap();
    assert!(app.lay.ed.batch[0].texts().contains(&"Count".to_string()));
    // and the style survives a saved workspace
    let json = app.workspace_document().write();
    let mut other = two_sample_app();
    other.apply_workspace_document(&sf_core::json::Json::parse(&json).unwrap()).unwrap();
    assert_eq!(other.lay.ed.doc.tiles[0].style, app.lay.ed.doc.tiles[0].style);
    assert_eq!(other.lay.ed.doc.tiles[0].kind, PlotKind::Histogram);
}

#[test]
fn transpose_and_next_previous_axis_walk_the_channels_and_keep_gates_on_their_own_axes() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Pseudocolor;
    let (x0, y0) = { let win = app.session.window(w).unwrap(); (win.x_chan.clone(), win.y_chan.clone()) };
    let xs0 = app.session.window(w).unwrap().x_scale.clone();
    app.axis_action(w, true, 0);
    { let win = app.session.window(w).unwrap(); assert_eq!((&win.x_chan, &win.y_chan), (&y0, &x0)); }
    app.axis_action(w, true, 0);
    { let win = app.session.window(w).unwrap(); assert_eq!((&win.x_chan, &win.y_chan, &win.x_scale), (&x0, &y0, &xs0), "twice is the identity, scales included"); }
    // next / previous walk the file's own channel order and wrap
    let chans = app.session.sample(0).channels.clone();
    let i = chans.iter().position(|c| *c == x0).unwrap();
    app.axis_action(w, true, 1);
    assert_eq!(app.session.window(w).unwrap().x_chan, chans[(i + 1) % chans.len()]);
    app.axis_action(w, true, -1);
    assert_eq!(app.session.window(w).unwrap().x_chan, x0);
    app.session.window_mut(w).unwrap().x_chan = chans[0].clone();
    app.axis_action(w, true, -1);
    assert_eq!(app.session.window(w).unwrap().x_chan, *chans.last().unwrap(), "wraps backwards");
    app.axis_action(w, false, 1);
    assert_ne!(app.session.window(w).unwrap().y_chan, y0);
    // histograms have one axis
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Histogram;
    let before = app.session.window(w).unwrap().clone();
    app.axis_action(w, true, 0);
    assert!(app.status.contains("one axis"), "{}", app.status);
    app.axis_action(w, false, 1);
    assert!(app.status.contains("no Y"), "{}", app.status);
    let after = app.session.window(w).unwrap();
    assert_eq!((&before.x_chan, &before.y_chan), (&after.x_chan, &after.y_chan));
    // the plot still draws after the changes
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Contour;
    settle(&mut app, &ctx);
    assert!(app.ws.graphs[&w].tex.is_some());
}

#[test]
fn the_export_editor_changes_size_resolution_format_and_content_and_previews_it() {
    use crate::ui::plot_export::{Format, Unit};
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    app.session.window_mut(w).unwrap().y_chan = "CD4-A".into();
    app.refit_window(w);
    settle(&mut app, &ctx);
    app.open_plot_export(w);
    settle(&mut app, &ctx);
    assert!(app.pexp.open == Some(w));
    assert!(app.pexp.preview_ready(), "the dialog drew a preview");
    let mut set = app.pexp.set.clone();
    // PNG: pixel size follows size x dpi, and the file records the dpi
    set.format = Format::Png;
    set.width_pt = 216.0;
    set.height_pt = 144.0;
    set.dpi = 300.0;
    let bytes = app.export_bytes(w, &set, "t").unwrap();
    let dec = png::Decoder::new(std::io::Cursor::new(&bytes));
    let rd = dec.read_info().unwrap();
    assert_eq!((rd.info().width, rd.info().height), (900, 600));
    set.dpi = 150.0;
    let b2 = app.export_bytes(w, &set, "t").unwrap();
    assert_eq!(png::Decoder::new(std::io::Cursor::new(&b2)).read_info().unwrap().info().width, 450);
    // a different unit is the same size
    assert!((Unit::Cm.to_points(7.62, 300.0) - 216.0).abs() < 0.1);
    // vector formats honour the size (points) and are valid
    set.format = Format::Pdf;
    let pdf = app.export_bytes(w, &set, "t").unwrap();
    assert_eq!(crate::figure::validate_pdf(&pdf), Ok(1));
    assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 216 144]"), "page is 216 x 144 pt");
    set.format = Format::Svg;
    let svg = String::from_utf8(app.export_bytes(w, &set, "t").unwrap()).unwrap();
    assert!(svg.contains("width=\"216pt\"") && svg.contains("height=\"144pt\""));
    // content switches: gates, titles, footer, grid and style
    let base_items = app.export_figure(w, &set).unwrap().count();
    let mut no_titles = set.clone();
    no_titles.titles = false;
    no_titles.footer = false;
    let f = app.export_figure(w, &no_titles).unwrap();
    assert!(app.export_figure(w, &set).unwrap().texts().len() > f.texts().len(), "titles and footer are removed");
    let mut no_grid = set.clone();
    no_grid.style.grid = false;
    assert!(app.export_figure(w, &no_grid).unwrap().count() < base_items, "grid lines are removed");
    let mut hist = set.clone();
    hist.style.hist_y = crate::density::HistY::Count;
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Histogram;
    assert!(app.export_figure(w, &hist).unwrap().texts().contains(&"Count".to_string()));
    // a transparent PNG has transparent corners; an opaque one does not
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Pseudocolor;
    set.format = Format::Png;
    set.transparent = true;
    let dec = png::Decoder::new(std::io::Cursor::new(app.export_bytes(w, &set, "t").unwrap()));
    let mut rd = dec.read_info().unwrap();
    let mut buf = vec![0; rd.output_buffer_size().unwrap()];
    rd.next_frame(&mut buf).unwrap();
    assert_eq!(buf[3], 0, "top-left pixel is clear");
    // too large is refused with a reason, never a crash
    set.transparent = false;
    set.width_pt = 2880.0;
    set.height_pt = 2880.0;
    set.dpi = 1200.0;
    assert!(app.export_bytes(w, &set, "t").unwrap_err().contains("too large"));
    // closing
    app.pexp.open = None;
    settle(&mut app, &ctx);
}

fn right_click(app: &mut App, ctx: &egui::Context, pos: Pos2) {
    pointer(app, ctx, vec![egui::Event::PointerMoved(pos)]);
    let ev = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    pointer(app, ctx, vec![ev(true)]);
    pointer(app, ctx, vec![ev(false)]);
    frame(app, ctx);
}

#[test]
fn right_clicking_a_gate_or_the_plot_opens_its_menu_and_every_menu_action_works() {
    use crate::stats_table::Metric;
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
    app.session.window_mut(w).unwrap().tool = Tool::Pointer;
    settle(&mut app, &ctx);
    let _ = map;

    // right-click inside the gate: its menu, with the gate selected
    right_click(&mut app, &ctx, at(0.42, 0.52));
    assert_eq!(app.ws.graphs[&w].menu.map(|m| m.1), Some(Some(kid)), "the gate under the pointer is the target");
    assert_eq!(app.session.selected, Some((0, kid)));
    assert!(app.dlg.plot_menus_shown > 0, "the gate menu was laid out");
    pointer(&mut app, &ctx, vec![key(egui::Key::Escape, false, false)]);
    let shown = app.dlg.plot_menus_shown;
    // right-click on empty plot: the plot's own menu
    right_click(&mut app, &ctx, at(0.9, 0.1));
    assert_eq!(app.ws.graphs[&w].menu.map(|m| m.1), Some(None));
    assert!(app.dlg.plot_menus_shown > shown, "and so was the plot menu");
    pointer(&mut app, &ctx, vec![key(egui::Key::Escape, false, false)]);

    // ---- the actions
    let n0 = app.session.count(0, kid);
    app.gate_to_polygon(0, kid);
    assert!(matches!(gate_of(&app, kid).kind, GateKind::Polygon), "{}", app.status);
    let corners = gate_of(&app, kid).xs.len();
    let g = gate_of(&app, kid);
    app.gate_add_node(0, kid, ((g.xs[0] + g.xs[1]) / 2.0, g.ys[0] - 1.0));
    assert_eq!(gate_of(&app, kid).xs.len(), corners + 1);
    app.gate_to_polygon(0, kid);
    assert!(app.status.contains("already"), "{}", app.status);
    // copy then paste under All Events (and into a gate); the pasted gate counts the same events
    app.gate_copy(0, kid);
    assert!(app.ws.gate_clip.is_some());
    app.gate_paste(0, GateTree::ROOT);
    assert!(app.status.contains("Pasted"), "{}", app.status);
    assert_eq!(children(&app).len(), 2);
    let pasted = *children(&app).last().unwrap();
    assert_eq!(app.session.count(0, pasted), app.session.count(0, kid));
    // exclusion
    app.gate_exclusion(0, pasted);
    assert_eq!(app.session.count(0, pasted) + app.session.count(0, kid), app.session.samples()[0].n_events);
    app.gate_exclusion(0, pasted);
    // colour and statistics on the label
    app.gate_set_color(0, kid, Some([10, 200, 30]));
    assert_eq!(app.ws.gate_colors[&(0, kid)], [10, 200, 30]);
    let ch = app.session.samples()[0].channels[0].clone();
    app.gate_add_stat(0, kid, Metric::Median, &ch);
    app.gate_add_stat(0, kid, Metric::Median, &ch);
    app.gate_add_stat(0, kid, Metric::Count, &ch);
    let lines = app.gate_stat_lines(0, kid);
    assert_eq!(lines.len(), 2, "a repeated statistic is added once: {lines:?}");
    let want = crate::stats_table::node_stat(&mut app.session, 0, kid, Metric::Median, &ch).unwrap();
    assert_eq!(lines[0], format!("Median {ch}: {want:.1}"));
    assert_eq!(lines[1], format!("Count: {}", app.session.count(0, kid)));
    // the number agrees with the Statistics table (one source of maths)
    let t = crate::stats_table::compute(&mut app.session, &[0], &[Metric::Median], Some(&ch));
    let row = t.rows.iter().find(|r| r.population == app.session.tree(0).path(kid).unwrap()).unwrap();
    assert_eq!(row.values[0], Some(want));
    // the gate statistic is recomputed after an edit (cache keyed on the sample's version)
    let g = gate_of(&app, kid);
    let mut moved = g.clone();
    moved.xs.iter_mut().for_each(|v| *v = *v * 0.4 + (range.x0 + range.x1) * 0.05);
    moved.ys.iter_mut().for_each(|v| *v = *v * 0.5);
    app.session.edit_gate(0, kid, moved).unwrap();
    let again = app.gate_stat_lines(0, kid);
    assert_eq!(again[1], format!("Count: {}", app.session.count(0, kid)), "the label follows the gate, not a stale cache");
    app.gate_remove_stat(0, kid, 0);
    assert_eq!(app.gate_stat_lines(0, kid).len(), 1);
    // FCS export of a population reads back with the same events
    let bytes = app.population_fcs_bytes(0, kid).unwrap();
    let f = sf_fcs::Fcs::parse(&bytes).unwrap();
    assert_eq!(f.n_events, app.session.count(0, kid));
    assert_eq!(f.n_params(), app.session.samples()[0].channels.len());
    // apply to all, delete, undo
    app.gate_apply_all(0, kid);
    app.gate_delete(0, pasted);
    assert!(!app.session.tree(0).is_alive(pasted));
    assert!(app.session.undo(0));
    assert!(app.session.tree(0).is_alive(pasted));
    let _ = n0;
    // every dialog draws without trouble
    app.dlg.color = Some((0, kid, [1, 2, 3]));
    app.dlg.stat = Some((0, kid, 3, ch.clone()));
    app.dlg.subsample = Some((0, kid, 10));
    app.dlg.keywords = Some(0);
    app.dlg.history = Some((0, kid));
    settle(&mut app, &ctx);
    app.dlg.subsample = Some((0, GateTree::ROOT, 50));
    settle(&mut app, &ctx);
    assert!(app.dlg.color.is_some() && app.dlg.history.is_some());
}

#[test]
fn layout_pages_save_as_png_at_the_chosen_resolution() {
    let (mut app, ctx) = editor_app();
    { let to = page_pt(&app, 0.3, 0.45); drag_sample(&mut app, &ctx, "t.fcs", to); }
    settle(&mut app, &ctx);
    app.editor_run_batch(Some(&ctx)).unwrap();
    app.pexp.set.dpi = 100.0;
    let page = app.lay.ed.batch[0].clone();
    let p = scratch("page.png");
    let msg = app.save_page_png(&page, &p).unwrap();
    let (w, h) = crate::raster::pixel_size(&page, 100.0);
    assert!(msg.contains(&format!("{w} x {h}")), "{msg}");
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&p).unwrap()));
    let info = dec.read_info().unwrap();
    assert_eq!((info.info().width, info.info().height), (w, h));
    app.pexp.set.dpi = 200.0;
    app.save_page_png(&page, &p).unwrap();
    let info = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&p).unwrap())).read_info().unwrap();
    assert_eq!(info.info().width, w * 2);
    // the screens that hold the buttons still draw with pages built
    settle(&mut app, &ctx);
}

fn three_sample_app() -> App {
    let mut app = App::new();
    for i in 0..3 {
        let mut rng = Rng::new(40 + i as u64);
        let events: Vec<f32> = (0..3000).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 200_000.0) as f32]).collect();
        app.session.add_sample(LoadedSample::synthetic(&format!("s{i}.fcs"), &["FSC-A", "SSC-A"], vec![262_144.0, 262_144.0], events));
    }
    app
}

#[test]
fn a_gate_drawn_in_one_sample_appears_in_all_and_editing_it_shows_an_edited_signal() {
    let ctx = egui::Context::default();
    let mut app = three_sample_app();
    assert_eq!(app.session.draw_target, crate::session::DrawTarget::app_default(), "the app shares new gates with every sample");
    let w = app.session.open_population(0, GateTree::ROOT).unwrap();
    app.open_tab(0, GateTree::ROOT).unwrap();
    settle(&mut app, &ctx);
    let g = app.session.commit_shape(w, &Shape::Rect { x0: 20_000.0, x1: 120_000.0, y0: 20_000.0, y1: 120_000.0 }, "Lymph").unwrap();
    for s in 0..3 {
        assert!(app.session.tree(s).find(&["Lymph"]).is_some(), "sample {s} has the gate");
    }
    assert_eq!(app.session.gate_owner(0, g).unwrap().group, "All Samples");
    // editing in sample 2 shows as edited there only
    let n2 = app.session.tree(2).find(&["Lymph"]).unwrap();
    let mut moved = gate_of_sample(&app, 2, n2);
    moved.xs.iter_mut().for_each(|v| *v += 10_000.0);
    app.session.edit_gate(2, n2, moved).unwrap();
    assert!(app.session.gate_owner(2, n2).unwrap().edited);
    assert!(!app.session.gate_owner(0, g).unwrap().edited);
    // hop to sample 2 and draw: the plot labels the gate as edited (a frame with the label must render)
    app.session.retarget_window(w, 2).unwrap();
    app.refit_window(w);
    settle(&mut app, &ctx);
    // the right-click actions on an edited gate
    app.status.clear();
    app.gate_apply_group(2, n2, "All Samples");
    assert!(!app.session.gate_owner(2, n2).unwrap().edited, "updating the group from sample 2 clears the signal");
    let n0 = app.session.tree(0).find(&["Lymph"]).unwrap();
    assert_eq!(gate_of_sample(&app, 0, n0), gate_of_sample(&app, 2, n2), "and every sample has the edited version");
    // a plain delete of a group-owned gate asks first
    app.gate_delete(1, app.session.tree(1).find(&["Lymph"]).unwrap());
    assert!(app.dlg.group_delete.is_some());
    assert!(app.session.tree(1).find(&["Lymph"]).is_some(), "nothing was deleted yet");
    settle(&mut app, &ctx);
    app.dlg.group_delete = None;
    // groups: the manager draws, and the current group limits which samples the arrows step through
    app.session.create_group("Pair", &[0, 2]).unwrap();
    app.dlg.mgr.open = true;
    app.dlg.mgr.load(&app.session, "Pair");
    settle(&mut app, &ctx);
    app.dlg.mgr.load(&app.session, "All Samples");
    settle(&mut app, &ctx);
    app.ws.cur_group = Some("Pair".into());
    assert_eq!(app.step_sample(0, 1), Some(2), "sample 1 is not in the group");
    assert_eq!(app.step_sample(2, -1), Some(0));
    assert_eq!(app.step_sample(2, 1), None);
    app.ws.cur_group = None;
    assert_eq!(app.step_sample(0, 1), Some(1));
    // new gates can go to a group, and the toolbar combo draws with the groups in it
    app.session.draw_target = crate::session::DrawTarget::Group("Pair".into());
    app.session.retarget_window(w, 0).unwrap();
    let h = app.session.commit_shape(w, &Shape::Rect { x0: 1.0, x1: 50_000.0, y0: 1.0, y1: 50_000.0 }, "OnlyPair").unwrap();
    assert!(app.session.tree(2).find(&["OnlyPair"]).is_some() && app.session.tree(1).find(&["OnlyPair"]).is_none());
    assert_eq!(app.session.gate_owner(0, h).unwrap().group, "Pair");
    settle(&mut app, &ctx);
    // key arrows follow the current group
    app.ws.cur_group = Some("Pair".into());
    pointer(&mut app, &ctx, vec![key(egui::Key::ArrowRight, false, false)]);
    assert_eq!(app.session.window(w).unwrap().sample, 2, "→ skipped the sample outside the group");
}

fn gate_of_sample(app: &App, s: usize, node: usize) -> Gate {
    match app.session.tree(s).kind(node).unwrap() {
        NodeKind::Gate(g) => g.clone(),
        k => panic!("{k:?}"),
    }
}

#[test]
fn a_group_exports_equivalent_populations_as_fcs_and_csv_alone_or_concatenated() {
    use crate::ui::group_export::{DataFormat, TableFormat};
    let ctx = egui::Context::default();
    let mut app = three_sample_app();
    let w = app.session.open_population(0, GateTree::ROOT).unwrap();
    app.open_tab(0, GateTree::ROOT).unwrap();
    settle(&mut app, &ctx);
    app.session.commit_shape(w, &Shape::Rect { x0: 20_000.0, x1: 120_000.0, y0: 20_000.0, y1: 120_000.0 }, "Lymph").unwrap();
    app.session.create_group("Pair", &[0, 2]).unwrap();
    let pop = vec!["Lymph".to_string()];
    assert_eq!(app.common_populations("Pair"), vec![Vec::<String>::new(), pop.clone()]);
    let count = |app: &mut App, s: usize| {
        let n = app.session.tree(s).find(&["Lymph"]).unwrap();
        app.session.count(s, n)
    };
    let (c0, c2) = (count(&mut app, 0), count(&mut app, 2));
    assert!(c0 > 100 && c2 > 100);
    // one FCS per member, only the group's samples, each with that sample's events of the population
    let (files, notes) = app.group_data_files("Pair", &pop, DataFormat::Fcs, false).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(files.iter().map(|f| f.0.clone()).collect::<Vec<_>>(), vec!["s0_Lymph.fcs", "s2_Lymph.fcs"]);
    let f0 = sf_fcs::Fcs::parse(&files[0].1).unwrap();
    assert_eq!((f0.n_events, f0.n_params()), (c0, 2));
    // every exported event satisfies the gate
    for r in 0..f0.n_events {
        let (x, y) = (f0.events[r * 2], f0.events[r * 2 + 1]);
        assert!((20_000.0..=120_000.0).contains(&x) && (20_000.0..=120_000.0).contains(&y), "event {r} = ({x}, {y})");
    }
    // channel CSV is the raw numbers, scale CSV is the transformed ones
    let (csv, _) = app.group_data_files("Pair", &pop, DataFormat::CsvChannel, false).unwrap();
    let text = String::from_utf8(csv[0].1.clone()).unwrap();
    let mut lines = text.lines();
    assert_eq!(lines.next().unwrap(), "FSC-A,SSC-A");
    let first: Vec<f64> = lines.next().unwrap().split(',').map(|v| v.parse().unwrap()).collect();
    assert_eq!(first, vec![f0.events[0] as f64, f0.events[1] as f64]);
    assert_eq!(text.lines().count(), c0 + 1);
    let (scaled, _) = app.group_data_files("Pair", &pop, DataFormat::CsvScale, false).unwrap();
    let st = String::from_utf8(scaled[0].1.clone()).unwrap();
    let sfirst: Vec<f64> = st.lines().nth(1).unwrap().split(',').map(|v| v.parse().unwrap()).collect();
    assert_eq!(sfirst[0], first[0], "scatter is linear, so scale = channel");
    // concatenated: one file, a SampleID column that says which sample each event came from
    let (cat, cnotes) = app.group_data_files("Pair", &pop, DataFormat::Fcs, true).unwrap();
    assert_eq!(cat.len(), 1);
    assert!(cnotes.iter().any(|n| n.contains("SampleID") && n.contains("1 = s0.fcs") && n.contains("2 = s2.fcs")), "{cnotes:?}");
    let fc = sf_fcs::Fcs::parse(&cat[0].1).unwrap();
    assert_eq!((fc.n_events, fc.n_params()), (c0 + c2, 3));
    assert_eq!(fc.params[2].name, "SampleID");
    let ids: Vec<f32> = (0..fc.n_events).map(|r| fc.events[r * 3 + 2]).collect();
    assert_eq!(ids.iter().filter(|&&v| v == 1.0).count(), c0);
    assert_eq!(ids.iter().filter(|&&v| v == 2.0).count(), c2);
    // a population one member lacks is skipped and said so; none at all is an error
    app.session.create_group("Mixed", &[0, 1]).unwrap();
    let n1 = app.session.tree(1).find(&["Lymph"]).unwrap();
    app.session.release_gate(1, n1).ok();
    app.session.delete(1, n1).unwrap();
    let (some, snotes) = app.group_data_files("Mixed", &pop, DataFormat::Fcs, false).unwrap();
    assert_eq!(some.len(), 1);
    assert!(snotes.iter().any(|n| n.contains("s1.fcs") && n.contains("skipped")), "{snotes:?}");
    assert!(app.group_data_files("Pair", &["Nope".to_string()], DataFormat::Fcs, false).is_err());
    // the group's statistics table: only its samples, as CSV, Excel and PDF
    let csv = String::from_utf8(app.group_table_bytes("Pair", TableFormat::Csv).unwrap()).unwrap();
    assert!(csv.contains("s0.fcs") && csv.contains("s2.fcs") && !csv.contains("s1.fcs"));
    let xlsx = app.group_table_bytes("Pair", TableFormat::Excel).unwrap();
    assert_eq!(&xlsx[..2], b"PK");
    let pdf = app.group_table_bytes("Pair", TableFormat::Pdf).unwrap();
    assert!(crate::figure::validate_pdf(&pdf).is_ok());
    // the window draws, and opening the layout selects exactly the group
    app.gx.open = true;
    app.gx.group = "Pair".into();
    settle(&mut app, &ctx);
    app.use_group_in_layout("Pair");
    assert_eq!(app.lay.sel, vec![true, false, true]);
}

#[test]
fn layout_pages_become_a_powerpoint_deck_with_one_slide_each() {
    let (mut app, ctx) = editor_app();
    { let to = page_pt(&app, 0.3, 0.45); drag_sample(&mut app, &ctx, "t.fcs", to); }
    settle(&mut app, &ctx);
    app.editor_run_batch(Some(&ctx)).unwrap();
    app.pexp.set.dpi = 72.0;
    let pages = app.lay.ed.batch.clone();
    let deck = app.pages_pptx(&pages, "Layout").unwrap();
    assert_eq!(&deck[..2], b"PK");
    let text = String::from_utf8_lossy(&deck);
    for i in 1..=pages.len() {
        assert!(text.contains(&format!("ppt/slides/slide{i}.xml")) && text.contains(&format!("ppt/media/image{i}.png")));
    }
    assert!(text.contains(&format!("page 1 of {}", pages.len())));
    assert!(app.pages_pptx(&[], "x").is_err());
}

// ---- layout stability, activity cue, export preference, unmixed channels, axis labels -----------------------

#[test]
fn the_plot_keeps_its_size_when_side_panes_open_and_close() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot) = ready_tab(&mut app, &ctx);
    assert!(!app.ws.options_open, "the options pane starts closed");
    let size = (plot.width().round(), plot.height().round());
    for (options, pops) in [(true, true), (true, false), (false, false), (false, true)] {
        app.ws.options_open = options;
        app.ws.pops_open = pops;
        settle(&mut app, &ctx);
        settle(&mut app, &ctx);
        let p = app.ws.graphs[&w].plot_rect.unwrap();
        assert_eq!((p.width().round(), p.height().round()), size, "options {options}, populations {pops}");
    }
}

#[test]
fn the_scatter_gate_of_a_control_is_drawn_moved_and_resized_without_losing_it() {
    let ctx = egui::Context::default();
    let size = egui::vec2(300.0, 260.0);
    let pts: Vec<(f32, f32, bool)> = (0..200).map(|i| (i as f32 * 0.5, 50.0, true)).collect();
    let range = [(0.0f32, 100.0f32), (0.0f32, 100.0f32)];
    let mut gate: Option<[f64; 4]> = None;
    let origin = std::cell::Cell::new(None::<Pos2>);
    let mut t = 0.0;
    let mut run = |gate: &mut Option<[f64; 4]>, events: Vec<egui::Event>| {
        t += 0.05;
        let input = egui::RawInput { screen_rect: Some(screen()), time: Some(t), events, ..Default::default() };
        let _ = ctx.run(input, |c| {
            egui::CentralPanel::default().show(c, |ui| {
                origin.set(Some(origin.get().unwrap_or_else(|| ui.next_widget_position())));
                comp_wizard_view::scatter_editor(ui, &pts, range, size, "x", "y", Some(gate));
            });
        });
    };
    run(&mut gate, vec![]);
    let o = origin.get().unwrap();
    // data (0..100) -> screen: the plot is the widget minus 6 px left/top/right and 16 px bottom
    let scr = |x: f64, y: f64| Pos2::new(o.x + 6.0 + (x / 100.0) as f32 * (size.x - 12.0), o.y + 6.0 + (1.0 - y / 100.0) as f32 * (size.y - 22.0));
    let drag = |run: &mut dyn FnMut(&mut Option<[f64; 4]>, Vec<egui::Event>), gate: &mut Option<[f64; 4]>, a: Pos2, b: Pos2| {
        run(gate, vec![egui::Event::PointerMoved(a)]);
        run(gate, vec![button(a, true)]);
        run(gate, vec![egui::Event::PointerMoved(Pos2::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0))]);
        run(gate, vec![egui::Event::PointerMoved(b)]);
        run(gate, vec![button(b, false)]);
        run(gate, vec![]);
    };
    // draw
    drag(&mut run, &mut gate, scr(20.0, 30.0), scr(60.0, 70.0));
    let g = gate.expect("a gate was drawn");
    assert!((g[0] - 20.0).abs() < 1.5 && (g[1] - 60.0).abs() < 1.5 && (g[2] - 30.0).abs() < 1.5 && (g[3] - 70.0).abs() < 1.5, "{g:?}");
    // move: dragging inside keeps the size and shifts the gate (the old one does not disappear)
    drag(&mut run, &mut gate, scr(40.0, 50.0), scr(50.0, 40.0));
    let m = gate.unwrap();
    assert!(((m[1] - m[0]) - (g[1] - g[0])).abs() < 0.5 && ((m[3] - m[2]) - (g[3] - g[2])).abs() < 0.5, "size kept: {m:?}");
    assert!((m[0] - g[0] - 10.0).abs() < 1.5 && (m[2] - g[2] + 10.0).abs() < 1.5, "moved by (+10, -10): {m:?} from {g:?}");
    // resize by the right edge: only that edge moves
    drag(&mut run, &mut gate, scr(m[1], (m[2] + m[3]) / 2.0), scr(m[1] - 15.0, (m[2] + m[3]) / 2.0));
    let r = gate.unwrap();
    assert!((r[1] - (m[1] - 15.0)).abs() < 1.5 && (r[0] - m[0]).abs() < 0.5 && (r[2] - m[2]).abs() < 0.5 && (r[3] - m[3]).abs() < 0.5, "{r:?} from {m:?}");
    // a corner moves two edges
    drag(&mut run, &mut gate, scr(r[0], r[3]), scr(r[0] + 5.0, r[3] + 8.0));
    let c = gate.unwrap();
    assert!((c[0] - (r[0] + 5.0)).abs() < 1.5 && (c[3] - (r[3] + 8.0)).abs() < 1.5 && (c[1] - r[1]).abs() < 0.5, "{c:?} from {r:?}");
    // empty space draws a new one
    drag(&mut run, &mut gate, scr(85.0, 10.0), scr(95.0, 20.0));
    let n = gate.unwrap();
    assert!(n[0] > 80.0 && n[1] < 99.0 && n[2] < 25.0, "{n:?}");
}

#[test]
fn a_graph_can_be_detached_into_its_own_window_and_docked_again() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    app.ws.detached.insert(w);
    settle(&mut app, &ctx);
    settle(&mut app, &ctx);
    let floating = app.ws.graphs[&w].plot_rect;
    assert!(floating.map_or(false, |r| r.width() > 50.0), "the floating window draws the plot");
    app.ws.detached.remove(&w);
    settle(&mut app, &ctx);
    settle(&mut app, &ctx);
    assert!(app.ws.graphs[&w].plot_rect.map_or(false, |r| r.width() > 50.0), "docked again");
    // a window that no longer exists drops out of the detached set
    app.ws.detached.insert(9_999);
    settle(&mut app, &ctx);
    assert!(!app.ws.detached.contains(&9_999));
}

#[test]
fn slow_work_shows_its_cue_for_two_frames_before_it_runs() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    settle(&mut app, &ctx);
    let ran = std::rc::Rc::new(std::cell::Cell::new(false));
    let flag = ran.clone();
    app.run_busy("Doing the thing…", move |_| flag.set(true));
    assert!(app.is_busy());
    frame(&mut app, &ctx);
    assert!(!ran.get() && app.act.shown.as_deref() == Some("Doing the thing…"), "the cue is on screen before the work starts");
    frame(&mut app, &ctx);
    assert!(!ran.get());
    frame(&mut app, &ctx);
    assert!(ran.get(), "the work ran");
    frame(&mut app, &ctx);
    assert!(!app.is_busy() && app.act.shown.is_none(), "and the cue is gone");
}

#[test]
fn the_export_preference_is_saved_reloaded_and_used_by_quick_exports() {
    let path = std::env::temp_dir().join(format!("sf_export_prefs_{}.json", std::process::id()));
    std::env::set_var("STREAMFLOW_EXPORT_PREFS", &path);
    let _ = std::fs::remove_file(&path);
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    // nothing saved: a quick export uses the factory settings with the plot own style
    assert!(app.pexp.defaults.is_none());
    let factory = app.quick_export_settings(w);
    assert_eq!((factory.dpi, factory.width_pt), (300.0, 360.0));
    let mut set = factory.clone();
    set.dpi = 150.0;
    set.width_pt = 216.0;
    set.height_pt = 144.0;
    set.unit = plot_export::Unit::Mm;
    set.transparent = true;
    set.gates = false;
    set.style.grid = true;
    set.style.pseudo_levels = 32;
    app.save_export_prefs(&set).unwrap();
    // a new session reads it back, and every quick export now follows it
    let mut app2 = app_with_sample();
    app2.load_export_prefs();
    assert_eq!(app2.pexp.defaults.as_ref(), Some(&set));
    let (w2, _) = ready_tab(&mut app2, &ctx);
    assert_eq!(app2.quick_export_settings(w2), set);
    let png = app2.export_bytes(w2, &plot_export::Settings { format: plot_export::Format::Png, ..app2.quick_export_settings(w2) }, "t").unwrap();
    let (pw, ph) = set.pixels();
    assert_eq!(&png[1..4], b"PNG");
    assert_eq!((u32::from_be_bytes(png[16..20].try_into().unwrap()), u32::from_be_bytes(png[20..24].try_into().unwrap())), (pw, ph), "the picture has the saved size and resolution");
    // the editor opens on the saved preference
    app2.open_plot_export(w2);
    assert_eq!(app2.pexp.set, set);
    // a damaged file is ignored with a message, not a crash
    std::fs::write(&path, "{ not json").unwrap();
    let mut app3 = app_with_sample();
    app3.load_export_prefs();
    assert!(app3.pexp.defaults.is_none() && app3.status.contains("Ignored"));
    app2.reset_export_prefs();
    assert!(app2.pexp.defaults.is_none() && !path.exists());
    std::env::remove_var("STREAMFLOW_EXPORT_PREFS");
}

#[test]
fn unmixed_channels_open_on_a_plot_and_lead_the_axis_lists() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let n = app.session.sample(0).n_events;
    app.session.set_unmixed(0, vec!["FITC".into(), "PE".into(), "Residual".into()], vec![vec![1.0; n], vec![2.0; n], vec![0.5; n]], 262_144.0).unwrap();
    app.open_unmixed_plot(0, true).unwrap();
    settle(&mut app, &ctx);
    let w = app.session.windows.last().unwrap();
    assert_eq!((w.x_chan.as_str(), w.y_chan.as_str()), ("Unmix-FITC", "Unmix-PE"));
    assert_eq!(app.module(), Module::Workstation);
    assert!(matches!(app.ws.tab, Tab::Graph(id) if id == w.id));
    // a sample with fewer than two unmixed markers says so
    app.session.clear_unmixed(0);
    assert!(app.open_unmixed_plot(0, true).is_err());
}

#[test]
fn crowded_axis_labels_are_thinned_so_none_overlap() {
    let ctx = egui::Context::default();
    let font = egui::FontId::proportional(10.5);
    let mut kept_total = 0;
    let _ = ctx.run(egui::RawInput { screen_rect: Some(screen()), ..Default::default() }, |c| {
        let p = c.layer_painter(egui::LayerId::background());
        // twelve labels 6 points apart cannot all fit
        let items: Vec<(f32, String)> = (0..12).map(|i| (100.0 + i as f32 * 6.0, if i == 3 { "0".to_string() } else { format!("{}k", i * 10) })).collect();
        let keep = graph::declutter(&p, &items, &font, true, Some(3), 4.0);
        assert!(keep[3], "the zero label is always kept");
        let widths: Vec<f32> = items.iter().map(|(_, t)| p.layout_no_wrap(t.clone(), font.clone(), egui::Color32::BLACK).size().x).collect();
        let kept: Vec<usize> = (0..12).filter(|&i| keep[i]).collect();
        for a in &kept {
            for b in &kept {
                if a < b {
                    assert!((items[*a].0 - items[*b].0).abs() >= (widths[*a] + widths[*b]) / 2.0, "labels {a} and {b} overlap");
                }
            }
        }
        kept_total = kept.len();
        // well-spaced labels are all kept, and minor ticks (no text) never block
        let spread: Vec<(f32, String)> = (0..5).map(|i| (i as f32 * 80.0, format!("{i}"))).chain([(40.0, String::new())]).collect();
        assert_eq!(graph::declutter(&p, &spread, &font, true, None, 4.0), vec![true, true, true, true, true, false]);
    });
    assert!(kept_total >= 2 && kept_total < 12, "kept {kept_total}");
}

#[test]
fn the_compensation_wizard_calculates_from_real_controls_and_its_windows_draw() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
    if !dir.join("fitc alone.fcs").exists() {
        eprintln!("SKIPPED: fixtures missing");
        return;
    }
    let ctx = egui::Context::default();
    let mut app = App::new();
    let files: Vec<std::path::PathBuf> = ["fitc alone", "pe alone", "apc alone", "AF700 alone", "bv421 alone", "bv711 alone", "unstained"].iter().map(|n| dir.join(format!("{n}.fcs"))).collect();
    app.load_files(&files);
    app.set_module(Module::Compensation);
    settle(&mut app, &ctx);
    app.open_comp_wizard();
    settle(&mut app, &ctx);
    assert_eq!(app.comp.wiz.rows.iter().filter(|r| r.detector.is_some()).count(), 6, "six controls recognised from the file names");
    assert_eq!(app.comp.wiz.unstained, Some(6));
    // the classic median method needs a clear positive population: this APC control has only a thin bright tail, and
    // the wizard says so instead of guessing
    app.comp.wiz.opt.method = crate::comp_wizard::Method::Medians;
    app.wizard_calculate();
    let e = app.comp.wiz.result.clone().unwrap().unwrap_err();
    assert!(e.contains("apc alone") && e.contains("positive"), "{e}");
    let mut got = Vec::new();
    for method in [crate::comp_wizard::Method::Slope, crate::comp_wizard::Method::Refined] {
        app.comp.wiz.opt.method = method;
        app.wizard_calculate();
        let r = app.comp.wiz.result.clone().unwrap().unwrap_or_else(|e| panic!("{method:?}: {e}"));
        assert_eq!(r.matrix.detectors.len(), 6);
        assert!(r.condition.is_finite() && r.condition < 30.0, "{method:?}: condition {}", r.condition);
        assert!(r.reports.iter().all(|c| !c.badges.iter().any(|b| b.blocks())), "{method:?}: {:?}", r.reports.iter().map(|c| &c.badges).collect::<Vec<_>>());
        got.push(r.matrix.spill.clone());
    }
    // the methods are different estimators but must agree on a clean data set
    let worst = |a: &Vec<f64>, b: &Vec<f64>| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0, f64::max);
    eprintln!("slope vs refined: {:.4}", worst(&got[0], &got[1]));
    assert!(worst(&got[0], &got[1]) < 0.25);
    app.comp.wiz.opt.method = crate::comp_wizard::Method::Refined;
    // the windows draw: results tabs, an opened control with a hand-set limit, and the matrix goes to the editor
    for tab in [comp_wizard_view::Tab::Summary, comp_wizard_view::Tab::Before, comp_wizard_view::Tab::Spread, comp_wizard_view::Tab::Cosine] {
        app.comp.wiz.show = tab;
        settle(&mut app, &ctx);
    }
    // the two pages and the maximised window draw, and the setup page has the controls' histograms and scatter gate
    for results in [true, false, true] {
        app.demo_comp_wizard_page(results);
        settle(&mut app, &ctx);
    }
    assert!(app.comp.wiz.maximized);
    app.comp.wiz.page = comp_wizard_view::Page::Setup;
    app.comp.wiz.inspect = Some(1);
    settle(&mut app, &ctx);
    app.comp.wiz.rows[1].manual = Some((300.0, 3000.0));
    app.wizard_calculate();
    settle(&mut app, &ctx);
    let r = app.comp.wiz.result.clone().unwrap().unwrap();
    let c = r.reports.iter().find(|c| c.sample == 1).unwrap();
    assert_eq!((c.neg_max, c.pos_min), (300.0, 3000.0));
    app.comp.use_matrix(r.matrix.clone(), r.notes.clone());
    assert_eq!(app.comp.matrix_for_test().unwrap().detectors, r.matrix.detectors);
    // no controls assigned: a clear message, not a crash
    app.comp.wiz.rows.iter_mut().for_each(|r| r.detector = None);
    app.wizard_calculate();
    assert!(app.comp.wiz.result.as_ref().unwrap().is_err());
}

#[test]
fn the_compensation_wizard_fits_the_screen_on_both_pages_maximised_or_not() {
    let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
    if !dir.join("apc alone.fcs").exists() {
        eprintln!("SKIPPED: fixtures missing");
        return;
    }
    let ctx = egui::Context::default();
    let mut app = App::new();
    let files: Vec<std::path::PathBuf> = ["apc alone", "bv421 alone", "fitc alone", "pe alone", "AF700 alone", "bv711 alone", "unstained"].iter().map(|n| dir.join(format!("{n}.fcs"))).collect();
    app.load_files(&files);
    app.set_module(Module::Compensation);
    app.open_comp_wizard();
    app.wizard_calculate();
    for (maximised, results) in [(true, false), (true, true), (false, false), (false, true)] {
        app.comp.wiz.maximized = maximised;
        app.comp.wiz.page = if results { comp_wizard_view::Page::Results } else { comp_wizard_view::Page::Setup };
        app.comp.wiz.request_resize();
        for _ in 0..4 {
            frame(&mut app, &ctx);
        }
        let r = ctx.memory(|m| m.area_rect(egui::Id::new("Compensation wizard"))).unwrap();
        assert!(r.left() >= -0.5 && r.right() <= screen().right() + 0.5 && r.bottom() <= screen().bottom() + 0.5, "maximised {maximised}, results {results}: {r:?} outside {:?}", screen());
        if maximised {
            assert!(r.width() > screen().width() - 40.0 && r.height() > screen().height() - 120.0, "a maximised wizard fills the window: {r:?}");
        }
    }
}

/// Type into whatever holds the keyboard: select everything, then the text.
fn type_over(app: &mut App, ctx: &egui::Context, text: &str) {
    pointer(app, ctx, vec![key(egui::Key::A, true, false)]);
    pointer(app, ctx, vec![egui::Event::Text(text.to_string())]);
}

#[test]
fn renaming_a_population_commits_on_enter_and_on_clicking_away_and_escape_cancels() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    let g = app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 5e4 }, "Population 1").unwrap();
    settle(&mut app, &ctx);

    // Enter keeps the new name and closes the box
    app.start_rename(0, g);
    settle(&mut app, &ctx);
    assert!(app.ws.rename.is_some() && app.ws.rename_focused, "the box is open and holds the keyboard");
    type_over(&mut app, &ctx, "Live");
    assert_eq!(app.ws.rename.as_ref().unwrap().2, "Live", "typing reaches the box");
    pointer(&mut app, &ctx, vec![key(egui::Key::Enter, false, false)]);
    settle(&mut app, &ctx);
    assert!(app.ws.rename.is_none(), "Enter closes the box");
    assert_eq!(app.session.tree(0).name(g).unwrap(), "Live");
    assert!(app.status.contains("Renamed"), "{}", app.status);

    // Escape drops what was typed
    app.start_rename(0, g);
    settle(&mut app, &ctx);
    type_over(&mut app, &ctx, "Dead");
    pointer(&mut app, &ctx, vec![key(egui::Key::Escape, false, false)]);
    settle(&mut app, &ctx);
    assert!(app.ws.rename.is_none());
    assert_eq!(app.session.tree(0).name(g).unwrap(), "Live");

    // clicking somewhere else keeps it
    app.start_rename(0, g);
    settle(&mut app, &ctx);
    type_over(&mut app, &ctx, "Lymphocytes");
    let away = Pos2::new(1400.0, 860.0);
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(away)]);
    pointer(&mut app, &ctx, vec![button(away, true)]);
    pointer(&mut app, &ctx, vec![button(away, false)]);
    settle(&mut app, &ctx);
    assert!(app.ws.rename.is_none(), "the box closes when the keyboard leaves it");
    assert_eq!(app.session.tree(0).name(g).unwrap(), "Lymphocytes");
    // the rename is one undo step
    assert!(app.session.undo(0));
    assert_eq!(app.session.tree(0).name(g).unwrap(), "Live");
}

#[test]
fn a_gates_name_on_the_plot_takes_the_right_click_and_a_double_click_opens_the_gate() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    let g = app.session.commit_shape(w, &Shape::Rect { x0: 4e4, x1: 1e5, y0: 2e4, y1: 5e4 }, "Population 1").unwrap();
    settle(&mut app, &ctx);
    let (node, rect) = *app.ws.graphs[&w].label_rects.last().expect("the gate's name was drawn");
    assert_eq!(node, g);
    let at = rect.center();
    assert_eq!(app.gate_label_at(w, at), Some(g));
    // the name sits outside the outline, so the gate itself is not under this point
    let secondary = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    pointer(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
    pointer(&mut app, &ctx, vec![secondary(true)]);
    pointer(&mut app, &ctx, vec![secondary(false)]);
    settle(&mut app, &ctx);
    assert_eq!(app.ws.graphs[&w].menu.map(|m| m.1), Some(Some(g)), "the menu is the gate's menu (with Rename), not the plot's");
    pointer(&mut app, &ctx, vec![key(egui::Key::Escape, false, false)]);
    settle(&mut app, &ctx);

    for _ in 0..2 {
        pointer(&mut app, &ctx, vec![button(at, true)]);
        pointer(&mut app, &ctx, vec![button(at, false)]);
    }
    settle(&mut app, &ctx);
    assert!(app.ws.rename.is_none(), "double-clicking the name does not rename");
    assert!(app.session.windows.iter().any(|x| x.sample == 0 && x.node == g), "it opens the population, like double-clicking the gate itself");
}

#[test]
fn channel_markers_label_axes_and_exports_and_are_saved_with_the_workspace() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    assert_eq!(app.session.channel_label("CD3-A"), "CD3-A", "no marker: just the channel");
    app.session.set_marker("CD3-A", Some(" CD45 "));
    assert_eq!(app.session.marker("CD3-A").as_deref(), Some("CD45"));
    assert_eq!(app.session.channel_label("CD3-A"), "CD3-A :: CD45");
    assert_eq!(app.session.channel_label("Comp-CD3-A"), "Comp-CD3-A :: CD45", "a compensated channel shares its detector's marker");
    // the exported figure's axis title carries it
    let (w, _) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().x_chan = "CD3-A".into();
    settle(&mut app, &ctx);
    let fig = app.window_figure(w, 300.0, 300.0).unwrap();
    assert!(crate::figure::to_svg(&fig).contains("CD3-A :: CD45"));
    // the window with the table opens and draws
    app.ws.markers_open = true;
    settle(&mut app, &ctx);
    // saved and restored; an emptied name hides the label, a reset brings the file's back
    let doc = app.session.to_workspace_json();
    let mut other = app_with_sample();
    other.session.apply_workspace_json(&doc).unwrap();
    assert_eq!(other.session.channel_label("CD3-A"), "CD3-A :: CD45");
    other.session.set_marker("CD3-A", Some(""));
    assert_eq!(other.session.channel_label("CD3-A"), "CD3-A");
    other.session.set_marker("CD3-A", None);
    assert!(other.session.marker_override("CD3-A").is_none());
}

#[test]
fn a_histogram_keeps_the_y_axis_column_so_the_plot_does_not_move() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot_2d) = ready_tab(&mut app, &ctx);
    app.session.window_mut(w).unwrap().kind = crate::density::PlotKind::Histogram;
    app.ws.graphs.get_mut(&w).unwrap().refit = true;
    settle(&mut app, &ctx);
    let plot_h = app.ws.graphs[&w].plot_rect.unwrap();
    assert_eq!(plot_h.left(), plot_2d.left(), "the Y axis column (now saying Histogram) is still there");
    assert_eq!(app.ws.graphs[&w].last_2d, Some(crate::density::PlotKind::Pseudocolor), "the plot type to go back to is remembered");
}

#[test]
fn characters_the_export_font_lacks_are_replaced_like_the_pdf_does() {
    assert_eq!(crate::figure::ascii_fallback('▸'), Some('>'));
    assert_eq!(crate::figure::ascii_fallback('→'), Some('>'));
    assert_eq!(crate::figure::ascii_fallback('A'), None);
    assert_eq!(crate::figure::ascii_fallback('é'), None, "a character WinAnsi has is kept");
}

#[test]
fn a_thirty_colour_matrix_fits_the_window_and_small_cells_stay_readable() {
    use super::comp_wizard_view::{matrix_cell, matrix_table, matrix_value};
    // 6 colours: comfortable but capped (cells used to grow to 120 x 44 and push the table off the page)
    let (w6, h6, _) = matrix_cell(6, 1400.0, 120.0, 1.0);
    assert!(w6 <= 72.0 && h6 <= 40.0, "{w6} x {h6}");
    // 30 colours in a 1400-wide page: every column is in view without scrolling, and the text is still legible
    let (w30, h30, f30) = matrix_cell(30, 1400.0, 120.0, 1.0);
    assert!(120.0 + 30.0 * w30 <= 1400.0, "30 columns of {w30} fit");
    assert!(w30 >= 36.0 && f30 >= 8.0 && 30.0 * h30 <= 700.0, "{w30} x {h30}, font {f30}");
    // the user's own factor shrinks or grows them
    assert!(matrix_cell(30, 1400.0, 120.0, 0.5).0 < w30 && matrix_cell(30, 1400.0, 120.0, 2.0).0 > w30);
    // values lose the leading zero before they would overflow a narrow cell, and vanish only when unreadable
    assert_eq!(matrix_value(0.19, 60.0), "0.19");
    assert_eq!(matrix_value(0.19, 30.0), ".19");
    assert_eq!(matrix_value(1.0, 30.0), "1");
    assert_eq!(matrix_value(0.0, 60.0), "");
    assert_eq!(matrix_value(0.31, 18.0), "");

    // drawn for real, small and large: the whole matrix is laid out without a panic and takes the room computed
    let ctx = egui::Context::default();
    for k in [1usize, 6, 30] {
        let det: Vec<String> = (0..k).map(|i| format!("Alexa Fluor {}-A", 400 + 10 * i)).collect();
        let m: Vec<f64> = (0..k * k).map(|i| if i / k == i % k { 1.0 } else { 0.01 * ((i % 7) as f64) }).collect();
        let input = egui::RawInput { screen_rect: Some(screen()), ..Default::default() };
        let mut used = egui::Vec2::ZERO;
        let _ = ctx.run(input, |c| {
            egui::CentralPanel::default().show(c, |ui| {
                let before = ui.cursor().min;
                matrix_table(ui, "t_matrix", &det, &m, 1.0, [60, 150, 90], "help");
                used = ui.cursor().min - before;
            });
        });
        assert!(used.y > 30.0, "{k} detectors drew something ({used:?})");
    }
}

#[test]
fn a_quadrant_crosshair_is_dragged_as_one_gate_and_one_undo_step() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, plot) = ready_tab(&mut app, &ctx);
    let range = app.ws.graphs[&w].range.unwrap();
    let map = Mapper { rect: plot, range };
    let c0 = ((range.x0 + range.x1) / 2.0, (range.y0 + range.y1) / 2.0);
    let ids = app.session.commit_quadrants(w, c0.0, c0.1).unwrap();
    settle(&mut app, &ctx);
    let before: Vec<usize> = ids.iter().map(|&n| app.session.count(0, n)).collect();
    let total: usize = before.iter().sum();
    assert_eq!(app.session.quadrant_set(0, ids[2]).len(), 4, "the four sectors are recognised as one gate");

    // grab the centre and drag right and up: both lines follow, all four populations change, the total does not
    let from = map.to_screen(c0);
    let to = Pos2::new(from.x + 0.15 * plot.width(), from.y - 0.10 * plot.height());
    drag(&mut app, &ctx, from, to);
    let g: Vec<Gate> = ids.iter().map(|&n| gate_of(&app, n)).collect();
    assert!(g.iter().all(|q| q.xs[0] == g[0].xs[0] && q.ys[0] == g[0].ys[0]), "the sectors still share one crosshair");
    assert!(g[0].xs[0] > c0.0 && g[0].ys[0] > c0.1, "moved right and up: {:?}", (g[0].xs[0], g[0].ys[0]));
    let after: Vec<usize> = ids.iter().map(|&n| app.session.count(0, n)).collect();
    assert_ne!(after, before);
    assert_eq!(after.iter().sum::<usize>(), total, "still a partition of the parent");
    assert!(app.status.contains("Moved the quadrant"), "{}", app.status);

    // a point on the vertical line, away from the centre, moves only that line
    let c1 = (g[0].xs[0], g[0].ys[0]);
    let on_line = Pos2::new(map.to_screen(c1).x, plot.top() + 0.1 * plot.height());
    drag(&mut app, &ctx, on_line, Pos2::new(on_line.x - 0.05 * plot.width(), on_line.y + 30.0));
    let g2 = gate_of(&app, ids[0]);
    assert!(g2.xs[0] < c1.0 && g2.ys[0] == c1.1, "only x changed: {:?} -> {:?}", c1, (g2.xs[0], g2.ys[0]));

    // each drag is one undo step for the whole gate
    assert!(app.session.undo(0));
    assert_eq!((gate_of(&app, ids[3]).xs[0], gate_of(&app, ids[3]).ys[0]), c1);
    assert!(app.session.undo(0));
    assert!(ids.iter().all(|&n| (gate_of(&app, n).xs[0], gate_of(&app, n).ys[0]) == c0));
}

#[test]
fn cluster_separation_tells_real_groups_from_a_cloud_cut_into_pieces() {
    use crate::embed::{separation, separation_verdict};
    let mut rng = Rng::new(21);
    let mut normal = move || {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    };
    let (n, d, k) = (1200usize, 4usize, 4usize);
    // four well-separated groups, labelled by group
    let mut x = Vec::new();
    let mut lab = Vec::new();
    for i in 0..n {
        let g = i % k;
        for j in 0..d {
            x.push(if j == g { 6.0 } else { 0.0 } + 0.5 * normal());
        }
        lab.push(Some(g));
    }
    let good = separation(&x, d, &lab, k, 600);
    assert!(good > 0.6, "{good}");
    assert!(separation_verdict(good).starts_with("clear"));
    // one structureless cloud, cut into four by k-means: the cut is real, the groups are not
    let cloud: Vec<f64> = (0..n * d).map(|_| normal()).collect();
    let km = sf_core::kmeans::kmeans(&cloud, n, d, k, 3, 50, 1).unwrap();
    let cut: Vec<Option<usize>> = km.labels.iter().map(|&l| Some(l)).collect();
    let weak = separation(&cloud, d, &cut, k, 600);
    assert!(weak < 0.2, "{weak}");
    assert!(separation_verdict(0.05).starts_with("weak"));
    assert_eq!(separation(&cloud, d, &[None; 0], k, 600), 0.0);
}

#[test]
fn the_analysis_log_records_every_finished_action_and_writes_a_methods_text() {
    let ctx = egui::Context::default();
    let mut app = app_with_sample();
    let (w, _) = ready_tab(&mut app, &ctx);
    assert!(app.journal.is_empty(), "the start-up hint is not an action");
    let g = app.session.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 5e4 }, "Lymphocytes").unwrap();
    app.session.set_marker("CD3-A", Some("CD3"));
    // a progress message is not logged; the result is, once, with a time
    app.status = "Clustering…".into();
    settle(&mut app, &ctx);
    assert!(app.journal.is_empty());
    app.run_apoptosis();
    let said = app.status.clone();
    settle(&mut app, &ctx);
    assert_eq!(app.journal.len(), 1, "{:?}", app.journal);
    assert_eq!(app.journal[0].1, said);
    assert_eq!(app.journal[0].0.len(), 8, "HH:MM:SS");
    app.start_rename(0, g);
    settle(&mut app, &ctx);
    type_over(&mut app, &ctx, "Live");
    pointer(&mut app, &ctx, vec![key(egui::Key::Enter, false, false)]);
    settle(&mut app, &ctx);
    assert!(app.journal.iter().any(|(_, m)| m.contains("Renamed 'Lymphocytes' to 'Live'")), "{:?}", app.journal);

    let m = app.methods_text();
    for want in ["StreamFLOW", "1 FCS file(s), 3000 events", "Compensation: none", "CD3-A :: CD3", "Live: rectangle gate on FSC-A", "vs SSC-A", "Steps performed", "Renamed"] {
        assert!(m.contains(want), "missing {want:?} in:\n{m}");
    }
    // the screen itself draws
    app.module = Module::AnalysisLog;
    settle(&mut app, &ctx);
}
