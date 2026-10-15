use sf_app::ui::App;
use sf_gating::GateTree;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // `streamflow [files]`: `.fcs` files are loaded, a `.wsp` FlowJo workspace is imported (its FCS files are
    // looked up next to it); the first population is then opened in a tab.
    let mut app = App::new();
    let (mut fcs, mut wsp) = (Vec::<PathBuf>::new(), Vec::<PathBuf>::new());
    for arg in std::env::args().skip(1).filter(|a| !a.starts_with("--")) {
        let p = PathBuf::from(&arg);
        match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("wsp") => wsp.push(p),
            _ => fcs.push(p),
        }
    }
    if !fcs.is_empty() {
        app.load_files(&fcs);
    }
    for w in &wsp {
        app.import_flowjo_path(w);
    }
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().find_map(|a| a.strip_prefix(&format!("--{name}=")).map(str::to_string));
    if !app.session.samples().is_empty() {
        let si = app.session.samples().iter().position(|s| s.comp.is_some()).unwrap_or(0);
        let deepest = app.session.tree(si).self_and_descendants(GateTree::ROOT).ok().and_then(|n| n.into_iter().max_by_key(|&x| app.session.tree(si).depth(x).unwrap_or(0))).unwrap_or(GateTree::ROOT);
        let _ = app.open_tab(si, GateTree::ROOT);
        if deepest != GateTree::ROOT {
            let _ = app.open_tab(si, deepest);
        }
    }
    if let Some(m) = flag("module") {
        let all: Vec<sf_app::ui::Module> = sf_app::ui::Module::GROUPS.iter().flat_map(|(_, i)| i.iter().copied()).collect();
        if let Some(found) = all.into_iter().find(|x| x.label().to_ascii_lowercase().starts_with(&m.to_ascii_lowercase())) {
            app.set_module(found);
        }
    }
    if let Some(dir) = flag("load-dir") {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().map_or(false, |x| x.eq_ignore_ascii_case("fcs"))).collect()).unwrap_or_default();
        files.sort();
        app.load_files(&files);
    }
    let wait = |app: &mut App| {
        while app.disc.is_busy() {
            std::thread::sleep(std::time::Duration::from_millis(50));
            app.poll_jobs();
        }
    };
    if args.iter().any(|a| a == "--run-embedding") {
        app.set_module(sf_app::ui::Module::DimReduction);
        app.poll_jobs();
        app.disc.per_sample = 2500;
        app.disc_run_embedding();
        wait(&mut app);
    }
    if args.iter().any(|a| a == "--run-cluster") {
        app.poll_jobs();
        app.disc_run_clustering();
        wait(&mut app);
        app.disc.picked = vec![true; app.disc.clus.as_ref().map_or(0, |c| c.n_clusters)];
        app.disc.color = sf_app::ui::ColorBy::Cluster;
    }
    if args.iter().any(|a| a == "--build-spectral") {
        app.spectral_guess_roles();
        app.spec.af_k = 0;
        match app.spectral_build() {
            Ok(()) => {
                let n = app.session.samples().len();
                app.spectral_unmix(&(0..n).collect::<Vec<_>>());
            }
            Err(e) => app.set_status(e),
        }
    }
    if let Some(i) = flag("analysis-sample").and_then(|v| v.parse::<usize>().ok()) {
        app.ana.sample = i.min(app.session.samples().len().saturating_sub(1));
    }
    if let Some(c) = flag("analysis-channel") {
        app.ana.cycle_channel = c.clone();
        app.ana.prolif_channel = c;
    }
    if args.iter().any(|a| a == "--run-cell-cycle") {
        app.set_module(sf_app::ui::Module::CellCycle);
        app.run_cell_cycle();
    }
    if args.iter().any(|a| a == "--run-proliferation") {
        app.set_module(sf_app::ui::Module::Proliferation);
        app.run_proliferation();
    }
    if args.iter().any(|a| a == "--run-apoptosis") {
        app.set_module(sf_app::ui::Module::Apoptosis);
        app.run_apoptosis();
    }
    if args.iter().any(|a| a == "--auto-groups") {
        let n = app.session.samples().len();
        app.cmp.groups = (0..n).map(|i| if i < n / 2 { "A".to_string() } else { "B".to_string() }).collect();
    }
    if args.iter().any(|a| a == "--run-comparison") {
        app.set_module(sf_app::ui::Module::StatsComparison);
        app.run_comparison();
    }
    if args.iter().any(|a| a == "--run-kinetics") {
        app.set_module(sf_app::ui::Module::Kinetic);
        app.run_kinetics();
    }
    if args.iter().any(|a| a == "--select-gate") {
        // select the first gate drawn on the open tab (for screenshots of the move/resize handles)
        if let Some(si) = (!app.session.samples().is_empty()).then_some(0) {
            if let Some(first) = app.session.tree(si).children(GateTree::ROOT).ok().and_then(|c| c.first().copied()) {
                app.session.selected = Some((si, first));
            }
        }
    }
    if args.iter().any(|a| a == "--demo-gates") {
        // draw a rectangle, an ellipse and a polygon on the first window, selecting the ellipse (for screenshots)
        if let Some(id) = app.session.windows.first().map(|w| w.id) {
            if let Some(r) = app.session.auto_range(id) {
                let (w, h) = (r.x1 - r.x0, r.y1 - r.y0);
                let _ = app.session.commit_shape(id, &sf_app::drawing::Shape::Rect { x0: r.x0 + 0.1 * w, x1: r.x0 + 0.35 * w, y0: r.y0 + 0.55 * h, y1: r.y0 + 0.85 * h }, "Rect");
                if let Ok(n) = app.session.commit_shape(id, &sf_app::drawing::Shape::Ellipse { cx: r.x0 + 0.6 * w, cy: r.y0 + 0.35 * h, rx: 0.2 * w, ry: 0.12 * h }, "Ellipse") {
                    let sample = app.session.window(id).map_or(0, |w| w.sample);
                    app.session.selected = Some((sample, n));
                }
                let _ = app.session.commit_shape(id, &sf_app::drawing::Shape::Polygon(vec![(r.x0 + 0.55 * w, r.y0 + 0.65 * h), (r.x0 + 0.9 * w, r.y0 + 0.7 * h), (r.x0 + 0.75 * w, r.y0 + 0.92 * h)]), "Poly");
            }
        }
    }
    if args.iter().any(|a| a == "--run-cross-sample") {
        app.set_module(sf_app::ui::Module::CrossSample);
        app.run_cross_sample();
    }
    if args.iter().any(|a| a == "--run-classifier") {
        app.set_module(sf_app::ui::Module::Classifier);
        app.run_cross_sample();
        app.run_classifier();
    }
    if (flag("axes").is_some() || flag("fmo-from").is_some() || args.iter().any(|a| a == "--strip")) && app.session.windows.is_empty() && !app.session.samples().is_empty() {
        let _ = app.open_tab(0, GateTree::ROOT);
    }
    if let Some(ax) = flag("axes") {
        // --axes=X,Y : show these channels on the first window
        if let (Some(id), Some((x, y))) = (app.session.windows.first().map(|w| w.id), ax.split_once(',')) {
            if let Some(win) = app.session.window_mut(id) {
                win.x_chan = x.to_string();
                win.y_chan = y.to_string();
            }
            app.refit_window(id);
        }
    }
    if let Some(name) = flag("fmo-from") {
        // --fmo-from=unstained : use the first sample whose name contains this as the FMO control, then come back
        if let Some(id) = app.session.windows.first().map(|w| w.id) {
            let back = app.session.window(id).map_or(0, |w| w.sample);
            if let Some(si) = app.session.samples().iter().position(|s| s.name.to_lowercase().contains(&name.to_lowercase())) {
                if app.session.retarget_window(id, si).is_ok() {
                    match app.session.set_fmo_from_window(id, 95.0) {
                        Ok(done) => app.set_status(format!("FMO from {name}: {done:?}")),
                        Err(e) => app.set_status(e),
                    }
                    let _ = app.session.retarget_window(id, back);
                }
            }
        }
    }
    if args.iter().any(|a| a == "--strip") {
        if let Some(id) = app.session.windows.first().map(|w| w.id) {
            app.set_strip_open(id, true);
        }
    }
    if args.iter().any(|a| a == "--build-layout") || flag("layout-pdf").is_some() {
        app.set_module(sf_app::ui::Module::Layout);
        if let Some(ax) = flag("axes") {
            if let Some((x, y)) = ax.split_once(',') {
                app.lay.x_chan = x.to_string();
                app.lay.y_chan = y.to_string();
            }
        }
        if let Some(g) = flag("layout-grid") {
            if let Some((c, r)) = g.split_once('x') {
                app.lay.spec.cols = c.parse().unwrap_or(3);
                app.lay.spec.rows = r.parse().unwrap_or(2);
            }
        }
        if let Some(pop) = flag("layout-population") {
            app.lay.population = pop.split('/').map(str::to_string).filter(|s| !s.is_empty()).collect();
        }
        match app.build_layout() {
            Ok(n) => eprintln!("layout built: {n} page(s)"),
            Err(e) => eprintln!("layout failed: {e}"),
        }
        if let Some(p) = flag("layout-pdf") {
            match app.write_layout_pdf(std::path::Path::new(&p)) {
                Ok(()) => eprintln!("wrote {p}"),
                Err(e) => eprintln!("could not write {p}: {e}"),
            }
            if let Some(p) = flag("layout-svg") {
                if let Err(e) = app.write_layout_svg(std::path::Path::new(&p)) {
                    eprintln!("could not write {p}: {e}");
                }
            }
            std::process::exit(0);
        }
    }
    if args.iter().any(|a| a == "--editor-demo") || flag("layout-batch-pdf").is_some() {
        app.demo_layout(flag("iterate-keyword"));
        if let Some(p) = flag("layout-batch-pdf") {
            match app.editor_run_batch(None) {
                Ok(n) => eprintln!("batch: {n} page(s)"),
                Err(e) => eprintln!("batch failed: {e}"),
            }
            match app.write_batch_pdf(std::path::Path::new(&p)) {
                Ok(()) => eprintln!("wrote {p}"),
                Err(e) => eprintln!("could not write {p}: {e}"),
            }
            std::process::exit(0);
        }
    }
    if args.iter().any(|a| a == "--compute-stats") {
        app.compute_statistics();
    }
    if flag("nav").as_deref() == Some("collapsed") {
        app.set_nav_expanded(false);
    }
    if flag("tab").as_deref() == Some("overview") {
        app.show_overview();
    }
    if let Some(p) = flag("screenshot") {
        app.request_screenshot(PathBuf::from(p), 60);
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_maximized(true).with_min_inner_size([960.0, 600.0]).with_title("StreamFLOW"),
        ..Default::default()
    };
    eframe::run_native("StreamFLOW", options, Box::new(move |_cc| Ok(Box::new(app))))
}
