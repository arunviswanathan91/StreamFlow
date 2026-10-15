//! Statistics module: choose samples, a channel and statistics; get a table (copy or save as CSV).

use super::{theme, App};
use crate::stats_table::{compute, pop_label, to_csv, Metric, Table};
use eframe::egui::{self, RichText};

pub struct State {
    pub(super) selected: Vec<bool>,
    pub(super) channel: String,
    pub(super) metrics: Vec<(Metric, bool)>,
    pub(super) percentile: f64,
    pub(super) table: Option<Table>,
}

impl Default for State {
    fn default() -> Self {
        State {
            selected: Vec::new(),
            channel: String::new(),
            metrics: Metric::COMMON.iter().map(|&m| (m, matches!(m, Metric::Count | Metric::PctParent | Metric::Median | Metric::Mean))).collect(),
            percentile: 90.0,
            table: None,
        }
    }
}

impl App {
    /// Compute the table from the current selection (the Compute button). Returns the number of rows.
    pub fn compute_statistics(&mut self) -> usize {
        let n = self.session.samples().len();
        self.stats.selected.resize(n, true);
        let samples: Vec<usize> = (0..n).filter(|&i| self.stats.selected[i]).collect();
        let metrics: Vec<Metric> = self.stats.metrics.iter().filter(|(_, on)| *on).map(|(m, _)| *m).collect();
        if samples.is_empty() || metrics.is_empty() {
            self.status = "Select at least one sample and one statistic.".into();
            return 0;
        }
        if self.stats.channel.is_empty() {
            self.stats.channel = self.session.samples()[samples[0]].channels.first().cloned().unwrap_or_default();
        }
        let ch = self.stats.channel.clone();
        let t = compute(&mut self.session, &samples, &metrics, Some(&ch));
        let rows = t.rows.len();
        self.status = format!("{rows} rows computed.");
        self.stats.table = Some(t);
        rows
    }

    pub(super) fn statistics(&mut self, ui: &mut egui::Ui) {
        let n = self.session.samples().len();
        self.stats.selected.resize(n, true);
        if n == 0 {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples first (File ▸ Load FCS…).").color(theme::MUTED)));
            return;
        }
        egui::SidePanel::left("stats_cfg").resizable(false).exact_width(270.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Statistics").strong().size(16.0));
                theme::caption(ui, "One row per sample and population.");
                theme::section(ui, "Samples");
                ui.horizontal(|ui| {
                    if ui.small_button("All").clicked() {
                        self.stats.selected.iter_mut().for_each(|s| *s = true);
                    }
                    if ui.small_button("None").clicked() {
                        self.stats.selected.iter_mut().for_each(|s| *s = false);
                    }
                });
                egui::ScrollArea::vertical().id_salt("stats_samples").max_height(190.0).show(ui, |ui| {
                    for i in 0..n {
                        let name = self.session.samples()[i].name.clone();
                        ui.checkbox(&mut self.stats.selected[i], name);
                    }
                });
                theme::section(ui, "Channel");
                let channels = self.session.samples()[0].channels.clone();
                if self.stats.channel.is_empty() || !channels.contains(&self.stats.channel) {
                    self.stats.channel = channels.first().cloned().unwrap_or_default();
                }
                egui::ComboBox::from_id_salt("stats_chan").width(220.0).selected_text(self.stats.channel.clone()).show_ui(ui, |ui| {
                    for c in &channels {
                        ui.selectable_value(&mut self.stats.channel, c.clone(), c);
                    }
                });
                theme::section(ui, "Statistics");
                for (m, on) in self.stats.metrics.iter_mut() {
                    ui.checkbox(on, m.label());
                }
                ui.horizontal(|ui| {
                    let has = self.stats.metrics.iter().any(|(m, _)| matches!(m, Metric::Percentile(_)));
                    let mut add = has;
                    ui.checkbox(&mut add, "Percentile");
                    ui.add(egui::DragValue::new(&mut self.stats.percentile).range(0.0..=100.0).speed(0.5));
                    if add && !has {
                        self.stats.metrics.push((Metric::Percentile(self.stats.percentile), true));
                    }
                    if !add && has {
                        self.stats.metrics.retain(|(m, _)| !matches!(m, Metric::Percentile(_)));
                    }
                    for (m, _) in self.stats.metrics.iter_mut() {
                        if let Metric::Percentile(p) = m {
                            *p = self.stats.percentile;
                        }
                    }
                });
                ui.add_space(10.0);
                if ui.add(egui::Button::new(RichText::new("Compute").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(220.0, 32.0))).clicked() {
                    self.compute_statistics();
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            let Some(t) = &self.stats.table else {
                ui.centered_and_justified(|ui| ui.label(RichText::new("Choose samples and statistics, then press Compute.").color(theme::MUTED)));
                return;
            };
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} rows", t.rows.len())).strong());
                if let Some(c) = &t.channel {
                    theme::caption(ui, format!("channel {c}"));
                }
                if ui.button("Copy CSV").clicked() {
                    ui.ctx().copy_text(to_csv(t));
                    self.status = "Table copied to the clipboard.".into();
                }
                if ui.button("Save CSV…").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("CSV", &["csv"]).set_file_name("statistics.csv").save_file() {
                        self.status = match std::fs::write(&p, to_csv(t)) {
                            Ok(()) => format!("Saved {}", p.display()),
                            Err(e) => format!("Could not save: {e}"),
                        };
                    }
                }
            });
            if !t.notes.is_empty() {
                egui::CollapsingHeader::new(RichText::new(format!("⚠ {} note(s)", t.notes.len())).color(theme::WARN)).show(ui, |ui| {
                    for n in t.notes.iter().take(50) {
                        theme::caption(ui, n);
                    }
                });
            }
            ui.add_space(6.0);
            let (w_s, w_p, w_m, row_h) = (150.0, 240.0, 92.0, 22.0);
            egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized([w_s, row_h], egui::Label::new(RichText::new("Sample").strong()));
                    ui.add_sized([w_p, row_h], egui::Label::new(RichText::new("Population").strong()));
                    for m in &t.metrics {
                        ui.add_sized([w_m, row_h], egui::Label::new(RichText::new(m.label()).strong()));
                    }
                });
                theme::rule(ui);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, t.rows.len(), |ui, range| {
                    for i in range {
                        let r = &t.rows[i];
                        ui.horizontal(|ui| {
                            ui.add_sized([w_s, row_h], egui::Label::new(RichText::new(&r.sample).size(12.0)).truncate());
                            ui.add_sized([w_p, row_h], egui::Label::new(RichText::new(pop_label(&r.population)).size(12.0)).truncate());
                            for (m, v) in t.metrics.iter().zip(&r.values) {
                                let text = match (v, m) {
                                    (None, _) => "—".to_string(),
                                    (Some(v), Metric::Count) => format!("{v:.0}"),
                                    (Some(v), Metric::PctParent | Metric::PctTotal | Metric::Cv | Metric::RobustCv) => format!("{v:.2}"),
                                    (Some(v), _) => format!("{v:.1}"),
                                };
                                ui.add_sized([w_m, row_h], egui::Label::new(RichText::new(text).monospace().size(12.0)));
                            }
                        });
                    }
                });
            });
        });
    }
}
