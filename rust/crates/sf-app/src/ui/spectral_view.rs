//! Spectral Unmixing screen: assign controls, build and inspect signatures, unmix samples.

use super::{theme, App};
use crate::spectral_app::{build_model, guess_role, spectral_detectors, unmix_sample, Reference, Role, SpectralModel};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke};
use sf_core::spectral::Method;

const COLOURS: [Color32; 10] = [
    Color32::from_rgb(0x1F, 0x77, 0xB4),
    Color32::from_rgb(0xFF, 0x7F, 0x0E),
    Color32::from_rgb(0x2C, 0xA0, 0x2C),
    Color32::from_rgb(0xD6, 0x27, 0x28),
    Color32::from_rgb(0x94, 0x67, 0xBD),
    Color32::from_rgb(0x8C, 0x56, 0x4B),
    Color32::from_rgb(0xE3, 0x77, 0xC2),
    Color32::from_rgb(0x7F, 0x7F, 0x7F),
    Color32::from_rgb(0xBC, 0xBD, 0x22),
    Color32::from_rgb(0x17, 0xBE, 0xCF),
];

#[derive(Clone, PartialEq)]
pub enum Assign {
    None,
    Single(String),
    Unstained,
}

pub struct State {
    pub roles: Vec<Assign>,
    pub model: Option<SpectralModel>,
    pub wls: bool,
    pub use_af: bool,
    pub af_k: usize,
    pub unmix_sel: Vec<bool>,
    pub log: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        State { roles: vec![], model: None, wls: true, use_af: true, af_k: 4, unmix_sel: vec![], log: vec![] }
    }
}

impl App {
    /// Fill the control table from file names (`fitc alone.fcs` = single stain FITC, `unstained.fcs` = unstained).
    pub fn spectral_guess_roles(&mut self) {
        let n = self.session.samples().len();
        self.spec.roles = (0..n)
            .map(|i| match guess_role(&self.session.samples()[i].name) {
                Role::Single(l) => Assign::Single(l.to_uppercase()),
                Role::Unstained => Assign::Unstained,
                Role::Other => Assign::None,
            })
            .collect();
    }

    /// Build the signature model from the control table. Returns an error message, if any.
    pub fn spectral_build(&mut self) -> Result<(), String> {
        let n = self.session.samples().len();
        let refs: Vec<Reference> = (0..n).filter_map(|i| if let Assign::Single(l) = &self.spec.roles[i] { Some(Reference { sample: i, label: l.clone() }) } else { None }).collect();
        if refs.len() < 2 {
            return Err("assign at least two samples as single-stain controls".into());
        }
        let unstained = (0..n).find(|&i| self.spec.roles[i] == Assign::Unstained);
        let detectors = spectral_detectors(self.session.sample(refs[0].sample));
        let m = build_model(&self.session, &refs, unstained, &detectors, if unstained.is_some() { self.spec.af_k } else { 0 }, 42)?;
        self.status = format!("Built {} signatures over {} detectors (condition number {:.1}).", m.f(), m.d(), m.qc.condition);
        self.spec.model = Some(m);
        Ok(())
    }

    pub fn spectral_unmix(&mut self, samples: &[usize]) {
        let Some(model) = self.spec.model.clone() else { return };
        let method = if self.spec.wls { Method::Wls } else { Method::Ols };
        let mut lines = Vec::new();
        for &i in samples {
            match unmix_sample(&mut self.session, i, &model, method, self.spec.use_af && model.af_library.is_some()) {
                Ok(r) => lines.push(format!("{}: {} events, median residual {:.1}{}", r.sample, r.n_events, r.median_residual, if r.used_af { ", per-cell autofluorescence" } else { "" })),
                Err(e) => lines.push(format!("{}: {e}", self.session.samples()[i].name)),
            }
        }
        self.status = format!("Unmixed {} sample(s). New channels are named Unmix-… and appear on every axis.", lines.len());
        self.spec.log = lines;
    }

    pub(super) fn spectral(&mut self, ui: &mut egui::Ui) {
        let n = self.session.samples().len();
        if n == 0 {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load single-stain controls, an unstained tube and your samples first.").color(theme::MUTED)));
            return;
        }
        if self.spec.roles.len() != n {
            self.spectral_guess_roles();
        }
        self.spec.unmix_sel.resize(n, true);
        egui::SidePanel::left("spec_cfg").resizable(false).exact_width(330.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Spectral unmixing").strong().size(16.0));
                theme::caption(ui, "Each single-stain control gives one fluorochrome's spectrum across the detectors. Unmixing then solves every event for the abundance of each fluorochrome.");
                theme::section(ui, "Controls");
                if ui.small_button("Guess from file names").clicked() {
                    self.spectral_guess_roles();
                }
                egui::ScrollArea::vertical().id_salt("spec_roles").max_height(260.0).show(ui, |ui| {
                    for i in 0..n {
                        let name = self.session.samples()[i].name.clone();
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name.trim_end_matches(".fcs")).size(12.0));
                        });
                        ui.horizontal(|ui| {
                            let kind = match &self.spec.roles[i] {
                                Assign::None => "—",
                                Assign::Single(_) => "Single stain",
                                Assign::Unstained => "Unstained",
                            };
                            egui::ComboBox::from_id_salt(("role", i)).width(110.0).selected_text(kind).show_ui(ui, |ui| {
                                if ui.selectable_label(self.spec.roles[i] == Assign::None, "—").clicked() {
                                    self.spec.roles[i] = Assign::None;
                                }
                                if ui.selectable_label(matches!(self.spec.roles[i], Assign::Single(_)), "Single stain").clicked() && !matches!(self.spec.roles[i], Assign::Single(_)) {
                                    self.spec.roles[i] = Assign::Single(String::new());
                                }
                                if ui.selectable_label(self.spec.roles[i] == Assign::Unstained, "Unstained").clicked() {
                                    self.spec.roles[i] = Assign::Unstained;
                                }
                            });
                            if let Assign::Single(l) = &mut self.spec.roles[i] {
                                ui.add(egui::TextEdit::singleline(l).hint_text("fluorochrome").desired_width(110.0));
                            }
                        });
                    }
                });
                theme::section(ui, "Autofluorescence");
                ui.horizontal(|ui| {
                    ui.label("library clusters");
                    ui.add(egui::DragValue::new(&mut self.spec.af_k).range(0..=12));
                });
                theme::caption(ui, "Clusters the unstained tube into typical autofluorescence spectra (0 = off). Each cell is then unmixed with its best-fitting one.");
                ui.add_space(8.0);
                if ui.add(egui::Button::new(RichText::new("Build signatures").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(290.0, 32.0))).clicked() {
                    if let Err(e) = self.spectral_build() {
                        self.status = format!("Cannot build signatures: {e}");
                    }
                }
                if self.spec.model.is_some() {
                    theme::section(ui, "Unmix");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut self.spec.wls, true, "Weighted (WLS)");
                        ui.radio_value(&mut self.spec.wls, false, "Ordinary (OLS)");
                    });
                    let has_lib = self.spec.model.as_ref().map_or(false, |m| m.af_library.is_some());
                    ui.add_enabled(has_lib, egui::Checkbox::new(&mut self.spec.use_af, "Per-cell autofluorescence"));
                    ui.horizontal(|ui| {
                        if ui.small_button("All").clicked() {
                            self.spec.unmix_sel.iter_mut().for_each(|s| *s = true);
                        }
                        if ui.small_button("None").clicked() {
                            self.spec.unmix_sel.iter_mut().for_each(|s| *s = false);
                        }
                    });
                    egui::ScrollArea::vertical().id_salt("spec_unmix").max_height(150.0).show(ui, |ui| {
                        for i in 0..n {
                            let nm = self.session.samples()[i].name.clone();
                            let tag = if self.session.sample(i).unmixed { " ✔" } else { "" };
                            ui.checkbox(&mut self.spec.unmix_sel[i], format!("{nm}{tag}"));
                        }
                    });
                    if ui.add(egui::Button::new(RichText::new("Unmix selected samples").strong()).min_size(egui::vec2(290.0, 30.0))).clicked() {
                        let sel: Vec<usize> = (0..n).filter(|&i| self.spec.unmix_sel[i]).collect();
                        self.spectral_unmix(&sel);
                    }
                    if ui.button("Remove unmixed channels from selected").clicked() {
                        for i in 0..n {
                            if self.spec.unmix_sel[i] {
                                self.session.clear_unmixed(i);
                            }
                        }
                        self.status = "Unmixed channels removed.".into();
                    }
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(16.0))).show_inside(ui, |ui| {
            let Some(m) = &self.spec.model else {
                ui.centered_and_justified(|ui| ui.label(RichText::new("Assign the controls, then press “Build signatures”.").color(theme::MUTED).size(15.0)));
                return;
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(format!("{} signatures · {} detectors", m.f(), m.d())).strong().size(15.0));
                // ---- spectra
                let (w, h) = (ui.available_width().min(760.0), 230.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
                let p = ui.painter_at(rect);
                p.rect_filled(rect, 8.0, theme::PANEL_2);
                let plot = Rect::from_min_max(Pos2::new(rect.left() + 36.0, rect.top() + 12.0), Pos2::new(rect.right() - 14.0, rect.bottom() - 40.0));
                for q in 0..=4 {
                    let y = plot.bottom() - plot.height() * q as f32 / 4.0;
                    p.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(0.5_f32, Color32::from_gray(60)));
                    p.text(Pos2::new(plot.left() - 5.0, y), Align2::RIGHT_CENTER, format!("{:.2}", q as f32 / 4.0), FontId::proportional(10.0), theme::MUTED);
                }
                let d = m.d();
                let xpos = |j: usize| plot.left() + plot.width() * (j as f32 + 0.5) / d as f32;
                for (j, det) in m.detectors.iter().enumerate() {
                    p.text(Pos2::new(xpos(j), plot.bottom() + 6.0), Align2::CENTER_TOP, crate::session::short_name(det), FontId::proportional(if d > 12 { 8.0 } else { 10.0 }), theme::MUTED);
                }
                for f in 0..m.f() {
                    let pts: Vec<Pos2> = (0..d).map(|j| Pos2::new(xpos(j), plot.bottom() - plot.height() * m.m[f * d + j].clamp(0.0, 1.05) as f32)).collect();
                    p.add(egui::Shape::line(pts, Stroke::new(2.0_f32, COLOURS[f % COLOURS.len()])));
                }
                ui.horizontal_wrapped(|ui| {
                    for (f, name) in m.fluors.iter().enumerate() {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                        ui.painter().rect_filled(r, 3.0, COLOURS[f % COLOURS.len()]);
                        ui.label(name);
                        ui.add_space(8.0);
                    }
                });
                // ---- QC
                ui.add_space(10.0);
                ui.label(RichText::new("Signature quality").strong());
                let cond = m.qc.condition;
                let verdict = if !cond.is_finite() { ("cannot be unmixed (a signature is explained by the others)", theme::GATE_RED) } else if cond > 30.0 { ("poorly conditioned: noise will be amplified", theme::WARN) } else { ("well conditioned", theme::GOOD) };
                ui.horizontal(|ui| {
                    ui.label(format!("condition number {}", if cond.is_finite() { format!("{cond:.2}") } else { "∞".into() }));
                    ui.label(RichText::new(verdict.0).color(verdict.1));
                });
                let bad = m.collinear(2.0);
                if !bad.is_empty() {
                    ui.label(RichText::new(format!("⚠ similar to others (collinearity ≥ 2): {}", bad.iter().map(|(f, c)| format!("{f} ({})", if c.is_finite() { format!("{c:.1}") } else { "∞".into() })).collect::<Vec<_>>().join(", "))).color(theme::WARN));
                }
                egui::Grid::new("cos").spacing([3.0, 3.0]).show(ui, |ui| {
                    ui.label("");
                    for f in &m.fluors {
                        ui.label(RichText::new(f).size(11.0).color(theme::MUTED));
                    }
                    ui.label(RichText::new("collinearity").size(11.0).color(theme::MUTED));
                    ui.end_row();
                    let f = m.f();
                    for a in 0..f {
                        ui.label(RichText::new(&m.fluors[a]).size(11.0).color(theme::MUTED));
                        for b in 0..f {
                            let v = m.qc.cosine[a * f + b];
                            let col = if a == b { Color32::from_gray(50) } else { Color32::from_rgba_unmultiplied(214, 39, 40, (v.clamp(0.0, 1.0) * 220.0) as u8) };
                            egui::Frame::none().fill(col).rounding(4.0).inner_margin(egui::Margin::symmetric(6.0, 3.0)).show(ui, |ui| ui.label(RichText::new(format!("{v:.2}")).monospace().size(11.0)));
                        }
                        let c = m.qc.collinearity[a];
                        ui.label(RichText::new(if c.is_finite() { format!("{c:.2}") } else { "∞".into() }).monospace().size(11.0).color(if c < 2.0 { theme::GOOD } else { theme::WARN }));
                        ui.end_row();
                    }
                });
                ui.add_space(8.0);
                egui::Grid::new("siginfo").spacing([14.0, 3.0]).show(ui, |ui| {
                    for h in ["Fluorochrome", "Control", "Peak detector", "Positive", "Negative", "Saturated"] {
                        ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
                    }
                    ui.end_row();
                    for i in &m.infos {
                        ui.label(&i.fluor);
                        ui.label(RichText::new(&i.sample).size(11.0));
                        ui.label(&i.peak_detector);
                        ui.label(format!("{}", i.n_pos));
                        ui.label(format!("{}", i.n_neg));
                        ui.label(format!("{}", i.n_saturated));
                        ui.end_row();
                    }
                });
                if let Some((lib, counts)) = &m.af_library {
                    ui.add_space(6.0);
                    ui.label(format!("Autofluorescence library: {} spectra from the unstained tube (cluster sizes {:?})", lib.len() / m.d(), counts));
                }
                for r in &m.rejected {
                    ui.label(RichText::new(format!("✕ {r}")).color(theme::WARN).size(12.0));
                }
                if !self.spec.log.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Last unmixing").strong());
                    for l in &self.spec.log {
                        theme::caption(ui, l);
                    }
                }
            });
        });
    }
}
