//! Cell Cycle, Proliferation and Apoptosis screens. Each one fits a population of one sample on one (or two)
//! channels with the ports in `sf_core::analysis`, shows the fit, and reports the numbers.

use super::{theme, App};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke};
use sf_core::analysis::{apoptosis, cell_cycle, proliferation, ApoptosisResult, CycleModel, CycleOptions, CycleResult, ProliferationResult};
use sf_gating::GateTree;

const G1: Color32 = Color32::from_rgb(0x4C, 0xAF, 0x50);
const S_PHASE: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
const G2: Color32 = Color32::from_rgb(0xE0, 0x55, 0x55);
const FIT: Color32 = Color32::from_rgb(0xFF, 0xFF, 0xFF);
const GEN: [Color32; 8] = [
    Color32::from_rgb(0x1F, 0x77, 0xB4),
    Color32::from_rgb(0xFF, 0x7F, 0x0E),
    Color32::from_rgb(0x2C, 0xA0, 0x2C),
    Color32::from_rgb(0xD6, 0x27, 0x28),
    Color32::from_rgb(0x94, 0x67, 0xBD),
    Color32::from_rgb(0x8C, 0x56, 0x4B),
    Color32::from_rgb(0xE3, 0x77, 0xC2),
    Color32::from_rgb(0xBC, 0xBD, 0x22),
];

/// Which analysis a screen runs; each keeps its own channel choices.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Cycle,
    Prolif,
    Apoptosis,
}

pub struct State {
    pub sample: usize,
    pub restrict: Vec<String>,
    pub cycle_channel: String,
    pub prolif_channel: String,
    pub annexin_channel: String,
    pub pi_channel: String,
    pub dj_fox: bool,
    pub use_hint: bool,
    pub mu_hint: f64,
    pub generations: usize,
    pub cofactor: f64,
    pub cycle: Option<(String, CycleResult)>,
    pub prolif: Option<(String, ProliferationResult)>,
    pub apop: Option<(String, ApoptosisResult, Vec<(f64, f64)>)>,
}

impl Default for State {
    fn default() -> Self {
        State {
            sample: 0,
            restrict: vec![],
            cycle_channel: String::new(),
            prolif_channel: String::new(),
            annexin_channel: String::new(),
            pi_channel: String::new(),
            dj_fox: true,
            use_hint: false,
            mu_hint: 0.0,
            generations: 8,
            cofactor: 150.0,
            cycle: None,
            prolif: None,
            apop: None,
        }
    }
}

/// First channel whose name contains one of the hints (case-insensitive), preferring area (`-A`) channels.
pub fn guess_channel(channels: &[String], hints: &[&str]) -> Option<String> {
    let lower: Vec<String> = channels.iter().map(|c| c.to_lowercase()).collect();
    for h in hints {
        let h = h.to_lowercase();
        let hits: Vec<usize> = (0..channels.len()).filter(|&i| lower[i].contains(&h)).collect();
        if let Some(&i) = hits.iter().find(|&&i| lower[i].ends_with("-a")).or(hits.first()) {
            return Some(channels[i].clone());
        }
    }
    None
}

impl App {
    fn ana_sync(&mut self) {
        let n = self.session.samples().len();
        if self.ana.sample >= n {
            self.ana.sample = 0;
            self.ana.restrict.clear();
        }
        if n == 0 {
            return;
        }
        let chans = self.session.sample(self.ana.sample).channels.clone();
        let fluor: Vec<String> = chans.iter().filter(|c| !c.starts_with("FSC") && !c.starts_with("SSC") && c.as_str() != "Time" && !c.ends_with("-W") && !c.ends_with("-H")).cloned().collect();
        let fix = |cur: &mut String, hints: &[&str], fallback: Option<&String>| {
            if cur.is_empty() || !chans.contains(cur) {
                *cur = guess_channel(&chans, hints).or_else(|| fallback.cloned()).unwrap_or_default();
            }
        };
        fix(&mut self.ana.cycle_channel, &["PI", "DAPI", "7-AAD", "7AAD", "Hoechst", "DNA", "FxCycle", "DRAQ"], fluor.first());
        fix(&mut self.ana.prolif_channel, &["CFSE", "CTV", "CellTrace", "Violet", "eFluor 670", "FITC"], fluor.first());
        fix(&mut self.ana.annexin_channel, &["Annexin", "FITC", "FL1"], fluor.first());
        fix(&mut self.ana.pi_channel, &["PI", "7-AAD", "7AAD", "FL2", "PE"], fluor.get(1).or(fluor.first()));
    }

    /// Raw values of `channel` for the chosen population of the chosen sample.
    fn ana_values(&mut self, channel: &str) -> Result<Vec<f64>, String> {
        let si = self.ana.sample;
        let path: Vec<String> = self.ana.restrict.clone();
        let node = if path.is_empty() {
            GateTree::ROOT
        } else {
            let refs: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
            self.session.tree(si).find(&refs).ok_or_else(|| format!("population '{}' does not exist in this sample", path.join(" ▸ ")))?
        };
        self.session.population_values(si, node, channel)
    }

    fn ana_label(&self) -> String {
        let name = self.session.samples()[self.ana.sample].name.clone();
        if self.ana.restrict.is_empty() {
            name
        } else {
            format!("{name} · {}", self.ana.restrict.join(" ▸ "))
        }
    }

    pub fn run_cell_cycle(&mut self) {
        self.ana_sync();
        let ch = self.ana.cycle_channel.clone();
        let result = self.ana_values(&ch).and_then(|v| {
            let opt = CycleOptions { mu_g1_hint: if self.ana.use_hint && self.ana.mu_hint > 0.0 { Some(self.ana.mu_hint) } else { None }, ..Default::default() };
            cell_cycle(&v, if self.ana.dj_fox { CycleModel::DeanJettFox } else { CycleModel::Watson }, &opt)
        });
        match result {
            Ok(r) => {
                self.status = format!("Cell cycle: G0/G1 {:.1}%, S {:.1}%, G2/M {:.1}% (R² {:.3}).", r.g1_pct, r.s_pct, r.g2_pct, r.r2);
                self.ana.cycle = Some((self.ana_label(), r));
            }
            Err(e) => {
                self.status = e;
                self.ana.cycle = None;
            }
        }
    }

    pub fn run_proliferation(&mut self) {
        self.ana_sync();
        let ch = self.ana.prolif_channel.clone();
        let g = self.ana.generations;
        match self.ana_values(&ch).and_then(|v| proliferation(&v, g, None)) {
            Ok(r) => {
                self.status = format!("Proliferation: {} generations, PI {:.2}, DI {:.2}, {:.1}% divided.", r.generations.len(), r.pi, r.di, r.pct_divided);
                self.ana.prolif = Some((self.ana_label(), r));
            }
            Err(e) => {
                self.status = e;
                self.ana.prolif = None;
            }
        }
    }

    pub fn run_apoptosis(&mut self) {
        self.ana_sync();
        let (a_ch, p_ch) = (self.ana.annexin_channel.clone(), self.ana.pi_channel.clone());
        if a_ch == p_ch {
            self.status = "Choose two different channels for Annexin V and the viability dye.".into();
            return;
        }
        let res = self.ana_values(&a_ch).and_then(|a| self.ana_values(&p_ch).map(|p| (a, p))).and_then(|(a, p)| {
            let r = apoptosis(&a, &p, self.ana.cofactor)?;
            // an even stride through the finite events, at most 5000 points, for the picture
            let pts: Vec<(f64, f64)> = a.iter().zip(&p).filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| (*x, *y)).collect();
            let step = (pts.len() / 5000).max(1);
            Ok((r, pts.into_iter().step_by(step).collect::<Vec<_>>()))
        });
        match res {
            Ok((r, pts)) => {
                let q = r.quadrants;
                self.status = format!("Apoptosis: live {:.1}%, early {:.1}%, late {:.1}%, necrotic {:.1}%.", q.live, q.early, q.late, q.necrotic);
                self.ana.apop = Some((self.ana_label(), r, pts));
            }
            Err(e) => {
                self.status = e;
                self.ana.apop = None;
            }
        }
    }

    /// Left panel shared by the three screens: sample, population, channels, options, run button.
    fn ana_panel(&mut self, ui: &mut egui::Ui, kind: Kind) {
        let n = self.session.samples().len();
        let (title, blurb) = match kind {
            Kind::Cycle => ("Cell cycle", "Fits the DNA-content histogram of a population with G0/G1 and G2/M peaks (G2/M at twice the G1 position) and an S-phase plateau between them."),
            Kind::Prolif => ("Proliferation", "Fits a dye-dilution histogram with one peak per generation, spaced one halving apart, and reports the division and proliferation indices."),
            Kind::Apoptosis => ("Apoptosis", "Splits Annexin V against a viability dye at the density valley of each axis into live, early apoptotic, late apoptotic and necrotic fractions."),
        };
        ui.label(RichText::new(title).strong().size(16.0));
        theme::caption(ui, blurb);
        theme::section(ui, "Data");
        let cur = self.session.samples()[self.ana.sample].name.clone();
        let mut chosen = self.ana.sample;
        egui::ComboBox::from_id_salt("ana_sample").width(260.0).selected_text(cur).show_ui(ui, |ui| {
            for i in 0..n {
                if ui.selectable_label(i == chosen, self.session.samples()[i].name.clone()).clicked() {
                    chosen = i;
                }
            }
        });
        if chosen != self.ana.sample {
            self.ana.sample = chosen;
            self.ana.restrict.clear();
        }
        let si = self.ana.sample;
        let paths: Vec<Vec<String>> = self.session.tree(si).self_and_descendants(GateTree::ROOT).unwrap_or_default().into_iter().skip(1).filter_map(|id| self.session.tree(si).path(id).ok()).collect();
        let cur = if self.ana.restrict.is_empty() { "All events".to_string() } else { self.ana.restrict.join(" ▸ ") };
        egui::ComboBox::from_id_salt("ana_pop").width(260.0).selected_text(format!("Population: {cur}")).show_ui(ui, |ui| {
            if ui.selectable_label(self.ana.restrict.is_empty(), "All events").clicked() {
                self.ana.restrict.clear();
            }
            for p in paths {
                if ui.selectable_label(self.ana.restrict == p, p.join(" ▸ ")).clicked() {
                    self.ana.restrict = p;
                }
            }
        });
        let chans = self.session.sample(si).channels.clone();
        let pick = |ui: &mut egui::Ui, id: &str, label: &str, cur: &mut String| {
            ui.horizontal(|ui| {
                ui.label(label);
                egui::ComboBox::from_id_salt(id.to_string()).width(170.0).selected_text(cur.clone()).show_ui(ui, |ui| {
                    for c in &chans {
                        if ui.selectable_label(c == cur, c.as_str()).clicked() {
                            *cur = c.clone();
                        }
                    }
                });
            });
        };
        match kind {
            Kind::Cycle => {
                pick(ui, "cc_chan", "DNA channel", &mut self.ana.cycle_channel);
                theme::section(ui, "Model");
                ui.radio_value(&mut self.ana.dj_fox, true, "Dean–Jett–Fox (G2/M width free)");
                ui.radio_value(&mut self.ana.dj_fox, false, "Watson pragmatic (G2/M width = 2 × G1)");
                ui.checkbox(&mut self.ana.use_hint, "Start from a G0/G1 peak position");
                if self.ana.use_hint {
                    ui.add(egui::DragValue::new(&mut self.ana.mu_hint).speed(100.0).range(0.0..=1e9));
                }
            }
            Kind::Prolif => {
                pick(ui, "pf_chan", "Dye channel", &mut self.ana.prolif_channel);
                ui.horizontal(|ui| {
                    ui.label("Most generations");
                    ui.add(egui::DragValue::new(&mut self.ana.generations).range(2..=12));
                });
            }
            Kind::Apoptosis => {
                pick(ui, "ap_ann", "Annexin V", &mut self.ana.annexin_channel);
                pick(ui, "ap_pi", "Viability dye", &mut self.ana.pi_channel);
                ui.horizontal(|ui| {
                    ui.label("arcsinh cofactor");
                    ui.add(egui::DragValue::new(&mut self.ana.cofactor).range(0.0..=100_000.0).speed(1.0));
                });
                theme::caption(ui, "Thresholds are found on the arcsinh scale and counted on the raw values. 0 = raw.");
            }
        }
        ui.add_space(10.0);
        if ui.add(egui::Button::new(RichText::new("Run").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(260.0, 32.0))).clicked() {
            match kind {
                Kind::Cycle => self.run_cell_cycle(),
                Kind::Prolif => self.run_proliferation(),
                Kind::Apoptosis => self.run_apoptosis(),
            }
        }
    }

    fn ana_frame(&mut self, ui: &mut egui::Ui, kind: Kind, body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load a sample first.").color(theme::MUTED)));
            return;
        }
        self.ana_sync();
        egui::SidePanel::left(format!("ana_cfg_{}", kind as u8)).resizable(false).exact_width(300.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.ana_panel(ui, kind));
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| body(self, ui));
        });
    }

    pub(super) fn cell_cycle_screen(&mut self, ui: &mut egui::Ui) {
        self.ana_frame(ui, Kind::Cycle, |app, ui| {
            let Some((label, r)) = app.ana.cycle.as_ref() else {
                ui.label(RichText::new("Pick the DNA channel and press Run.").color(theme::MUTED));
                return;
            };
            ui.label(RichText::new(format!("{label} · {} events fitted", r.n)).strong());
            let rect = plot_area(ui, 300.0);
            let (x, hist) = (&r.x, &r.hist);
            let ymax = hist.iter().cloned().fold(1.0, f64::max) * 1.08;
            let frame = Frame::new(rect, x[0], *x.last().unwrap(), 0.0, ymax);
            frame.draw_axes(ui);
            frame.bars(ui, x, hist, Color32::from_rgba_unmultiplied(0x60, 0x80, 0xA0, 150));
            frame.line(ui, x, &r.g1, G1, 1.6);
            frame.line(ui, x, &r.s, S_PHASE, 1.6);
            frame.line(ui, x, &r.g2, G2, 1.6);
            frame.line(ui, x, &r.total, FIT, 1.8);
            for (b, name) in [(r.boundaries[1], "G0/G1 | S"), (r.boundaries[2], "S | G2/M")] {
                frame.vline(ui, b, Color32::from_gray(150), name);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                phase_card(ui, "G0/G1", r.g1_pct, G1, Some(r.cv_g1));
                phase_card(ui, "S", r.s_pct, S_PHASE, None);
                phase_card(ui, "G2/M", r.g2_pct, G2, Some(r.cv_g2));
            });
            ui.add_space(6.0);
            ui.label(format!("G0/G1 peak {:.0} · G2/M peak {:.0} · R² {:.4} · RMSE {:.2}", r.mu_g1, r.mu_g2, r.r2, r.rmse));
            if r.r2 < 0.8 {
                ui.label(RichText::new(format!("Poor fit (R² {:.2}). This histogram does not look like a DNA-content distribution with a G0/G1 and a G2/M peak, so the phase percentages are not reliable. Check the channel, gate single cells first, or place the G0/G1 peak by hand.", r.r2)).color(theme::WARN));
            }
            if ui.button("Copy results").clicked() {
                let t = format!("sample\t{label}\nG0/G1 %\t{:.2}\nS %\t{:.2}\nG2/M %\t{:.2}\nCV G0/G1 %\t{:.2}\nCV G2/M %\t{:.2}\nR2\t{:.4}\nevents\t{}\n", r.g1_pct, r.s_pct, r.g2_pct, r.cv_g1, r.cv_g2, r.r2, r.n);
                ui.ctx().copy_text(t);
            }
        });
    }

    pub(super) fn proliferation_screen(&mut self, ui: &mut egui::Ui) {
        self.ana_frame(ui, Kind::Prolif, |app, ui| {
            let Some((label, r)) = app.ana.prolif.as_ref() else {
                ui.label(RichText::new("Pick the dye channel and press Run.").color(theme::MUTED));
                return;
            };
            ui.label(RichText::new(format!("{label} · {} events fitted", r.n)).strong());
            let rect = plot_area(ui, 300.0);
            let ymax = r.hist.iter().cloned().fold(1.0, f64::max) * 1.08;
            let frame = Frame::new(rect, r.x[0], *r.x.last().unwrap(), 0.0, ymax);
            frame.draw_axes(ui);
            frame.bars(ui, &r.x, &r.hist, Color32::from_rgba_unmultiplied(0x60, 0x80, 0xA0, 150));
            frame.line(ui, &r.x, &r.total, FIT, 1.2);
            for (k, c) in r.curves.iter().enumerate() {
                frame.line(ui, &r.x, c, GEN[k % GEN.len()], 1.5);
            }
            ui.label(RichText::new("log10 of the dye signal; generation 0 (undivided) is the brightest peak on the right.").size(11.0).color(theme::MUTED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                big_number(ui, "Division index", format!("{:.2}", r.di));
                big_number(ui, "Proliferation index", format!("{:.2}", r.pi));
                big_number(ui, "% divided", format!("{:.1}", r.pct_divided));
            });
            ui.add_space(8.0);
            egui::Grid::new("gen_tbl").num_columns(3).spacing([24.0, 4.0]).show(ui, |ui| {
                for h in ["Generation", "Cells", "% of cells"] {
                    ui.label(RichText::new(h).color(theme::MUTED));
                }
                ui.end_row();
                for g in &r.generations {
                    ui.label(RichText::new(format!("● {}", g.gen)).color(GEN[g.gen % GEN.len()]));
                    ui.label(format!("{:.0}", g.count));
                    ui.label(format!("{:.2}", g.pct));
                    ui.end_row();
                }
            });
            theme::caption(ui, "Indices use the precursor cohort of each generation (cells ÷ 2^generation), so they are not biased by the doubling in cell number.");
            if ui.button("Copy results").clicked() {
                let mut t = format!("sample\t{label}\nDI\t{:.3}\nPI\t{:.3}\n% divided\t{:.2}\n", r.di, r.pi, r.pct_divided);
                for g in &r.generations {
                    t += &format!("generation {}\t{:.0}\t{:.2}\n", g.gen, g.count, g.pct);
                }
                ui.ctx().copy_text(t);
            }
        });
    }

    pub(super) fn apoptosis_screen(&mut self, ui: &mut egui::Ui) {
        self.ana_frame(ui, Kind::Apoptosis, |app, ui| {
            let cof = app.ana.cofactor;
            let (a_ch, p_ch) = (app.ana.annexin_channel.clone(), app.ana.pi_channel.clone());
            let Some((label, r, pts)) = app.ana.apop.as_ref() else {
                ui.label(RichText::new("Pick the two channels and press Run.").color(theme::MUTED));
                return;
            };
            ui.label(RichText::new(format!("{label} · {} events", r.n)).strong());
            let rect = plot_area(ui, 380.0);
            let tf = |v: f64| if cof > 0.0 { (v / cof).asinh() } else { v };
            let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
            for (x, y) in pts {
                let (a, b) = (tf(*x), tf(*y));
                x0 = x0.min(a);
                x1 = x1.max(a);
                y0 = y0.min(b);
                y1 = y1.max(b);
            }
            let frame = Frame::new(rect, x0, x1.max(x0 + 1e-9), y0, y1.max(y0 + 1e-9));
            frame.draw_axes(ui);
            let (tx, ty) = (tf(r.annexin_threshold), tf(r.pi_threshold));
            let painter = ui.painter_at(rect);
            for (x, y) in pts {
                let (a, b) = (*x >= r.annexin_threshold, *y >= r.pi_threshold);
                let c = match (a, b) {
                    (false, false) => Color32::from_rgba_unmultiplied(0x4C, 0xAF, 0x50, 110),
                    (true, false) => Color32::from_rgba_unmultiplied(0xF5, 0xA6, 0x23, 110),
                    (true, true) => Color32::from_rgba_unmultiplied(0xE0, 0x55, 0x55, 110),
                    (false, true) => Color32::from_rgba_unmultiplied(0x9B, 0x59, 0xB6, 110),
                };
                painter.circle_filled(frame.to_screen(tf(*x), tf(*y)), 1.3, c);
            }
            frame.vline(ui, tx, Color32::from_gray(200), "");
            frame.hline(ui, ty, Color32::from_gray(200));
            let q = r.quadrants;
            for (txt, fx, fy) in [("Necrotic", 0.12, 0.08), ("Late apoptotic", 0.88, 0.08), ("Live", 0.12, 0.88), ("Early apoptotic", 0.88, 0.88)] {
                let pct = match txt {
                    "Necrotic" => q.necrotic,
                    "Late apoptotic" => q.late,
                    "Live" => q.live,
                    _ => q.early,
                };
                painter.text(Pos2::new(rect.left() + fx * rect.width(), rect.top() + fy * rect.height()), Align2::CENTER_CENTER, format!("{txt}\n{pct:.1}%"), FontId::proportional(12.0), Color32::WHITE);
            }
            ui.label(RichText::new(format!("x: {a_ch}   y: {p_ch}   (arcsinh display, {} of {} events drawn)", pts.len(), r.n)).size(11.0).color(theme::MUTED));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                phase_card(ui, "Live", q.live, G1, None);
                phase_card(ui, "Early apoptotic", q.early, S_PHASE, None);
                phase_card(ui, "Late apoptotic", q.late, G2, None);
                phase_card(ui, "Necrotic", q.necrotic, Color32::from_rgb(0x9B, 0x59, 0xB6), None);
            });
            ui.add_space(6.0);
            ui.label(format!("Thresholds (raw units): {a_ch} {:.1} · {p_ch} {:.1}", r.annexin_threshold, r.pi_threshold));
            if ui.button("Copy results").clicked() {
                let t = format!("sample\t{label}\nlive %\t{:.2}\nearly apoptotic %\t{:.2}\nlate apoptotic %\t{:.2}\nnecrotic %\t{:.2}\nannexin threshold\t{:.4}\nviability threshold\t{:.4}\nevents\t{}\n", q.live, q.early, q.late, q.necrotic, r.annexin_threshold, r.pi_threshold, r.n);
                ui.ctx().copy_text(t);
            }
        });
    }
}

fn big_number(ui: &mut egui::Ui, label: &str, value: String) {
    egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).inner_margin(egui::Margin::symmetric(16.0, 10.0)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(label).size(11.0).color(theme::MUTED));
            ui.label(RichText::new(value).strong().size(22.0));
        });
    });
}

fn phase_card(ui: &mut egui::Ui, name: &str, pct: f64, colour: Color32, cv: Option<f64>) {
    egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).inner_margin(egui::Margin::symmetric(16.0, 10.0)).show(ui, |ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(format!("● {name}")).size(12.0).color(colour));
            ui.label(RichText::new(format!("{pct:.1}%")).strong().size(22.0));
            if let Some(cv) = cv {
                ui.label(RichText::new(format!("CV {cv:.1}%")).size(11.0).color(theme::MUTED));
            }
        });
    });
}

fn plot_area(ui: &mut egui::Ui, height: f32) -> Rect {
    let w = ui.available_width().min(900.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, height), Sense::hover());
    ui.painter().rect_filled(rect, 6.0, theme::INPUT);
    rect
}

/// Data-to-screen mapping for one plot, with a margin for the axis labels.
struct Frame {
    rect: Rect,
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Frame {
    fn new(outer: Rect, x0: f64, x1: f64, y0: f64, y1: f64) -> Frame {
        Frame { rect: Rect::from_min_max(Pos2::new(outer.left() + 46.0, outer.top() + 10.0), Pos2::new(outer.right() - 12.0, outer.bottom() - 24.0)), x0, x1, y0, y1 }
    }
    fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        let fx = ((x - self.x0) / (self.x1 - self.x0)) as f32;
        let fy = ((y - self.y0) / (self.y1 - self.y0)) as f32;
        Pos2::new(self.rect.left() + fx * self.rect.width(), self.rect.bottom() - fy * self.rect.height())
    }
    fn draw_axes(&self, ui: &egui::Ui) {
        let p = ui.painter();
        let grid = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 20));
        for k in 0..=4 {
            let f = k as f32 / 4.0;
            let (y, x) = (self.rect.bottom() - f * self.rect.height(), self.rect.left() + f * self.rect.width());
            p.line_segment([Pos2::new(self.rect.left(), y), Pos2::new(self.rect.right(), y)], grid);
            p.text(Pos2::new(self.rect.left() - 4.0, y), Align2::RIGHT_CENTER, short(self.y0 + (self.y1 - self.y0) * f as f64), FontId::proportional(10.0), theme::MUTED);
            p.text(Pos2::new(x, self.rect.bottom() + 3.0), Align2::CENTER_TOP, short(self.x0 + (self.x1 - self.x0) * f as f64), FontId::proportional(10.0), theme::MUTED);
        }
        p.rect_stroke(self.rect, 0.0, Stroke::new(1.0_f32, Color32::from_gray(70)));
    }
    fn bars(&self, ui: &egui::Ui, x: &[f64], y: &[f64], colour: Color32) {
        let p = ui.painter_at(self.rect);
        let w = ((self.rect.width() / x.len() as f32) - 0.5).max(1.0);
        for (xv, yv) in x.iter().zip(y) {
            let top = self.to_screen(*xv, *yv);
            let base = self.to_screen(*xv, self.y0);
            p.rect_filled(Rect::from_min_max(Pos2::new(top.x - w / 2.0, top.y), Pos2::new(top.x + w / 2.0, base.y)), 0.0, colour);
        }
    }
    fn line(&self, ui: &egui::Ui, x: &[f64], y: &[f64], colour: Color32, width: f32) {
        let pts: Vec<Pos2> = x.iter().zip(y).map(|(a, b)| self.to_screen(*a, *b)).collect();
        ui.painter_at(self.rect).add(Shape::line(pts, Stroke::new(width, colour)));
    }
    fn vline(&self, ui: &egui::Ui, x: f64, colour: Color32, label: &str) {
        if x < self.x0 || x > self.x1 {
            return;
        }
        let (a, b) = (self.to_screen(x, self.y0), self.to_screen(x, self.y1));
        let p = ui.painter_at(self.rect);
        p.line_segment([a, b], Stroke::new(1.0_f32, colour));
        if !label.is_empty() {
            p.text(Pos2::new(a.x + 3.0, self.rect.top() + 2.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), colour);
        }
    }
    fn hline(&self, ui: &egui::Ui, y: f64, colour: Color32) {
        if y < self.y0 || y > self.y1 {
            return;
        }
        let (a, b) = (self.to_screen(self.x0, y), self.to_screen(self.x1, y));
        ui.painter_at(self.rect).line_segment([a, b], Stroke::new(1.0_f32, colour));
    }
}

fn short(v: f64) -> String {
    if v.abs() >= 10_000.0 {
        format!("{:.0}k", v / 1000.0)
    } else if v.abs() >= 100.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_guess_prefers_area_channels_and_known_dyes() {
        let c: Vec<String> = ["FSC-A", "PI-W", "PI-A", "FITC-A"].iter().map(|s| s.to_string()).collect();
        assert_eq!(guess_channel(&c, &["PI", "DAPI"]).as_deref(), Some("PI-A"));
        assert_eq!(guess_channel(&c, &["Hoechst", "FITC"]).as_deref(), Some("FITC-A"), "later hints are tried in order");
        assert_eq!(guess_channel(&c, &["CFSE"]), None);
    }
}
