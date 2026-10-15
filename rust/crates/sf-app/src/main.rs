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
