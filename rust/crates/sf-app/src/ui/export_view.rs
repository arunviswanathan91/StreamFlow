//! Export module: statistics tables, GatingML 2.0 gates, and the workspace.

use super::{theme, App};
use crate::stats_table::{compute, to_csv, Metric};
use eframe::egui::{self, RichText};
use sf_gating::gatingml::to_gatingml;

#[derive(Default)]
pub struct State {
    sample: usize,
    /// Warnings from the last GatingML export (things that could not be written exactly).
    gml_warnings: Vec<String>,
    last_gml: Option<(String, usize)>,
}

impl App {
    pub(super) fn export_module(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none().inner_margin(egui::Margin::same(20.0)).show(ui, |ui| {
            ui.label(RichText::new("Export").strong().size(18.0));
            theme::caption(ui, "Everything is written exactly as computed; anything that cannot be represented is listed, never approximated.");
            ui.add_space(12.0);
            let n = self.session.samples().len();
            if n == 0 {
                ui.label(RichText::new("Load samples first.").color(theme::MUTED));
                return;
            }
            if self.export.sample >= n {
                self.export.sample = 0;
            }
            card(ui, "Statistics", "Count, % of parent and total, median, mean, SD, CV for every population of every sample.", |ui| {
                if ui.button("Export all samples as CSV…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name("statistics.csv").save_file() {
                        let samples: Vec<usize> = (0..n).collect();
                        let ch = self.session.samples()[0].channels.first().cloned();
                        let metrics = [Metric::Count, Metric::PctParent, Metric::PctTotal, Metric::Median, Metric::Mean, Metric::Sd, Metric::Cv];
                        let t = compute(&mut self.session, &samples, &metrics, ch.as_deref());
                        self.status = match std::fs::write(&p, to_csv(&t)) {
                            Ok(()) => format!("Saved {} rows to {} (channel statistics use {}; use the Statistics module to choose another).", t.rows.len(), p.display(), ch.unwrap_or_default()),
                            Err(e) => format!("Could not save: {e}"),
                        };
                    }
                }
            });
            card(ui, "Gates (GatingML 2.0)", "Rectangle, interval, polygon and ellipse gates on linear axes, nested; And/Or/Not gates at the top level. Opens in FlowJo, FlowKit, Cytobank, FCS Express.", |ui| {
                ui.horizontal(|ui| {
                    let name = self.session.samples()[self.export.sample].name.clone();
                    egui::ComboBox::from_id_salt("exp_sample").width(220.0).selected_text(name).show_ui(ui, |ui| {
                        for i in 0..n {
                            let nm = self.session.samples()[i].name.clone();
                            ui.selectable_value(&mut self.export.sample, i, nm);
                        }
                    });
                    if ui.button("Export…").clicked() {
                        let si = self.export.sample;
                        let ex = to_gatingml(self.session.tree(si), &Default::default());
                        if let Some(p) = rfd::FileDialog::new().add_filter("GatingML", &["xml"]).set_file_name("gates.xml").save_file() {
                            self.status = match std::fs::write(&p, &ex.xml) {
                                Ok(()) => format!("Wrote {} gate(s) to {}; {} could not be exported.", ex.exported.len(), p.display(), ex.warnings.len()),
                                Err(e) => format!("Could not save: {e}"),
                            };
                            self.export.last_gml = Some((p.display().to_string(), ex.exported.len()));
                            self.export.gml_warnings = ex.warnings;
                        }
                    }
                });
                if let Some((path, k)) = &self.export.last_gml {
                    theme::caption(ui, format!("Last export: {k} gate(s) → {path}"));
                }
                if !self.export.gml_warnings.is_empty() {
                    ui.add_space(4.0);
                    ui.label(RichText::new(format!("Not exported ({}):", self.export.gml_warnings.len())).color(theme::WARN).size(12.0));
                    egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                        for w in &self.export.gml_warnings {
                            theme::caption(ui, w);
                        }
                    });
                }
            });
            card(ui, "Workspace", "Samples, gating trees (with each gate's scales), compensation and remembered views.", |ui| {
                if ui.button("Save workspace…  (Ctrl+S)").clicked() {
                    self.save_workspace();
                }
            });
        });
    }
}

fn card(ui: &mut egui::Ui, title: &str, blurb: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::none().fill(theme::PANEL_2).rounding(10.0).inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
        ui.set_width(ui.available_width().min(760.0));
        ui.label(RichText::new(title).strong().size(14.0));
        theme::caption(ui, blurb);
        ui.add_space(6.0);
        body(ui);
    });
    ui.add_space(10.0);
}
