//! The compensation wizard window, laid out like the old app's wizard: the controls table with the scatter gate of
//! the selected control (draw it, move it, resize it by its edges), every control's positive / negative split as a
//! grid of histograms whose limits are dragged in place, then a second page that gives the matrix and its quality
//! checks the whole window. The window can be maximised.

use super::{theme, App};
use crate::comp_auto::{detector_from_name, detector_from_signal};
use crate::comp_wizard::{auto_cut, compensated_pair, compute, control_mask, square_csv, CleanMode, ControlSetup, Method, NegSource, Options, SplitMethod, WizardResult};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Clone, Default)]
pub struct WizRow {
    pub detector: Option<String>,
    /// (negative up to, positive from), when set by hand.
    pub manual: Option<(f64, f64)>,
    /// This control's own negative (None = the wizard-wide choice).
    pub neg: Option<NegSource>,
    /// A scatter gate drawn by hand: FSC low, high, SSC low, high (raw units).
    pub scatter: Option<[f64; 4]>,
}

impl WizState {
    /// Re-apply the window size on the next frame (after maximise / restore).
    pub fn request_resize(&mut self) {
        self.resize = true;
    }
}

impl WizRow {
    pub fn edited(&self) -> bool {
        self.manual.is_some() || self.neg.is_some() || self.scatter.is_some()
    }
}

/// What is drawn for one control, kept until its settings change (the clean-up is too slow to redo every frame).
pub struct Cached {
    key: u64,
    vals: Vec<f64>,
    n_events: usize,
    zlo: f64,
    zhi: f64,
    hist: Vec<f64>,
    unst: Option<Vec<f64>>,
    auto: f64,
    pts: Vec<(f32, f32, bool)>,
    scatter_range: Option<[(f32, f32); 2]>,
}

pub struct WizState {
    pub open: bool,
    pub rows: Vec<WizRow>,
    pub opt: Options,
    pub clean_on: bool,
    pub clean_frac: f64,
    /// Which sample is the unstained tube, when the negative comes from one.
    pub unstained: Option<usize>,
    pub result: Option<Result<WizardResult, String>>,
    pub inspect: Option<usize>,
    pub show: Tab,
    /// Share of the calculated compensation shown in the before/after plots (1 = the matrix as calculated).
    pub strength: f64,
    pub page: Page,
    /// The control whose scatter gate is shown.
    pub sel: Option<usize>,
    pub maximized: bool,
    resize: bool,
    after_calc: bool,
    cache: HashMap<usize, Cached>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Summary,
    Spread,
    Cosine,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Setup,
    Results,
}

impl Default for WizState {
    fn default() -> Self {
        WizState { open: false, rows: vec![], opt: Options::default(), clean_on: true, clean_frac: 0.12, unstained: None, result: None, inspect: None, show: Tab::Summary, strength: 1.0, page: Page::Setup, sel: None, maximized: false, resize: false, after_calc: false, cache: HashMap::new() }
    }
}

const COF: f64 = 150.0;
const BINS: usize = 140;

fn z_of(v: f64) -> f64 {
    (v / COF).asinh()
}

fn raw_of(z: f64) -> f64 {
    COF * z.sinh()
}

fn short(v: f64) -> String {
    crate::axis::format_raw(v)
}

fn heat(v: f64, scale: f64, col: [u8; 3]) -> Color32 {
    let a = (v / scale).clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(col[0], col[1], col[2], (a * 170.0) as u8)
}

/// Fill (or refresh) what is drawn for control `si`.
fn ensure_cache(wiz: &mut WizState, session: &crate::session::Session, si: usize) {
    let Some(det) = wiz.rows[si].detector.clone() else {
        wiz.cache.remove(&si);
        return;
    };
    let s = session.sample(si);
    let Some(ci) = s.channel_index(&det) else {
        wiz.cache.remove(&si);
        return;
    };
    let un = wiz.unstained.filter(|&u| u != si);
    let key = {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (det.clone(), wiz.clean_on, wiz.clean_frac.to_bits(), wiz.rows[si].scatter.map(|r| r.map(f64::to_bits)), wiz.opt.split.label().to_string(), un, s.n_events).hash(&mut h);
        h.finish()
    };
    if wiz.cache.get(&si).map_or(false, |c| c.key == key) {
        return;
    }
    let d = s.channels.len();
    let keep = control_mask(s, if wiz.clean_on { CleanMode::Dominant(wiz.clean_frac) } else { CleanMode::Off }, Some(ci), wiz.rows[si].scatter);
    let vals: Vec<f64> = (0..s.n_events).filter(|&r| keep[r]).map(|r| s.events[r * d + ci] as f64).collect();
    let zs: Vec<f64> = (0..s.n_events).map(|r| z_of(s.events[r * d + ci] as f64)).collect();
    let (zlo, zhi) = (sf_core::stats::percentile(&zs, 0.2), sf_core::stats::percentile(&zs, 99.9));
    let hist_of = |v: &mut dyn Iterator<Item = f64>| -> Vec<f64> {
        let mut h = vec![0.0; BINS];
        for x in v {
            let t = (z_of(x) - zlo) / (zhi - zlo).max(1e-9);
            if (0.0..1.0).contains(&t) {
                h[(t * BINS as f64) as usize] += 1.0;
            }
        }
        h
    };
    let hist = hist_of(&mut vals.iter().copied());
    let unst = un.and_then(|u| {
        let us = session.sample(u);
        us.channel_index(&det).map(|c| {
            let ud = us.channels.len();
            hist_of(&mut (0..us.n_events).map(|r| us.events[r * ud + c] as f64))
        })
    });
    let auto = if vals.is_empty() { 0.0 } else { auto_cut(&vals, wiz.opt.split) };
    let fsc = s.channels.iter().position(|c| c.to_ascii_uppercase() == "FSC-A");
    let ssc = s.channels.iter().position(|c| c.to_ascii_uppercase() == "SSC-A");
    let (pts, scatter_range) = match (fsc, ssc) {
        (Some(f), Some(sc)) => ((0..s.n_events).step_by((s.n_events / 6000).max(1)).map(|r| (s.events[r * d + f], s.events[r * d + sc], keep[r])).collect(), Some([(0.0, s.ranges[f] as f32), (0.0, s.ranges[sc] as f32)])),
        _ => (vec![], None),
    };
    wiz.cache.insert(si, Cached { key, vals, n_events: s.n_events, zlo, zhi, hist, unst, auto, pts, scatter_range });
}

impl App {
    /// Open the wizard with a first guess of which sample is which.
    pub fn open_comp_wizard(&mut self) {
        let n = self.session.samples().len();
        if n == 0 {
            self.status = "Load the single-stain controls first.".into();
            return;
        }
        let dets = super::comp_view::fluor_detectors(self.session.sample(0).base_channels());
        let w = &mut self.comp.wiz;
        if w.rows.len() != n {
            w.rows = (0..n).map(|i| WizRow { detector: detector_from_name(&self.session.samples()[i].name, &dets), ..Default::default() }).collect();
            w.unstained = (0..n).find(|&i| {
                let nm = self.session.samples()[i].name.to_ascii_lowercase();
                nm.contains("unstain") || nm.contains("negative") || nm.contains("blank")
            });
            w.result = None;
            w.cache.clear();
            w.sel = w.rows.iter().position(|r| r.detector.is_some());
        }
        w.open = true;
    }

    /// Open the wizard on the Compensation screen, calculate, and optionally open one control (screenshots, checks).
    pub fn demo_comp_wizard(&mut self, inspect: Option<usize>) {
        self.set_module(super::Module::Compensation);
        self.open_comp_wizard();
        self.wizard_calculate();
        self.comp.wiz.inspect = inspect;
    }

    /// Show a page of the wizard (screenshots, checks).
    pub fn demo_comp_wizard_page(&mut self, results: bool) {
        self.comp.wiz.page = if results { Page::Results } else { Page::Setup };
        self.comp.wiz.maximized = true;
        self.comp.wiz.request_resize();
    }

    fn wiz_setups(&self) -> Vec<ControlSetup> {
        self.comp.wiz.rows.iter().enumerate().filter_map(|(i, r)| r.detector.clone().map(|d| ControlSetup { sample: i, detector: d, manual: r.manual, neg: r.neg, scatter: r.scatter })).collect()
    }

    /// Calculate with the wizard's current choices.
    pub fn wizard_calculate(&mut self) {
        let n = self.session.samples().len();
        if n == 0 {
            return;
        }
        let dets = super::comp_view::fluor_detectors(self.session.sample(0).base_channels());
        let setups = self.wiz_setups();
        let mut opt = self.comp.wiz.opt.clone();
        opt.clean = if self.comp.wiz.clean_on { CleanMode::Dominant(self.comp.wiz.clean_frac) } else { CleanMode::Off };
        if let NegSource::Unstained(_) = opt.negative {
            match self.comp.wiz.unstained {
                Some(u) => opt.negative = NegSource::Unstained(u),
                None => opt.negative = NegSource::Own,
            }
        }
        let refs: Vec<&crate::sample::LoadedSample> = (0..n).map(|i| self.session.sample(i)).collect();
        let r = compute(&refs, &setups, &dets, &opt);
        self.status = match &r {
            Ok(w) => format!("Calculated a {0}×{0} matrix from {1} control(s) ({2}). Review it, then use it.", dets.len(), setups.len(), opt.method.label()) + &if w.rounds > 0 { format!(" Refined in {} round(s).", w.rounds) } else { String::new() },
            Err(e) => format!("Cannot calculate: {e}"),
        };
        self.comp.wiz.result = Some(r);
    }

    pub(crate) fn comp_wizard_window(&mut self, ctx: &egui::Context) {
        if !self.comp.wiz.open {
            return;
        }
        let n = self.session.samples().len();
        if n == 0 || self.comp.wiz.rows.len() != n {
            self.comp.wiz.open = false;
            return;
        }
        let dets = super::comp_view::fluor_detectors(self.session.sample(0).base_channels());
        let mut wiz = std::mem::take(&mut self.comp.wiz);
        for i in 0..n {
            ensure_cache(&mut wiz, &self.session, i);
        }
        if wiz.sel.map_or(true, |s| s >= n || wiz.rows[s].detector.is_none()) {
            wiz.sel = wiz.rows.iter().position(|r| r.detector.is_some());
        }
        if wiz.after_calc && wiz.result.is_some() {
            wiz.after_calc = false;
            if matches!(wiz.result, Some(Ok(_))) {
                wiz.page = Page::Results;
            }
        }
        let mut open = true;
        let (mut calc, mut use_it, mut inspect_click) = (false, false, None::<usize>);
        let mut export_msg: Option<String> = None;
        let screen = ctx.screen_rect();
        let normal = Rect::from_min_size(Pos2::new(100.0, 40.0), Vec2::new(1180.0f32.min(screen.width() - 120.0), (screen.height() - 90.0).clamp(420.0, 780.0)));
        let full = Rect::from_min_max(Pos2::new(screen.left() + 8.0, screen.top() + 44.0), Pos2::new(screen.right() - 8.0, screen.bottom() - 34.0));
        let mut win = egui::Window::new("Compensation wizard").open(&mut open).collapsible(false).resizable(true).default_size(normal.size()).default_pos(normal.min);
        // maximised: the window is pinned to the screen; otherwise it is free (one fixed frame after a restore)
        let pinned = if wiz.maximized { Some(full) } else if wiz.resize { Some(normal) } else { None };
        if let Some(r) = pinned {
            win = win.current_pos(r.min).fixed_size(r.size() - Vec2::new(18.0, 46.0));
        }
        wiz.resize = false;
        let (avail_w, avail_h) = if wiz.maximized { (full.width() - 18.0, full.height() - 46.0) } else { (normal.width() - 18.0, normal.height() - 46.0) };
        win.show(ctx, |ui| {
            let avail = Vec2::new(avail_w, avail_h);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut wiz.page, Page::Setup, RichText::new("1  Controls and gates").size(13.5));
                ui.selectable_value(&mut wiz.page, Page::Results, RichText::new("2  Matrix and quality").size(13.5));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(if wiz.maximized { "🗗 Restore" } else { "🗖 Maximize" }).on_hover_text("Make the wizard fill the window").clicked() {
                        wiz.maximized = !wiz.maximized;
                        wiz.request_resize();
                    }
                });
            });
            ui.separator();
            let footer = 44.0;
            let body_h = (avail.y - footer - 88.0).max(220.0);
            ui.allocate_ui(Vec2::new(avail.x, body_h), |ui| {
                ui.set_min_size(Vec2::new(avail.x, body_h));
                match wiz.page {
                    Page::Setup => self.wizard_setup_page(ui, &mut wiz, &dets, avail.x, body_h, &mut inspect_click),
                    Page::Results => self.wizard_results_page(ui, &mut wiz, &mut export_msg),
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(RichText::new("Calculate").strong()).fill(theme::ACCENT_DIM).min_size(Vec2::new(130.0, 30.0))).clicked() {
                    calc = true;
                }
                let ok = matches!(wiz.result, Some(Ok(_)));
                if ui.add_enabled(ok, egui::Button::new("Use this matrix").min_size(Vec2::new(140.0, 30.0))).on_hover_text("Puts the matrix in the Compensation screen to review, edit and apply").clicked() {
                    use_it = true;
                }
                match &wiz.result {
                    Some(Err(e)) => {
                        ui.add(egui::Label::new(RichText::new(format!("✕ {e}")).color(theme::GATE_RED)).truncate());
                    }
                    Some(Ok(_)) => {
                        ui.label(RichText::new("✓ calculated").color(theme::GOOD));
                    }
                    None => {
                        theme::caption(ui, "Press Calculate.");
                    }
                }
            });
        });
        if let Some(i) = inspect_click {
            wiz.inspect = Some(i);
        }
        if let Some(m) = export_msg {
            self.status = m;
        }
        if calc {
            wiz.after_calc = true;
            wiz.result = None;
        }
        self.comp.wiz = wiz;
        if calc {
            self.run_busy("Calculating the spillover matrix…", |app| app.wizard_calculate());
        }
        if use_it {
            if let Some(Ok(r)) = &self.comp.wiz.result {
                self.comp.use_matrix(r.matrix.clone(), r.notes.clone());
                self.comp.for_sample = Some(self.comp.sample);
                self.status = "The calculated matrix is in the Compensation screen: review it, then apply it.".into();
                open = false;
            }
        }
        if !open {
            self.comp.wiz.open = false;
        }
        self.inspect_window(ctx);
    }

    fn wizard_setup_page(&mut self, ui: &mut egui::Ui, wiz: &mut WizState, dets: &[String], width: f32, height: f32, inspect_click: &mut Option<usize>) {
        let n = self.session.samples().len();
        let left_w = (width * 0.43).clamp(430.0, 620.0);
        ui.horizontal_top(|ui| {
            // ---- left: controls and options
            ui.vertical(|ui| {
                ui.set_width(left_w);
                ui.label(RichText::new("Controls").strong());
                egui::ScrollArea::vertical().id_salt("wiz_rows").max_height(((n as f32 + 1.0) * 38.0).min(height * 0.5).max(150.0)).auto_shrink([false, true]).show(ui, |ui| {
                    egui::Grid::new("wiz_grid").spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
                        for h in ["Sample", "Parameter", "Neg | Pos", ""] {
                            ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
                        }
                        ui.end_row();
                        for i in 0..n {
                            let nm = self.session.samples()[i].name.trim_end_matches(".fcs").to_string();
                            let is_un = wiz.unstained == Some(i);
                            let label = if is_un { format!("{nm}  (unstained)") } else { nm };
                            if ui.selectable_label(wiz.sel == Some(i), RichText::new(label).size(12.0)).on_hover_text("Click to see this control's scatter gate").clicked() && wiz.rows[i].detector.is_some() {
                                wiz.sel = Some(i);
                            }
                            let cur = wiz.rows[i].detector.clone().unwrap_or_else(|| "—".into());
                            egui::ComboBox::from_id_salt(("wiz_det", i)).width(110.0).selected_text(cur).show_ui(ui, |ui| {
                                if ui.selectable_label(wiz.rows[i].detector.is_none(), "—").clicked() {
                                    wiz.rows[i] = WizRow::default();
                                }
                                for d in dets {
                                    if ui.selectable_label(wiz.rows[i].detector.as_deref() == Some(d.as_str()), d).clicked() {
                                        wiz.rows[i] = WizRow { detector: Some(d.clone()), ..Default::default() };
                                    }
                                }
                            });
                            let status = match (&wiz.result, wiz.rows[i].detector.as_ref()) {
                                (Some(Ok(r)), Some(_)) => r.reports.iter().find(|c| c.sample == i).map(|c| {
                                    let mut t = format!("{} | {}", c.n_neg, c.n_pos);
                                    if wiz.rows[i].edited() {
                                        t += " (manual)";
                                    }
                                    (t, c.badges.iter().any(|b| b.blocks()), !c.badges.is_empty(), c.badges.iter().map(|b| b.label().to_string()).chain([format!("{} of {} events kept by the scatter clean-up", c.n_clean, c.n_events), format!("stain index {:.1}", c.stain_index)]).collect::<Vec<_>>().join("\n"))
                                }),
                                _ => None,
                            };
                            match status {
                                Some((t, bad, warn, tip)) => {
                                    ui.label(RichText::new(t).size(11.5).color(if bad { theme::GATE_RED } else if warn { theme::WARN } else { theme::GOOD })).on_hover_text(tip);
                                }
                                None => {
                                    ui.label("");
                                }
                            }
                            ui.horizontal(|ui| {
                                if ui.add_enabled(wiz.rows[i].detector.is_some(), egui::Button::new("Details…").small()).on_hover_text("This control before and after compensation, with its own negative and a compensation-strength preview").clicked() {
                                    *inspect_click = Some(i);
                                }
                                if ui.add_enabled(wiz.rows[i].edited(), egui::Button::new("↺").small()).on_hover_text("Reset this control to the automatic limits, gate and negative").clicked() {
                                    wiz.rows[i].manual = None;
                                    wiz.rows[i].neg = None;
                                    wiz.rows[i].scatter = None;
                                }
                                let guess = detector_from_name(&self.session.samples()[i].name, dets);
                                if wiz.rows[i].detector.is_some() && guess != wiz.rows[i].detector {
                                    let tip = if guess.is_some() { "The file name looks like a different detector. Check that this is the right control." } else { "The file name does not mention this detector. Are you sure this is the right control?" };
                                    ui.label(RichText::new("⚠").color(theme::WARN)).on_hover_text(tip);
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Guess from file names").clicked() {
                        for i in 0..n {
                            wiz.rows[i] = WizRow { detector: detector_from_name(&self.session.samples()[i].name, dets), ..Default::default() };
                        }
                    }
                    if ui.small_button("Guess from signal").on_hover_text("The detector that is brightest in each file").clicked() {
                        for i in 0..n {
                            if wiz.rows[i].detector.is_none() {
                                wiz.rows[i] = WizRow { detector: detector_from_signal(self.session.sample(i), dets), ..Default::default() };
                            }
                        }
                    }
                    if ui.small_button("Clear all").clicked() {
                        wiz.rows.iter_mut().for_each(|r| *r = WizRow::default());
                    }
                    if ui.small_button("Reset all edits").on_hover_text("Recalculate every control automatically: any limits, scatter gates and negatives set by hand are lost").clicked() {
                        wiz.rows.iter_mut().for_each(|r| {
                            r.manual = None;
                            r.neg = None;
                            r.scatter = None;
                        });
                    }
                    if ui.small_button("Remove unusable").on_hover_text("Unassign controls found to have no usable positive or negative population").clicked() {
                        let bad: Vec<usize> = match &wiz.result {
                            Some(Ok(res)) => res.reports.iter().filter(|c| c.badges.iter().any(|b| b.blocks())).map(|c| c.sample).collect(),
                            _ => vec![],
                        };
                        for i in bad {
                            wiz.rows[i] = WizRow::default();
                        }
                    }
                });
                ui.separator();
                ui.label(RichText::new("Method").strong());
                ui.horizontal_wrapped(|ui| {
                    for m in Method::ALL {
                        ui.radio_value(&mut wiz.opt.method, m, m.label());
                    }
                });
                theme::caption(ui, match wiz.opt.method {
                    Method::Medians => "Needs a clear negative and positive population in every control.",
                    Method::Slope => "Needs no negative population; uses the whole intensity range of the control.",
                    Method::Refined => "Needs no negative population. A robust line per detector, then the matrix is corrected until compensated controls show no leftover slope.",
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Negative").strong());
                let mut which = match wiz.opt.negative {
                    NegSource::Own => 0,
                    NegSource::Unstained(_) => 1,
                    NegSource::TrueZero => 2,
                };
                ui.radio_value(&mut which, 0, "Each control's own negative events");
                ui.add_enabled_ui(wiz.unstained.is_some() || n > 1, |ui| {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut which, 1, "Unstained tube:");
                        let cur = wiz.unstained.map_or("—".to_string(), |u| self.session.samples()[u].name.trim_end_matches(".fcs").to_string());
                        egui::ComboBox::from_id_salt("wiz_unstained").width(150.0).selected_text(cur).show_ui(ui, |ui| {
                            for i in 0..n {
                                ui.selectable_value(&mut wiz.unstained, Some(i), self.session.samples()[i].name.trim_end_matches(".fcs").to_string());
                            }
                        });
                    });
                });
                ui.radio_value(&mut which, 2, "True zero (no background subtracted)");
                wiz.opt.negative = match which {
                    1 => NegSource::Unstained(wiz.unstained.unwrap_or(0)),
                    2 => NegSource::TrueZero,
                    _ => NegSource::Own,
                };
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.checkbox(&mut wiz.clean_on, "Scatter clean-up");
                    ui.add_enabled(wiz.clean_on, egui::Slider::new(&mut wiz.clean_frac, 0.03..=0.6).text("keep above").fixed_decimals(2)).on_hover_text("Fraction of the density peak a scatter region must reach to be kept; lower keeps more events");
                });
                ui.horizontal(|ui| {
                    ui.label("Split");
                    egui::ComboBox::from_id_salt("wiz_split").width(170.0).selected_text(wiz.opt.split.label()).show_ui(ui, |ui| {
                        for s in SplitMethod::ALL {
                            ui.selectable_value(&mut wiz.opt.split, s, s.label());
                        }
                    });
                });
            });
            ui.separator();
            // ---- right: the selected control's scatter gate, then every control's split
            ui.vertical(|ui| {
                let rw = ui.available_width().max(300.0);
                ui.set_max_width(rw);
                if let Some(si) = wiz.sel {
                    let name = self.session.samples()[si].name.trim_end_matches(".fcs").to_string();
                    let det = wiz.rows[si].detector.clone().unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Scatter gate: {name} — {det}")).strong());
                        if ui.add_enabled(wiz.rows[si].scatter.is_some(), egui::Button::new("Automatic").small()).on_hover_text("Back to the automatic clean-up").clicked() {
                            wiz.rows[si].scatter = None;
                        }
                    });
                    theme::caption(ui, "Drag on empty space to draw the gate, drag inside it to move it, drag an edge or corner to resize it. Each control keeps its own.");
                    ui.horizontal_top(|ui| {
                        let plot_h = (height * 0.36).clamp(180.0, 330.0);
                        if let Some(c) = wiz.cache.get(&si) {
                            if let Some(range) = c.scatter_range {
                                scatter_editor(ui, &c.pts, range, Vec2::new(plot_h * 1.15, plot_h), "FSC-A", "SSC-A", Some(&mut wiz.rows[si].scatter));
                            } else {
                                theme::caption(ui, "This file has no FSC-A / SSC-A channels, so there is nothing to clean up.");
                            }
                        }
                        ui.vertical(|ui| {
                            if let Some(c) = wiz.cache.get(&si) {
                                ui.label(RichText::new(format!("{} of {} events inside", c.vals.len(), c.n_events)).size(12.0).color(theme::MUTED));
                            }
                            ui.label("Negative for this control");
                            let cur = match wiz.rows[si].neg {
                                None => "same as the wizard".to_string(),
                                Some(NegSource::Own) => "its own negative events".into(),
                                Some(NegSource::TrueZero) => "true zero".into(),
                                Some(NegSource::Unstained(u)) => format!("unstained: {}", self.session.samples()[u].name.trim_end_matches(".fcs")),
                            };
                            egui::ComboBox::from_id_salt("wiz_row_neg_main").width(210.0).selected_text(cur).show_ui(ui, |ui| {
                                ui.selectable_value(&mut wiz.rows[si].neg, None, "same as the wizard");
                                ui.selectable_value(&mut wiz.rows[si].neg, Some(NegSource::Own), "its own negative events");
                                ui.selectable_value(&mut wiz.rows[si].neg, Some(NegSource::TrueZero), "true zero");
                                for u in (0..n).filter(|&u| u != si) {
                                    ui.selectable_value(&mut wiz.rows[si].neg, Some(NegSource::Unstained(u)), format!("unstained: {}", self.session.samples()[u].name.trim_end_matches(".fcs")));
                                }
                            });
                        });
                    });
                    ui.separator();
                }
                ui.label(RichText::new("Positive / negative separation").strong());
                theme::caption(ui, "Drag the blue line (negative up to) or the red line (positive from) in any histogram. Click one to select its control.");
                let cols = ((ui.available_width() - 14.0) / 268.0).floor().max(1.0) as usize;
                let shown: Vec<usize> = (0..n).filter(|&i| wiz.rows[i].detector.is_some() && wiz.cache.contains_key(&i)).collect();
                egui::ScrollArea::vertical().id_salt("wiz_hists").auto_shrink([false, false]).show(ui, |ui| {
                    // wrapped by hand: a block of widgets does not wrap by itself
                    for line in shown.chunks(cols) {
                        ui.horizontal(|ui| {
                            for &i in line {
                                let nm = self.session.samples()[i].name.trim_end_matches(".fcs").to_string();
                                let det = wiz.rows[i].detector.clone().unwrap_or_default();
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(format!("{nm} — {}", det.trim_end_matches("-A"))).size(11.5).color(if wiz.sel == Some(i) { theme::ACCENT } else { theme::MUTED }));
                                    let (c, row) = (wiz.cache.get(&i).unwrap(), &mut wiz.rows[i]);
                                    if hist_editor(ui, c, row, Vec2::new(260.0, 130.0), ("wiz_h", i)).clicked() {
                                        wiz.sel = Some(i);
                                    }
                                });
                            }
                        });
                        ui.add_space(4.0);
                    }
                });
            });
        });
    }

    fn wizard_results_page(&mut self, ui: &mut egui::Ui, wiz: &mut WizState, export_msg: &mut Option<String>) {
        match &wiz.result {
            None => {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| theme::caption(ui, "Press Calculate to see the matrix and its checks here."));
            }
            Some(Err(e)) => {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| ui.label(RichText::new(format!("✕ {e}")).color(theme::GATE_RED)));
            }
            Some(Ok(r)) => {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut wiz.show, Tab::Summary, "Summary and matrix");
                    ui.selectable_value(&mut wiz.show, Tab::Spread, "Spillover spreading");
                    ui.selectable_value(&mut wiz.show, Tab::Cosine, "Signature similarity");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (what, text) = match wiz.show {
                            Tab::Spread => ("spillover_spreading.csv", square_csv(&r.matrix.detectors, &r.spread)),
                            Tab::Cosine => ("signature_similarity.csv", square_csv(&r.matrix.detectors, &r.cosine)),
                            Tab::Summary => ("spillover.csv", square_csv(&r.matrix.detectors, &r.matrix.spill)),
                        };
                        if ui.small_button("Export CSV…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name(what).save_file() {
                                *export_msg = Some(match std::fs::write(&p, text) {
                                    Ok(()) => format!("Saved {}", p.display()),
                                    Err(e) => format!("Could not save: {e}"),
                                });
                            }
                        }
                    });
                });
                ui.add_space(4.0);
                match wiz.show {
                    Tab::Summary => {
                        result_summary(ui, r, &self.session);
                        ui.add_space(6.0);
                        matrix_table(ui, "wiz_matrix", &r.matrix.detectors, &r.matrix.spill, 1.0, [60, 150, 90], "The spillover matrix: row = the stained control's detector, column = the detector that sees its light (1 on the diagonal).");
                    }
                    Tab::Spread => matrix_table(ui, "wiz_spread", &r.matrix.detectors, &r.spread, 1.0, [224, 120, 40], "Noise each stained control adds to every other detector after compensation (standard deviation per √intensity). Large values mean that pair is hard to resolve."),
                    Tab::Cosine => matrix_table(ui, "wiz_cos", &r.matrix.detectors, &r.cosine, 1.0, [0, 150, 200], "Cosine similarity of the spillover rows: close to 1 means two fluorochromes look alike on this instrument."),
                }
            }
        }
    }

    /// One control in detail: its split, the scatter gate, its own negative and, once a matrix is calculated, the
    /// control before and after compensation with a strength preview.
    fn inspect_window(&mut self, ctx: &egui::Context) {
        let Some(si) = self.comp.wiz.inspect else { return };
        let Some(det) = self.comp.wiz.rows.get(si).and_then(|r| r.detector.clone()) else {
            self.comp.wiz.inspect = None;
            return;
        };
        let mut wiz = std::mem::take(&mut self.comp.wiz);
        ensure_cache(&mut wiz, &self.session, si);
        let mut open = true;
        let s = self.session.sample(si);
        let name = s.name.clone();
        let ci = s.channel_index(&det);
        egui::Window::new(format!("Control: {name} — {det}")).id(egui::Id::new("wiz_inspect")).open(&mut open).collapsible(false).resizable(true).default_size([700.0, 640.0]).default_pos([260.0, 90.0]).show(ctx, |ui| {
            let Some(ci) = ci else {
                ui.label("This sample has no such channel.");
                return;
            };
            let d = s.channels.len();
            let zhi = wiz.cache.get(&si).map_or(1.0, |c| c.zhi);
            if let Some(c) = wiz.cache.get(&si) {
                let row = &mut wiz.rows[si];
                hist_editor(ui, c, row, Vec2::new(ui.available_width().min(660.0), 220.0), ("wiz_hi", si));
                let (nm, pm) = row.manual.unwrap_or((c.auto, c.auto));
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("negative ≤ {}: {}", short(nm), c.vals.iter().filter(|&&v| v <= nm).count())).color(Color32::from_rgb(40, 90, 170)));
                    ui.label(RichText::new(format!("positive ≥ {}: {}", short(pm), c.vals.iter().filter(|&&v| v >= pm).count())).color(Color32::from_rgb(170, 50, 40)));
                    if ui.button("Automatic").on_hover_text(format!("Back to the automatic limit ({})", short(c.auto))).clicked() {
                        row.manual = None;
                    }
                    if c.unst.is_some() {
                        theme::caption(ui, "black line: the unstained tube");
                    }
                });
            }
            ui.separator();
            ui.horizontal_top(|ui| {
                if let Some((Some(range), true)) = wiz.cache.get(&si).map(|c| (c.scatter_range, true)) {
                    let pts = wiz.cache[&si].pts.clone();
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Scatter gate").size(11.0));
                        scatter_editor(ui, &pts, range, Vec2::new(230.0, 200.0), "FSC-A", "SSC-A", Some(&mut wiz.rows[si].scatter));
                    });
                }
                if let Some(Ok(r)) = &wiz.result {
                    let scaled = {
                        let mut m = r.matrix.clone();
                        let k = m.detectors.len();
                        for a in 0..k {
                            for b in 0..k {
                                if a != b {
                                    m.spill[a * k + b] *= wiz.strength;
                                }
                            }
                        }
                        m
                    };
                    if let Some(o) = strongest_spill(r, &det) {
                        let step = (s.n_events / 4000).max(1);
                        if let Ok((cx, cy)) = compensated_pair(s, &scaled, &det, &o) {
                            let raw: Vec<(f32, f32, bool)> = (0..s.n_events).step_by(step).map(|r| (z_of(s.events[r * d + ci] as f64) as f32, z_of(s.channel_index(&o).map_or(0.0, |c| s.events[r * d + c] as f64)) as f32, true)).collect();
                            let step2 = (cx.len() / 4000).max(1);
                            let after: Vec<(f32, f32, bool)> = (0..cx.len()).step_by(step2).map(|i| (z_of(cx[i]) as f32, z_of(cy[i]) as f32, true)).collect();
                            let rng = [(z_of(-500.0) as f32, zhi as f32), (z_of(-500.0) as f32, zhi as f32)];
                            ui.vertical(|ui| {
                                ui.label(RichText::new(format!("{det} vs {o}: before")).size(11.0));
                                scatter_editor(ui, &raw, rng, Vec2::new(230.0, 200.0), &det, &o, None);
                            });
                            ui.vertical(|ui| {
                                ui.label(RichText::new(format!("after compensation ({:.0}%)", wiz.strength * 100.0)).size(11.0));
                                scatter_editor(ui, &after, rng, Vec2::new(230.0, 200.0), &det, &o, None);
                            });
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Negative for this control");
                let cur = match wiz.rows[si].neg {
                    None => "same as the wizard".to_string(),
                    Some(NegSource::Own) => "its own negative events".into(),
                    Some(NegSource::TrueZero) => "true zero".into(),
                    Some(NegSource::Unstained(u)) => format!("unstained: {}", self.session.samples()[u].name.trim_end_matches(".fcs")),
                };
                egui::ComboBox::from_id_salt("wiz_row_neg").width(220.0).selected_text(cur).show_ui(ui, |ui| {
                    ui.selectable_value(&mut wiz.rows[si].neg, None, "same as the wizard");
                    ui.selectable_value(&mut wiz.rows[si].neg, Some(NegSource::Own), "its own negative events");
                    ui.selectable_value(&mut wiz.rows[si].neg, Some(NegSource::TrueZero), "true zero");
                    for u in (0..self.session.samples().len()).filter(|&u| u != si) {
                        ui.selectable_value(&mut wiz.rows[si].neg, Some(NegSource::Unstained(u)), format!("unstained: {}", self.session.samples()[u].name.trim_end_matches(".fcs")));
                    }
                });
            });
            if matches!(wiz.result, Some(Ok(_))) {
                ui.horizontal(|ui| {
                    ui.label("Compensation applied");
                    ui.add(egui::Slider::new(&mut wiz.strength, 0.0..=3.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).custom_parser(|t| t.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0)));
                    if ui.small_button("100%").clicked() {
                        wiz.strength = 1.0;
                    }
                    theme::caption(ui, "under- and over-compensation preview");
                });
            }
        });
        if !open {
            wiz.inspect = None;
        }
        self.comp.wiz = wiz;
    }
}

fn strongest_spill(r: &WizardResult, det: &str) -> Option<String> {
    let d = r.matrix.detectors.len();
    let i = r.matrix.detectors.iter().position(|n| n == det)?;
    (0..d).filter(|&j| j != i).max_by(|&a, &b| r.matrix.spill[i * d + a].partial_cmp(&r.matrix.spill[i * d + b]).unwrap_or(std::cmp::Ordering::Equal)).map(|j| r.matrix.detectors[j].clone())
}

/// A control's histogram on the arcsinh scale with the negative / positive limits as two lines that are dragged in
/// place (the nearer one follows the pointer).
fn hist_editor(ui: &mut egui::Ui, c: &Cached, row: &mut WizRow, size: Vec2, id: (&str, usize)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let _ = id;
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, Color32::WHITE);
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 6.0, rect.top() + 6.0), Pos2::new(rect.right() - 6.0, rect.bottom() - 18.0));
    let (neg_max, pos_min) = row.manual.unwrap_or((c.auto, c.auto));
    let hmax = c.hist.iter().cloned().fold(1.0, f64::max).max(c.unst.as_ref().map_or(0.0, |u| u.iter().cloned().fold(0.0, f64::max)));
    let span = (c.zhi - c.zlo).max(1e-9);
    let x_of = |z: f64| plot.left() + ((z - c.zlo) / span) as f32 * plot.width();
    let z_at = |x: f32| c.zlo + ((x - plot.left()) / plot.width()) as f64 * span;
    for (b, &n) in c.hist.iter().enumerate() {
        let x0 = plot.left() + plot.width() * b as f32 / BINS as f32;
        let hgt = (n / hmax) as f32 * plot.height();
        let z_mid = c.zlo + span * (b as f64 + 0.5) / BINS as f64;
        let col = if raw_of(z_mid) <= neg_max { Color32::from_rgb(70, 130, 200) } else if raw_of(z_mid) >= pos_min { Color32::from_rgb(200, 80, 70) } else { Color32::from_gray(170) };
        p.rect_filled(Rect::from_min_max(Pos2::new(x0, plot.bottom() - hgt), Pos2::new(x0 + plot.width() / BINS as f32, plot.bottom())), 0.0, col);
    }
    if let Some(hu) = &c.unst {
        let pts: Vec<Pos2> = hu.iter().enumerate().map(|(b, &n)| Pos2::new(plot.left() + plot.width() * (b as f32 + 0.5) / BINS as f32, plot.bottom() - (n / hmax) as f32 * plot.height())).collect();
        p.add(egui::Shape::line(pts, Stroke::new(1.3_f32, Color32::from_rgb(40, 40, 40))));
    }
    for raw in [-1000.0, 0.0, 1000.0, 10_000.0, 100_000.0] {
        let z = z_of(raw);
        if z > c.zlo && z < c.zhi {
            let x = x_of(z);
            p.line_segment([Pos2::new(x, plot.bottom()), Pos2::new(x, plot.bottom() + 3.0)], Stroke::new(1.0_f32, Color32::GRAY));
            p.text(Pos2::new(x, plot.bottom() + 4.0), Align2::CENTER_TOP, short(raw), FontId::proportional(9.5), Color32::from_gray(60));
        }
    }
    p.rect_stroke(plot, 0.0, Stroke::new(1.0_f32, Color32::GRAY));
    let (xn, xp) = (x_of(z_of(neg_max)), x_of(z_of(pos_min)));
    p.line_segment([Pos2::new(xn, plot.top()), Pos2::new(xn, plot.bottom())], Stroke::new(2.0_f32, Color32::from_rgb(40, 90, 170)));
    p.line_segment([Pos2::new(xp, plot.top()), Pos2::new(xp, plot.bottom())], Stroke::new(2.0_f32, Color32::from_rgb(170, 50, 40)));
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos().filter(|q| plot.expand(4.0).contains(*q)) {
            let raw = raw_of(z_at(pos.x.clamp(plot.left(), plot.right())));
            let (mut nm, mut pm) = (neg_max, pos_min);
            // the limit closest to the pointer moves; they keep their order
            if (pos.x - xn).abs() <= (pos.x - xp).abs() {
                nm = raw.min(pm);
            } else {
                pm = raw.max(nm);
            }
            row.manual = Some((nm, pm));
        }
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    resp.on_hover_text("Drag the blue line (negative up to) or the red line (positive from)")
}

/// A scatter plot of events. With a gate it is also an editor: drag on empty space to draw a rectangle, inside it to
/// move it, on an edge or corner to resize it.
pub(super) fn scatter_editor(ui: &mut egui::Ui, pts: &[(f32, f32, bool)], range: [(f32, f32); 2], size: Vec2, xl: &str, yl: &str, mut gate: Option<&mut Option<[f64; 4]>>) {
    let (rect, resp) = ui.allocate_exact_size(size, if gate.is_some() { Sense::click_and_drag() } else { Sense::hover() });
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::WHITE);
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 6.0, rect.top() + 6.0), Pos2::new(rect.right() - 6.0, rect.bottom() - 16.0));
    let (xr, yr) = (((range[0].1 - range[0].0).max(1e-6)) as f64, ((range[1].1 - range[1].0).max(1e-6)) as f64);
    let (x0, y0) = (range[0].0 as f64, range[1].0 as f64);
    for &(x, y, kept) in pts {
        let (fx, fy) = (((x - range[0].0) as f64 / xr) as f32, ((y - range[1].0) as f64 / yr) as f32);
        if (0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy) {
            p.circle_filled(Pos2::new(plot.left() + fx * plot.width(), plot.bottom() - fy * plot.height()), 1.1, if kept { Color32::from_rgba_unmultiplied(40, 90, 170, 150) } else { Color32::from_rgba_unmultiplied(170, 170, 170, 120) });
        }
    }
    p.rect_stroke(plot, 0.0, Stroke::new(1.0_f32, Color32::GRAY));
    if let Some(g) = gate.as_deref_mut() {
        let to_raw = |q: Pos2| (x0 + ((q.x - plot.left()) / plot.width()).clamp(0.0, 1.0) as f64 * xr, y0 + ((plot.bottom() - q.y) / plot.height()).clamp(0.0, 1.0) as f64 * yr);
        let to_scr = |x: f64, y: f64| Pos2::new(plot.left() + ((x - x0) / xr) as f32 * plot.width(), plot.bottom() - ((y - y0) / yr) as f32 * plot.height());
        // 0 = outside (draw), 1 = inside (move), 10 + edges = resize (1 left, 2 right, 4 bottom, 8 top)
        let mode_at = |q: Pos2, r: Option<[f64; 4]>| -> i32 {
            let Some(r) = r else { return 0 };
            let rr = Rect::from_two_pos(to_scr(r[0], r[3]), to_scr(r[1], r[2]));
            let tol = 6.0;
            if !rr.expand(tol).contains(q) {
                return 0;
            }
            let mut f = 0;
            if (q.x - rr.left()).abs() <= tol {
                f |= 1;
            }
            if (q.x - rr.right()).abs() <= tol {
                f |= 2;
            }
            if (q.y - rr.bottom()).abs() <= tol {
                f |= 4;
            }
            if (q.y - rr.top()).abs() <= tol {
                f |= 8;
            }
            if f != 0 {
                10 + f
            } else if rr.contains(q) {
                1
            } else {
                0
            }
        };
        let id = resp.id.with("gate_drag");
        if resp.drag_started() {
            // egui reports the start only after the pointer has moved a little: the gate begins where the button went DOWN
            if let Some(q) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos()) {
                let (rx, ry) = to_raw(q);
                let mode = mode_at(q, *g);
                ui.data_mut(|d| d.insert_temp(id, (mode, rx, ry, (*g).unwrap_or([rx, rx, ry, ry]))));
            }
        }
        if resp.dragged() {
            if let (Some(q), Some((mode, ax, ay, orig))) = (resp.interact_pointer_pos(), ui.data(|d| d.get_temp::<(i32, f64, f64, [f64; 4])>(id))) {
                let (nx, ny) = to_raw(q);
                *g = Some(match mode {
                    0 => [ax.min(nx), ax.max(nx), ay.min(ny), ay.max(ny)],
                    1 => {
                        let dx = (nx - ax).max(x0 - orig[0]).min(x0 + xr - orig[1]);
                        let dy = (ny - ay).max(y0 - orig[2]).min(y0 + yr - orig[3]);
                        [orig[0] + dx, orig[1] + dx, orig[2] + dy, orig[3] + dy]
                    }
                    m => {
                        let f = m - 10;
                        let mut r = orig;
                        if f & 1 != 0 {
                            r[0] = nx;
                        }
                        if f & 2 != 0 {
                            r[1] = nx;
                        }
                        if f & 4 != 0 {
                            r[2] = ny;
                        }
                        if f & 8 != 0 {
                            r[3] = ny;
                        }
                        [r[0].min(r[1]), r[0].max(r[1]), r[2].min(r[3]), r[2].max(r[3])]
                    }
                });
            }
        }
        if let Some(r) = *g {
            let gr = Rect::from_two_pos(to_scr(r[0], r[3]), to_scr(r[1], r[2]));
            p.rect_filled(gr, 0.0, Color32::from_rgba_unmultiplied(200, 60, 40, 22));
            p.rect_stroke(gr, 0.0, Stroke::new(1.8_f32, Color32::from_rgb(200, 60, 40)));
            for c in [gr.left_top(), gr.right_top(), gr.left_bottom(), gr.right_bottom()] {
                p.rect_filled(Rect::from_center_size(c, Vec2::splat(6.0)), 1.0, Color32::from_rgb(200, 60, 40));
            }
        }
        if let Some(q) = resp.hover_pos() {
            ui.ctx().set_cursor_icon(match mode_at(q, *g) {
                0 => egui::CursorIcon::Crosshair,
                1 => egui::CursorIcon::Grab,
                m => {
                    let f = m - 10;
                    if f & 3 != 0 && f & 12 != 0 {
                        egui::CursorIcon::ResizeNeSw
                    } else if f & 3 != 0 {
                        egui::CursorIcon::ResizeHorizontal
                    } else {
                        egui::CursorIcon::ResizeVertical
                    }
                }
            });
        }
        resp.on_hover_text("Drag to draw, move or resize the scatter gate used for this control");
    }
    p.text(Pos2::new(plot.center().x, rect.bottom() - 2.0), Align2::CENTER_BOTTOM, format!("{xl}  \u{2194}   {yl} \u{2195}"), FontId::proportional(9.5), Color32::from_gray(80));
}

fn result_summary(ui: &mut egui::Ui, r: &WizardResult, session: &crate::session::Session) {
    ui.horizontal_wrapped(|ui| {
        ui.label(format!("{} controls", r.reports.len()));
        ui.label("·");
        let c = r.condition;
        ui.label(RichText::new(format!("condition {}", if c.is_finite() { format!("{c:.1}") } else { "∞".into() })).color(if !c.is_finite() || c > 30.0 { theme::GATE_RED } else if c > 10.0 { theme::WARN } else { theme::GOOD }));
        if r.rounds > 0 {
            ui.label("·");
            ui.label(format!("refined in {} round(s), largest leftover slope {:.4}", r.rounds, r.residual_slope));
        }
    });
    egui::Grid::new("wiz_sum").spacing([14.0, 3.0]).striped(true).show(ui, |ui| {
        for h in ["Control", "Detector", "Events kept", "Negative", "Positive", "Stain index", "Warnings"] {
            ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
        }
        ui.end_row();
        for c in &r.reports {
            ui.label(RichText::new(session.samples()[c.sample].name.trim_end_matches(".fcs")).size(12.0));
            ui.label(RichText::new(c.detector.trim_end_matches("-A")).size(12.0));
            ui.label(RichText::new(format!("{} / {}", c.n_clean, c.n_events)).size(12.0));
            ui.label(RichText::new(format!("{} (≤{})", c.n_neg, short(c.neg_max))).size(12.0));
            ui.label(RichText::new(format!("{} (≥{})", c.n_pos, short(c.pos_min))).size(12.0));
            ui.label(RichText::new(if c.stain_index.is_finite() { format!("{:.1}", c.stain_index) } else { "—".into() }).size(12.0));
            let w: Vec<&str> = c.badges.iter().map(|b| b.label()).collect();
            ui.label(RichText::new(if w.is_empty() { "—".to_string() } else { w.join("; ") }).size(11.5).color(if w.is_empty() { theme::MUTED } else { theme::WARN }));
            ui.end_row();
        }
    });
    for n in &r.notes {
        theme::caption(ui, n.clone());
    }
}

/// A heat-map table. The cells grow with the room there is, so the figure is as large as the window allows.
pub(super) fn matrix_table(ui: &mut egui::Ui, id: &str, det: &[String], m: &[f64], scale: f64, col: [u8; 3], help: &str) {
    theme::caption(ui, help);
    let k = det.len().max(1);
    let cell_w = ((ui.available_width() - 70.0) / k as f32 - 3.0).clamp(46.0, 120.0);
    let cell_h = (cell_w * 0.5).clamp(20.0, 44.0);
    let font = (cell_h * 0.5).clamp(10.5, 16.0);
    egui::ScrollArea::both().id_salt(id).auto_shrink([false, true]).max_height(ui.available_height().max(240.0)).show(ui, |ui| {
        egui::Grid::new(id).spacing([3.0, 3.0]).min_col_width(cell_w).show(ui, |ui| {
            ui.label("");
            for d in det {
                ui.label(RichText::new(d.trim_end_matches("-A")).size(font - 1.0).color(theme::MUTED));
            }
            ui.end_row();
            for i in 0..det.len() {
                ui.label(RichText::new(det[i].trim_end_matches("-A")).size(font - 1.0).color(theme::MUTED));
                for j in 0..det.len() {
                    let v = m[i * det.len() + j];
                    egui::Frame::none().fill(heat(v, scale, col)).rounding(3.0).show(ui, |ui| ui.add_sized([cell_w, cell_h], egui::Label::new(RichText::new(if v == 0.0 { "".to_string() } else { format!("{v:.2}") }).monospace().size(font))));
                }
                ui.end_row();
            }
        });
    });
}
