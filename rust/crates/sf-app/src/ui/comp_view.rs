//! Compensation module: view and edit a spillover matrix, apply it to a sample or to all samples.
//! Compensated detectors become `Comp-<detector>` channels on every axis list.

use super::{theme, App};
use crate::comp_auto::{detector_from_name, spillover_from_controls, Estimator};
use crate::sample::{matrix_to_csv, parse_matrix_csv, CompInfo};
use eframe::egui::{self, Color32, RichText};
use sf_core::compensation::Mode;

#[derive(Default)]
pub struct State {
    sample: usize,
    /// Per sample: the detector it is a single-stain control for.
    controls: Vec<Option<String>>,
    slope: bool,
    notes: Vec<String>,
    /// The matrix being edited (not applied until the user says so).
    matrix: Option<CompInfo>,
    for_sample: Option<usize>,
    non_negative: bool,
}

/// Fluorescence detectors of a sample: area channels that are not scatter, time or already compensated.
fn detectors_of(channels: &[String]) -> Vec<String> {
    channels
        .iter()
        .filter(|c| {
            let u = c.to_ascii_uppercase();
            u.ends_with("-A") && !u.contains("FSC") && !u.contains("SSC") && !u.contains("TIME")
        })
        .cloned()
        .collect()
}

impl App {
    /// Compute the matrix from the samples marked as single-stain controls (the "Compute matrix" button).
    pub fn compensation_from_controls(&mut self) -> bool {
        let n = self.session.samples().len();
        if n == 0 {
            return false;
        }
        let dets = detectors_of(self.session.sample(0).base_channels());
        if self.comp.controls.len() != n {
            self.comp.controls = (0..n).map(|i| detector_from_name(&self.session.samples()[i].name, &dets)).collect();
        }
        let controls: Vec<(usize, String)> = (0..n).filter_map(|i| self.comp.controls[i].clone().map(|d| (i, d))).collect();
        let refs: Vec<&crate::sample::LoadedSample> = (0..n).map(|i| self.session.sample(i)).collect();
        let est = if self.comp.slope { Estimator::Slope } else { Estimator::Medians };
        match spillover_from_controls(&refs, &controls, &dets, est) {
            Ok(r) => {
                self.status = format!("Computed a {0}×{0} matrix from {1} control(s); review it, then apply.", dets.len(), controls.len());
                self.comp.notes = r.notes;
                self.comp.matrix = Some(r.matrix);
                self.comp.for_sample = Some(self.comp.sample);
                true
            }
            Err(e) => {
                self.status = format!("Cannot compute: {e}");
                false
            }
        }
    }

    /// What a sample starts from: its applied matrix, else the one embedded in the file, else identity.
    fn starting_matrix(&self, si: usize) -> CompInfo {
        let s = self.session.sample(si);
        if let Some(c) = &s.comp {
            return c.clone();
        }
        if let Some(Ok(c)) = s.embedded_spillover() {
            return c;
        }
        CompInfo::identity(detectors_of(s.base_channels()))
    }

    pub(super) fn compensation(&mut self, ui: &mut egui::Ui) {
        let n = self.session.samples().len();
        if n == 0 {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples first (File ▸ Load FCS…).").color(theme::MUTED)));
            return;
        }
        if self.comp.sample >= n {
            self.comp.sample = 0;
            self.comp.matrix = None;
        }
        if self.comp.matrix.is_none() || self.comp.for_sample != Some(self.comp.sample) {
            self.comp.matrix = Some(self.starting_matrix(self.comp.sample));
            self.comp.for_sample = Some(self.comp.sample);
        }
        let si = self.comp.sample;
        egui::SidePanel::left("comp_cfg").resizable(false).exact_width(290.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Compensation").strong().size(16.0));
                theme::caption(ui, "Spillover matrix: row i = how much of detector i's signal appears in each detector. Corrected = measured × inverse(matrix).");
                theme::section(ui, "Sample");
                let mut pick = si;
                let name = self.session.samples()[si].name.clone();
                egui::ComboBox::from_id_salt("comp_sample").width(240.0).selected_text(name).show_ui(ui, |ui| {
                    for i in 0..n {
                        let nm = self.session.samples()[i].name.clone();
                        ui.selectable_value(&mut pick, i, nm);
                    }
                });
                if pick != si {
                    self.comp.sample = pick;
                    return;
                }
                let applied = self.session.sample(si).comp.clone();
                match &applied {
                    Some(c) => {
                        ui.label(RichText::new(format!("✔ Applied ({}): {} detectors", c.source, c.detectors.len())).color(theme::GOOD));
                        if c.is_identity() {
                            theme::caption(ui, "This matrix is the identity, so Comp- channels equal the raw ones.");
                        }
                    }
                    None => {
                        ui.label(RichText::new("Not compensated").color(theme::WARN));
                    }
                }
                theme::section(ui, "Matrix source");
                if ui.button("Use the matrix embedded in the file").on_hover_text("$SPILLOVER keyword").clicked() {
                    match self.session.sample(si).embedded_spillover() {
                        Some(Ok(c)) => self.comp.matrix = Some(c),
                        Some(Err(e)) => self.status = format!("The embedded matrix is unusable: {e}"),
                        None => self.status = "This file has no embedded spillover matrix.".into(),
                    }
                }
                if ui.button("Reset to identity").clicked() {
                    let det = self.comp.matrix.as_ref().map(|m| m.detectors.clone()).unwrap_or_default();
                    self.comp.matrix = Some(CompInfo::identity(det));
                }
                if ui.button("Import matrix (CSV / TSV)…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("Matrix", &["csv", "tsv", "txt"]).pick_file() {
                        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| parse_matrix_csv(&t)) {
                            Ok(m) => {
                                self.status = format!("Imported a {0}×{0} matrix from {1}", m.detectors.len(), p.display());
                                self.comp.matrix = Some(m);
                            }
                            Err(e) => self.status = format!("Could not import: {e}"),
                        }
                    }
                }
                if ui.button("Export matrix (CSV)…").clicked() {
                    if let (Some(m), Some(p)) = (&self.comp.matrix, rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name("spillover.csv").save_file()) {
                        self.status = match std::fs::write(&p, matrix_to_csv(m)) {
                            Ok(()) => format!("Saved {}", p.display()),
                            Err(e) => format!("Could not save: {e}"),
                        };
                    }
                }
                egui::CollapsingHeader::new("Compute from single-stain controls").default_open(false).show(ui, |ui| {
                    let dets = detectors_of(self.session.sample(0).base_channels());
                    if self.comp.controls.len() != n {
                        self.comp.controls = (0..n).map(|i| detector_from_name(&self.session.samples()[i].name, &dets)).collect();
                    }
                    theme::caption(ui, "Mark which samples are single-stain controls and for which detector. Each control gives one row of the matrix.");
                    egui::ScrollArea::vertical().id_salt("comp_ctrl").max_height(180.0).show(ui, |ui| {
                        for i in 0..n {
                            let nm = self.session.samples()[i].name.clone();
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(nm.trim_end_matches(".fcs")).size(11.5));
                            });
                            let cur = self.comp.controls[i].clone().unwrap_or_else(|| "—".into());
                            egui::ComboBox::from_id_salt(("ctrl", i)).width(190.0).selected_text(cur).show_ui(ui, |ui| {
                                if ui.selectable_label(self.comp.controls[i].is_none(), "—").clicked() {
                                    self.comp.controls[i] = None;
                                }
                                for d in &dets {
                                    if ui.selectable_label(self.comp.controls[i].as_deref() == Some(d.as_str()), d).clicked() {
                                        self.comp.controls[i] = Some(d.clone());
                                    }
                                }
                            });
                        }
                    });
                    ui.radio_value(&mut self.comp.slope, false, Estimator::Medians.label());
                    ui.radio_value(&mut self.comp.slope, true, Estimator::Slope.label());
                    if ui.add(egui::Button::new("Compute matrix").min_size(egui::vec2(240.0, 26.0))).clicked() {
                        self.compensation_from_controls();
                    }
                    for note in &self.comp.notes {
                        theme::caption(ui, note);
                    }
                });
                theme::section(ui, "Method");
                ui.radio_value(&mut self.comp.non_negative, false, "Standard (matrix inverse)");
                ui.radio_value(&mut self.comp.non_negative, true, "Non-negative least squares");
                theme::caption(ui, "Non-negative never produces values below zero, but is a different estimator from FlowJo's.");
                theme::section(ui, "Apply");
                let mode = if self.comp.non_negative { Mode::NonNegative } else { Mode::Standard };
                if ui.add(egui::Button::new(RichText::new("Apply to this sample").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(240.0, 30.0))).clicked() {
                    if let Some(m) = self.comp.matrix.clone() {
                        self.status = match self.session.set_compensation(si, &m, mode) {
                            Ok(()) => format!("Compensated {} — new channels Comp-… are available on every axis.", self.session.samples()[si].name),
                            Err(e) => format!("Not applied: {e}"),
                        };
                    }
                }
                if ui.add(egui::Button::new("Apply to all samples").min_size(egui::vec2(240.0, 28.0))).clicked() {
                    if let Some(m) = self.comp.matrix.clone() {
                        let failed = self.session.set_compensation_all(&m, mode);
                        self.status = if failed.is_empty() { format!("Compensated all {n} samples.") } else { format!("Compensated {} of {n}; {} could not be: {}", n - failed.len(), failed.len(), failed.join(" | ")) };
                    }
                }
                if ui.add_enabled(applied.is_some(), egui::Button::new("Remove compensation").min_size(egui::vec2(240.0, 28.0))).clicked() {
                    self.session.clear_compensation(si);
                    self.status = "Compensation removed.".into();
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(16.0))).show_inside(ui, |ui| {
            let Some(m) = self.comp.matrix.as_mut() else { return };
            let k = m.detectors.len();
            if k == 0 {
                ui.label(RichText::new("This sample has no fluorescence detectors to compensate.").color(theme::MUTED));
                return;
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Spillover matrix  ({k} × {k})")).strong());
                theme::caption(ui, "values in percent · diagonal fixed at 100%");
            });
            ui.add_space(6.0);
            egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("comp_grid").spacing([4.0, 4.0]).min_col_width(64.0).show(ui, |ui| {
                    ui.label("");
                    for d in &m.detectors {
                        ui.label(RichText::new(d.trim_end_matches("-A")).size(11.0).color(theme::MUTED));
                    }
                    ui.end_row();
                    for i in 0..k {
                        ui.label(RichText::new(m.detectors[i].trim_end_matches("-A")).size(11.0).color(theme::MUTED));
                        for j in 0..k {
                            let idx = i * k + j;
                            if i == j {
                                ui.add_sized([64.0, 24.0], egui::Label::new(RichText::new("100").monospace().color(theme::MUTED)));
                                continue;
                            }
                            let mut pct = m.spill[idx] * 100.0;
                            // heat: stronger cyan with larger spillover
                            let heat = (pct.abs() / 40.0).clamp(0.0, 1.0) as f32;
                            let bg = Color32::from_rgba_unmultiplied(0, 180, 216, (heat * 140.0) as u8);
                            let r = egui::Frame::none().fill(bg).rounding(4.0).show(ui, |ui| ui.add_sized([64.0, 22.0], egui::DragValue::new(&mut pct).speed(0.05).range(-50.0..=200.0).fixed_decimals(2))).inner;
                            if r.changed() {
                                m.spill[idx] = pct / 100.0;
                                m.source = "edited".into();
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
    }
}

impl State {
    /// The matrix currently being edited (tests).
    #[cfg(test)]
    pub fn matrix_for_test(&self) -> Option<CompInfo> {
        self.matrix.clone()
    }
}
