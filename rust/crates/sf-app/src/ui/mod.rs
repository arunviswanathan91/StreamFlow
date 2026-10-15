//! The egui front end: a left navigation rail (collapsible, like the original app's sidebar), a module
//! host, and a status strip. All state lives in [`Session`]; each module draws it and forwards input.

mod comp_view;
mod analysis_view;
mod discover_view;
mod export_view;
mod graph;
mod modules;
mod spectral_view;
mod stats_view;
mod theme;
mod workstation;

#[cfg(test)]
mod tests;

pub use discover_view::ColorBy;
pub use graph::Mapper;

use crate::session::Session;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use sf_core::json::Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Module {
    Workstation,
    Compensation,
    Statistics,
    DimReduction,
    Clustering,
    Spectral,
    CellCycle,
    Proliferation,
    Apoptosis,
    StatsComparison,
    Kinetic,
    Classifier,
    CrossSample,
    Longitudinal,
    Scatter3d,
    Export,
    Plugins,
    AnalysisLog,
}

impl Module {
    pub const GROUPS: [(&'static str, &'static [Module]); 4] = [
        ("Workspace", &[Module::Workstation, Module::Compensation, Module::Statistics]),
        ("Discovery", &[Module::DimReduction, Module::Clustering, Module::Spectral]),
        (
            "Analyses",
            &[Module::CellCycle, Module::Proliferation, Module::Apoptosis, Module::StatsComparison, Module::Kinetic, Module::Classifier, Module::CrossSample, Module::Longitudinal, Module::Scatter3d],
        ),
        ("Output", &[Module::Export, Module::Plugins, Module::AnalysisLog]),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Module::Workstation => "Workstation",
            Module::Compensation => "Compensation",
            Module::Statistics => "Statistics",
            Module::DimReduction => "Dim. Reduction",
            Module::Clustering => "Clustering",
            Module::Spectral => "Spectral Unmixing",
            Module::CellCycle => "Cell Cycle",
            Module::Proliferation => "Proliferation",
            Module::Apoptosis => "Apoptosis",
            Module::StatsComparison => "Stats Comparison",
            Module::Kinetic => "Kinetic",
            Module::Classifier => "Classifier",
            Module::CrossSample => "Cross-Sample",
            Module::Longitudinal => "Longitudinal",
            Module::Scatter3d => "3D Scatter",
            Module::Export => "Export",
            Module::Plugins => "Plugins",
            Module::AnalysisLog => "Analysis Log",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Module::Workstation => "▦",
            Module::Compensation => "⇄",
            Module::Statistics => "∑",
            Module::DimReduction => "✺",
            Module::Clustering => "◍",
            Module::Spectral => "≋",
            Module::CellCycle => "◔",
            Module::Proliferation => "⤴",
            Module::Apoptosis => "✕",
            Module::StatsComparison => "≠",
            Module::Kinetic => "∿",
            Module::Classifier => "◈",
            Module::CrossSample => "⊞",
            Module::Longitudinal => "⌇",
            Module::Scatter3d => "◇",
            Module::Export => "⇪",
            Module::Plugins => "⚙",
            Module::AnalysisLog => "☰",
        }
    }
}

pub struct App {
    pub session: Session,
    pub(crate) module: Module,
    nav_expanded: bool,
    pub(crate) ws: workstation::State,
    pub(crate) stats: stats_view::State,
    pub(crate) comp: comp_view::State,
    pub(crate) export: export_view::State,
    pub disc: discover_view::State,
    pub ana: analysis_view::State,
    pub(crate) disc_last_click: Option<(f64, eframe::egui::Pos2)>,
    pub spec: spectral_view::State,
    pub(crate) status: String,
    themed: bool,
    shot: Option<Shot>,
}

/// A request to save this app's own rendering as a PNG and quit (for documentation and checks); it only ever
/// captures the app window, never the desktop.
struct Shot {
    path: std::path::PathBuf,
    wait_frames: u32,
    requested: bool,
}

impl Default for App {
    fn default() -> Self {
        App::new()
    }
}

impl App {
    pub fn new() -> App {
        App {
            session: Session::new(),
            module: Module::Workstation,
            nav_expanded: true,
            ws: workstation::State::default(),
            stats: stats_view::State::default(),
            comp: comp_view::State::default(),
            export: export_view::State::default(),
            disc: discover_view::State::default(),
            ana: analysis_view::State::default(),
            disc_last_click: None,
            spec: spectral_view::State::default(),
            status: "Open FCS files from the File menu (Ctrl+O).".into(),
            themed: false,
            shot: None,
        }
    }

    /// Save a PNG of the app's window after `wait_frames` frames, then quit.
    pub fn request_screenshot(&mut self, path: std::path::PathBuf, wait_frames: u32) {
        self.shot = Some(Shot { path, wait_frames, requested: false });
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    pub fn module(&self) -> Module {
        self.module
    }

    pub fn set_module(&mut self, m: Module) {
        self.module = m;
    }

    /// Start-up view options (used by the command line and screenshots).
    pub fn set_nav_expanded(&mut self, expanded: bool) {
        self.nav_expanded = expanded;
    }

    /// Show the Workstation Overview tab.
    pub fn show_overview(&mut self) {
        self.ws.tab = workstation::Tab::Overview;
    }

    // ---- file actions -------------------------------------------------------------------------

    pub(crate) fn open_fcs(&mut self) {
        let Some(files) = rfd::FileDialog::new().add_filter("FCS files", &["fcs"]).set_title("Open FCS files").pick_files() else { return };
        self.load_files(&files);
    }

    pub fn load_files(&mut self, files: &[std::path::PathBuf]) {
        let (mut ok, mut failed) = (0, Vec::new());
        for f in files {
            match self.session.load_fcs(f) {
                Ok(_) => ok += 1,
                Err(e) => failed.push(e),
            }
        }
        self.status = if failed.is_empty() { format!("Loaded {ok} file(s).") } else { format!("Loaded {ok}; {} failed: {}", failed.len(), failed.join(" | ")) };
    }

    pub(crate) fn save_workspace(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("StreamFLOW workspace", &["sfw"]).set_file_name("experiment.sfw").save_file() else { return };
        for w in self.session.windows.iter().map(|w| w.id).collect::<Vec<_>>() {
            self.session.remember_view(w);
        }
        self.status = match std::fs::write(&path, self.session.to_workspace_json().write()) {
            Ok(()) => format!("Saved {}", path.display()),
            Err(e) => format!("Could not save {}: {e}", path.display()),
        };
    }

    pub(crate) fn open_workspace(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("StreamFLOW workspace", &["sfw"]).pick_file() else { return };
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| Json::parse(&t).map_err(|e| e.to_string()))
            .and_then(|j| self.session.apply_workspace_json(&j));
        self.ws.reset();
        self.status = match result {
            Ok(w) if w.is_empty() => format!("Opened {}", path.display()),
            Ok(w) => format!("Opened {} with {} note(s): {}", path.display(), w.len(), w.join(" | ")),
            Err(e) => format!("Could not open {}: {e}", path.display()),
        };
    }

    // ---- frame ----------------------------------------------------------------------------------

    /// One frame of UI. Separate from `eframe::App::update` so it can be run headlessly in tests.
    pub fn show(&mut self, ctx: &egui::Context) {
        if !self.themed {
            theme::apply(ctx);
            self.themed = true;
        }
        self.shortcuts(ctx);
        self.title_bar(ctx);
        self.status_bar(ctx);
        self.nav_rail(ctx);
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG)).show(ctx, |ui| match self.module {
            Module::Workstation => self.workstation(ui, ctx),
            Module::Compensation => self.compensation(ui),
            Module::Statistics => self.statistics(ui),
            Module::DimReduction => self.dimred(ui, ctx),
            Module::Clustering => self.clustering(ui, ctx),
            Module::Spectral => self.spectral(ui),
            Module::CellCycle => self.cell_cycle_screen(ui),
            Module::Proliferation => self.proliferation_screen(ui),
            Module::Apoptosis => self.apoptosis_screen(ui),
            Module::Export => self.export_module(ui),
            Module::AnalysisLog => self.analysis_log(ui),
            other => modules::placeholder(ui, other),
        });
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let ctrl = |ctx: &egui::Context, k: egui::Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        if ctrl(ctx, egui::Key::B) {
            self.nav_expanded = !self.nav_expanded;
        }
        if ctrl(ctx, egui::Key::O) {
            self.open_fcs();
        }
        if ctrl(ctx, egui::Key::S) {
            self.save_workspace();
        }
    }

    fn title_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("titlebar").frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 6.0))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("☰").size(16.0)).frame(false)).on_hover_text("Show / hide the navigation (Ctrl+B)").clicked() {
                    self.nav_expanded = !self.nav_expanded;
                }
                ui.label(RichText::new("StreamFLOW").strong().size(16.0).color(theme::ACCENT));
                ui.add_space(14.0);
                ui.menu_button("File", |ui| {
                    if ui.button("Load FCS…        Ctrl+O").clicked() {
                        ui.close_menu();
                        self.open_fcs();
                    }
                    ui.separator();
                    if ui.button("Open workspace…").clicked() {
                        ui.close_menu();
                        self.open_workspace();
                    }
                    if ui.button("Import FlowJo workspace…").clicked() {
                        ui.close_menu();
                        self.import_flowjo();
                    }
                    if ui.button("Save workspace…   Ctrl+S").clicked() {
                        ui.close_menu();
                        self.save_workspace();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui.button(if self.nav_expanded { "Collapse navigation" } else { "Expand navigation" }).clicked() {
                        ui.close_menu();
                        self.nav_expanded = !self.nav_expanded;
                    }
                    if ui.button(if self.ws.pops_open { "Hide populations panel" } else { "Show populations panel" }).clicked() {
                        ui.close_menu();
                        self.ws.pops_open = !self.ws.pops_open;
                    }
                    if ui.button(if self.ws.options_open { "Hide plot options" } else { "Show plot options" }).clicked() {
                        ui.close_menu();
                        self.ws.options_open = !self.ws.options_open;
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label("Double-click a population to open it.");
                    ui.label("P R E I Q  – gate tools,  N – pointer");
                    ui.label("Ctrl+Z / Ctrl+Y – undo / redo");
                    ui.label("Ctrl+B – hide / show navigation");
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(job) = self.disc.job_label() {
                        ui.label(RichText::new(format!("⏳ {job}")).color(theme::WARN).size(12.0));
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.add_space(8.0);
                    }
                    let n = self.session.samples().len();
                    let ev: usize = self.session.samples().iter().map(|s| s.n_events).sum();
                    ui.label(RichText::new(format!("{n} sample{}  ·  {ev} events", if n == 1 { "" } else { "s" })).color(theme::MUTED).size(12.0));
                });
            });
        });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(12.0, 4.0))).show(ctx, |ui| {
            let w = ui.available_width();
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(egui::vec2(w * 0.68, 18.0), Layout::left_to_right(Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&self.status).size(12.0)).truncate()).on_hover_text(&self.status);
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(l) = self.session.log.last() {
                        ui.add(egui::Label::new(RichText::new(l).size(11.0).color(theme::WARN)).truncate()).on_hover_text(l);
                    }
                });
            });
        });
    }

    fn nav_rail(&mut self, ctx: &egui::Context) {
        let target = if self.nav_expanded { 214.0 } else { 54.0 };
        let w = ctx.animate_value_with_time(egui::Id::new("nav_width"), target, 0.12);
        egui::SidePanel::left("nav").exact_width(w).resizable(false).frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(6.0, 8.0))).show(ctx, |ui| {
            let expanded = self.nav_expanded && w > 120.0;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (gi, (group, items)) in Module::GROUPS.iter().enumerate() {
                    if gi > 0 {
                        ui.add_space(6.0);
                    }
                    if expanded {
                        ui.label(RichText::new(group.to_uppercase()).size(10.0).strong().color(theme::MUTED));
                    } else if gi > 0 {
                        theme::rule(ui);
                    }
                    for &m in items.iter() {
                        let r = theme::nav_item(ui, m.icon(), m.label(), self.module == m, expanded);
                        if r.clicked() {
                            self.module = m;
                        }
                    }
                }
            });
        });
    }

    fn import_flowjo(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("FlowJo workspace", &["wsp"]).set_title("Import FlowJo workspace").pick_file() else { return };
        self.import_flowjo_path(&path);
    }

    pub fn import_flowjo_path(&mut self, path: &std::path::Path) {
        let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        self.status = match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| self.session.import_wsp(&t, &dir)) {
            Ok(r) => {
                self.ws.reset();
                r.summary()
            }
            Err(e) => format!("Could not import {}: {e}", path.display()),
        };
    }

    // ---- analysis log -----------------------------------------------------------------------------

    fn analysis_log(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none().inner_margin(16.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Analysis log");
                if ui.button("Clear").clicked() {
                    self.session.log.clear();
                }
            });
            theme::caption(ui, "Warnings from loading, evaluation and imports. Nothing is hidden: a gate that could not be evaluated is reported here.");
            ui.add_space(8.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if self.session.log.is_empty() {
                    ui.label(RichText::new("Nothing to report.").color(theme::MUTED));
                }
                for (i, l) in self.session.log.iter().enumerate().rev() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("{:>3}", i + 1)).color(theme::MUTED).monospace());
                        ui.label(RichText::new(l).color(if l.contains("skipped") || l.contains("not in this") { theme::WARN } else { Color32::LIGHT_GRAY }));
                    });
                }
            });
        });
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.show(ctx);
        if let Some(s) = self.shot.as_mut() {
            ctx.request_repaint();
            if s.wait_frames > 0 {
                s.wait_frames -= 1;
            } else if !s.requested {
                s.requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let image = ctx.input(|i| i.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(img) = image {
                let path = s.path.clone();
                if let Err(e) = write_png(&path, &img) {
                    eprintln!("screenshot failed: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

fn write_png(path: &std::path::Path, img: &egui::ColorImage) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), img.width() as u32, img.height() as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    w.write_image_data(&bytes).map_err(|e| e.to_string())
}
