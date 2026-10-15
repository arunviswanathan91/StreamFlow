//! Cross-Sample (population x sample matrix, consistency, radar, MFI drift) and Classifier (PCA biplot and
//! Random Forest on per-sample population frequencies).

use super::analysis_view::{plot_area, Frame, GEN};
use super::{theme, App};
use crate::cross_sample::{self, FreqMatrix};
use crate::stats_table::pop_label;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, RichText, Sense, Shape, Stroke};
use sf_core::classify::{classify, ClassifierResult};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Matrix,
    Consistency,
    Radar,
    Drift,
}

pub struct State {
    pub sel: Vec<bool>,
    pub tab: Tab,
    pub matrix: Option<FreqMatrix>,
    pub drift_pop: usize,
    pub drift: Option<(Vec<String>, Vec<Vec<f64>>)>,
    pub use_pop: Vec<bool>,
    pub class: Option<ClassifierResult>,
    pub seed: u64,
}

impl Default for State {
    fn default() -> Self {
        State { sel: vec![], tab: Tab::Matrix, matrix: None, drift_pop: 0, drift: None, use_pop: vec![], class: None, seed: 42 }
    }
}

/// The names with the prefix they all share removed (cut at a `_` / space so no word is split), then shortened.
/// Falls back to plain shortening when stripping would leave a name empty.
fn short_all(names: &[String]) -> Vec<String> {
    let clean: Vec<String> = names.iter().map(|n| n.trim_end_matches(".fcs").trim_end_matches(".FCS").to_string()).collect();
    let mut prefix: Vec<char> = clean.first().map(|n| n.chars().collect()).unwrap_or_default();
    for n in &clean[clean.len().min(1)..] {
        let common = prefix.iter().zip(n.chars()).take_while(|(a, b)| **a == *b).count();
        prefix.truncate(common);
    }
    while prefix.last().map_or(false, |c| *c != '_' && *c != ' ') {
        prefix.pop();
    }
    let cut = prefix.len();
    clean.iter().map(|n| if clean.len() > 1 && cut > 0 && n.chars().count() > cut { short(&n.chars().skip(cut).collect::<String>()) } else { short(n) }).collect()
}

fn short(name: &str) -> String {
    let n = name.trim_end_matches(".fcs").trim_end_matches(".FCS");
    if n.chars().count() > 14 {
        format!("{}…", n.chars().take(13).collect::<String>())
    } else {
        n.to_string()
    }
}

/// Blue (low) - near-white (mean) - red (high) for a z-score.
fn heat(z: f64) -> Color32 {
    let t = (z / 2.5).clamp(-1.0, 1.0);
    let (r, g, b) = if t >= 0.0 { (255.0, 255.0 - 150.0 * t, 255.0 - 170.0 * t) } else { (255.0 + 170.0 * t, 255.0 + 100.0 * t, 255.0) };
    Color32::from_rgb(r as u8, g as u8, b as u8)
}

/// A coloured table drawn directly: `values[row][col]`, colour from `z[row][col]`.
fn heat_table(ui: &mut egui::Ui, id: &str, rows: &[String], cols: &[String], values: &[Vec<f64>], z: &[Vec<f64>], decimals: usize, cols_are_samples: bool) {
    let col_labels: Vec<String> = if cols_are_samples { short_all(cols) } else { cols.iter().map(|c| short(c)).collect() };
    let row_labels: Vec<String> = if cols_are_samples { rows.to_vec() } else { short_all(rows) };
    let (lw, cw, rh, hh) = (230.0_f32, 74.0_f32, 22.0_f32, 36.0_f32);
    egui::ScrollArea::both().id_salt(id.to_string()).auto_shrink([false, false]).show(ui, |ui| {
        let size = egui::vec2(lw + cw * cols.len() as f32, hh + rh * rows.len() as f32);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let p = ui.painter_at(rect);
        for (c, name) in col_labels.iter().enumerate() {
            p.text(Pos2::new(rect.left() + lw + cw * (c as f32 + 0.5), rect.top() + hh - 4.0), Align2::CENTER_BOTTOM, name, FontId::proportional(10.5), theme::MUTED);
        }
        for (r, name) in row_labels.iter().enumerate() {
            let y = rect.top() + hh + rh * r as f32;
            p.text(Pos2::new(rect.left() + 4.0, y + rh / 2.0), Align2::LEFT_CENTER, name, FontId::proportional(12.0), theme::TEXT);
            for c in 0..cols.len() {
                let cell = egui::Rect::from_min_size(Pos2::new(rect.left() + lw + cw * c as f32, y), egui::vec2(cw - 2.0, rh - 2.0));
                let v = values[r][c];
                if v.is_nan() {
                    p.rect_filled(cell, 2.0, Color32::from_rgb(0x20, 0x2E, 0x3E));
                    p.text(cell.center(), Align2::CENTER_CENTER, "—", FontId::proportional(11.0), theme::MUTED);
                } else {
                    let zz = z[r][c];
                    p.rect_filled(cell, 2.0, heat(zz));
                    p.text(cell.center(), Align2::CENTER_CENTER, format!("{v:.decimals$}"), FontId::monospace(11.0), Color32::from_rgb(20, 20, 20));
                }
            }
        }
    });
}

impl App {
    fn xs_sync(&mut self) {
        let n = self.session.samples().len();
        self.xs.sel.resize(n, true);
        self.cmp.groups.resize(n, String::new());
    }

    fn xs_selected(&self) -> Vec<usize> {
        (0..self.session.samples().len()).filter(|&i| self.xs.sel.get(i).copied().unwrap_or(false)).collect()
    }

    pub fn run_cross_sample(&mut self) {
        self.xs_sync();
        let sel = self.xs_selected();
        if sel.is_empty() {
            self.status = "Tick at least one sample, then Compute.".into();
            return;
        }
        let m = cross_sample::frequency_matrix(&mut self.session, &sel);
        if m.populations.is_empty() {
            self.status = "The selected samples have no gated populations. Draw gates and apply them to all samples first.".into();
            self.xs.matrix = None;
            return;
        }
        self.status = format!("{} sample(s) × {} population(s).", m.samples.len(), m.populations.len());
        self.xs.use_pop.resize(m.populations.len(), true);
        self.xs.drift_pop = self.xs.drift_pop.min(m.populations.len() - 1);
        self.xs.matrix = Some(m);
        self.run_drift();
    }

    pub fn run_drift(&mut self) {
        let Some(m) = self.xs.matrix.as_ref() else { return };
        let (samples, pop) = (m.samples.clone(), m.populations[self.xs.drift_pop.min(m.populations.len() - 1)].clone());
        let chans = cross_sample::fluorescence_channels(&self.session.sample(samples[0]).channels.clone());
        let d = cross_sample::mfi_drift(&mut self.session, &samples, &pop, &chans);
        self.xs.drift = Some((chans, d));
    }

    pub fn run_classifier(&mut self) {
        self.xs_sync();
        let sel = self.xs_selected();
        if sel.len() < 2 {
            self.status = "Need at least 2 samples with gate feature vectors.".into();
            return;
        }
        let m = cross_sample::frequency_matrix(&mut self.session, &sel);
        if m.populations.is_empty() {
            self.status = "Feature vectors are empty — gate at least one population first.".into();
            return;
        }
        self.xs.use_pop.resize(m.populations.len(), true);
        let (feats, flat) = cross_sample::feature_matrix(&m, &self.xs.use_pop);
        let groups: Vec<Option<String>> = sel.iter().map(|&i| Some(self.cmp.groups[i].trim().to_string()).filter(|g| !g.is_empty())).collect();
        match classify(&m.sample_names, &feats, &flat, &groups, self.xs.seed) {
            Ok(r) => {
                self.status = match r.oob_accuracy {
                    Some(a) => format!("Classifier: PC1+PC2 explain {:.1}%, out-of-bag accuracy {:.0}%.", 100.0 * r.var_explained.iter().sum::<f64>(), 100.0 * a),
                    None => format!("Classifier: PC1+PC2 explain {:.1}% of the variance.", 100.0 * r.var_explained.iter().sum::<f64>()),
                };
                self.xs.class = Some(r);
            }
            Err(e) => {
                self.status = e;
                self.xs.class = None;
            }
        }
        self.xs.matrix = Some(m);
    }

    fn xs_samples_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                self.xs.sel.iter_mut().for_each(|s| *s = true);
            }
            if ui.small_button("None").clicked() {
                self.xs.sel.iter_mut().for_each(|s| *s = false);
            }
        });
    }

    pub(super) fn cross_sample_screen(&mut self, ui: &mut egui::Ui) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples with gated populations first.").color(theme::MUTED)));
            return;
        }
        self.xs_sync();
        egui::SidePanel::left("xs_cfg").resizable(false).exact_width(280.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            ui.label(RichText::new("Cross-sample").strong().size(16.0));
            theme::caption(ui, "How each gated population varies from sample to sample: a frequency matrix, a consistency check, a radar and per-channel MFI drift.");
            theme::section(ui, "Samples");
            self.xs_samples_panel(ui);
            egui::ScrollArea::vertical().id_salt("xs_samples").max_height(380.0).show(ui, |ui| {
                let labels = short_all(&self.session.samples().iter().map(|s| s.name.clone()).collect::<Vec<_>>());
                for i in 0..labels.len() {
                    ui.checkbox(&mut self.xs.sel[i], labels[i].clone());
                }
            });
            ui.add_space(8.0);
            if ui.add(egui::Button::new(RichText::new("Compute").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(250.0, 32.0))).clicked() {
                self.run_cross_sample();
            }
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            let Some(m) = self.xs.matrix.clone() else {
                ui.label(RichText::new("Tick the samples and press Compute.").color(theme::MUTED));
                return;
            };
            ui.horizontal(|ui| {
                for (t, l) in [(Tab::Matrix, "Matrix"), (Tab::Consistency, "Consistency"), (Tab::Radar, "Radar"), (Tab::Drift, "MFI drift")] {
                    if theme::toggle(ui, self.xs.tab == t, l).clicked() {
                        self.xs.tab = t;
                    }
                }
            });
            ui.add_space(6.0);
            let pops: Vec<String> = m.populations.iter().map(|p| pop_label(p)).collect();
            match self.xs.tab {
                Tab::Matrix => {
                    theme::caption(ui, "Percent of all events in each sample. Colour = how far the value is from that population's mean across samples (z-score). — means the sample has no such population.");
                    let z: Vec<Vec<f64>> = m.freq.iter().map(|r| cross_sample::z_scores(r)).collect();
                    heat_table(ui, "xs_matrix", &pops, &m.sample_names, &m.freq, &z, 1, true);
                }
                Tab::Consistency => {
                    theme::caption(ui, "Mean, standard deviation (population SD) and CV of each population over the samples that have it. A CV above 50% is flagged.");
                    let rows = cross_sample::consistency(&m);
                    egui::ScrollArea::vertical().id_salt("xs_cons").auto_shrink([false, false]).show(ui, |ui| {
                        egui::Grid::new("cons").num_columns(6).spacing([22.0, 5.0]).striped(true).show(ui, |ui| {
                            for h in ["Population", "Samples", "Mean %", "SD", "CV %", ""] {
                                ui.label(RichText::new(h).color(theme::MUTED));
                            }
                            ui.end_row();
                            for c in rows {
                                ui.label(pop_label(&c.population));
                                ui.label(c.n.to_string());
                                ui.label(format!("{:.2}", c.mean));
                                ui.label(format!("{:.2}", c.sd));
                                ui.label(format!("{:.1}", c.cv));
                                if c.consistent {
                                    ui.label(RichText::new("consistent").color(theme::GOOD));
                                } else {
                                    ui.label(RichText::new("⚠ inconsistent").color(theme::GATE_RED).strong());
                                }
                                ui.end_row();
                            }
                        });
                    });
                }
                Tab::Radar => match cross_sample::radar(&m, 12) {
                    None => {
                        ui.label(RichText::new("A radar needs at least three populations across the selected samples.").color(theme::MUTED));
                    }
                    Some(r) => {
                        theme::caption(ui, "One polygon per sample; each axis is scaled so its largest value across samples reaches the rim (first 12 populations).");
                        let rect = plot_area(ui, 460.0);
                        let p = ui.painter_at(rect);
                        let (c, rad) = (rect.center() + egui::vec2(0.0, 6.0), (rect.height().min(rect.width()) / 2.0 - 56.0).max(40.0));
                        let k = r.axes.len();
                        let at = |ai: usize, f: f32| {
                            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * ai as f32 / k as f32;
                            c + egui::vec2(rad * f * a.cos(), rad * f * a.sin())
                        };
                        let grid = Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 30));
                        for ring in 1..=4 {
                            let pts: Vec<Pos2> = (0..k).map(|ai| at(ai, ring as f32 / 4.0)).collect();
                            p.add(Shape::closed_line(pts, grid));
                        }
                        for ai in 0..k {
                            p.line_segment([c, at(ai, 1.0)], grid);
                            let lp = at(ai, 1.12);
                            p.text(lp, Align2::CENTER_CENTER, short(&pop_label(&r.axes[ai])), FontId::proportional(10.5), theme::TEXT);
                        }
                        for (si, fr) in r.fractions.iter().enumerate() {
                            let col = GEN[si % GEN.len()];
                            let pts: Vec<Pos2> = (0..k).map(|ai| at(ai, fr[ai] as f32)).collect();
                            p.add(Shape::convex_polygon(pts.clone(), Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 22), Stroke::NONE));
                            p.add(Shape::closed_line(pts, Stroke::new(1.6_f32, col)));
                        }
                        ui.horizontal_wrapped(|ui| {
                            for (si, n) in short_all(&m.sample_names).iter().enumerate().take(14) {
                                ui.label(RichText::new(format!("● {n}")).size(11.0).color(GEN[si % GEN.len()]));
                            }
                        });
                    }
                },
                Tab::Drift => {
                    let cur = pops[self.xs.drift_pop.min(pops.len() - 1)].clone();
                    let mut chosen = self.xs.drift_pop;
                    ui.horizontal(|ui| {
                        ui.label("Population");
                        egui::ComboBox::from_id_salt("xs_drift_pop").width(260.0).selected_text(cur).show_ui(ui, |ui| {
                            for (i, n) in pops.iter().enumerate() {
                                if ui.selectable_label(i == chosen, n).clicked() {
                                    chosen = i;
                                }
                            }
                        });
                    });
                    if chosen != self.xs.drift_pop {
                        self.xs.drift_pop = chosen;
                        self.run_drift();
                    }
                    theme::caption(ui, "Median of each fluorescence channel in the population, per sample. Colour = z-score within the channel (a drifting instrument or staining shows as a gradient down a column).");
                    if let Some((chans, d)) = self.xs.drift.clone() {
                        let cols: Vec<Vec<f64>> = (0..chans.len()).map(|c| cross_sample::z_scores(&d.iter().map(|r| r[c]).collect::<Vec<_>>())).collect();
                        let z: Vec<Vec<f64>> = (0..d.len()).map(|r| (0..chans.len()).map(|c| cols[c][r]).collect()).collect();
                        heat_table(ui, "xs_drift", &m.sample_names, &chans, &d, &z, 0, false);
                    }
                }
            }
        });
    }

    pub(super) fn classifier_screen(&mut self, ui: &mut egui::Ui) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples with gated populations first.").color(theme::MUTED)));
            return;
        }
        self.xs_sync();
        egui::SidePanel::left("cls_cfg").resizable(false).exact_width(320.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new("Classifier").strong().size(16.0));
                theme::caption(ui, "Each sample becomes a vector of population frequencies. PCA shows how the samples relate; with two or more groups a Random Forest reports how well the groups can be told apart and which populations do it.");
                theme::section(ui, "Samples and groups");
                theme::caption(ui, "Group names are shared with Stats Comparison. A sample without a group is plotted but not used to train the forest.");
                self.xs_samples_panel(ui);
                egui::ScrollArea::vertical().id_salt("cls_samples").max_height(300.0).show(ui, |ui| {
                    let labels = short_all(&self.session.samples().iter().map(|s| s.name.clone()).collect::<Vec<_>>());
                    for i in 0..labels.len() {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut self.xs.sel[i], "");
                            ui.add(egui::TextEdit::singleline(&mut self.cmp.groups[i]).desired_width(70.0).hint_text("group"));
                            ui.label(RichText::new(&labels[i]).size(12.0));
                        });
                    }
                });
                if let Some(m) = self.xs.matrix.clone() {
                    theme::section(ui, "Populations used as features");
                    self.xs.use_pop.resize(m.populations.len(), true);
                    egui::ScrollArea::vertical().id_salt("cls_pops").max_height(160.0).show(ui, |ui| {
                        for (i, p) in m.populations.iter().enumerate() {
                            ui.checkbox(&mut self.xs.use_pop[i], pop_label(p));
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("seed");
                    ui.add(egui::DragValue::new(&mut self.xs.seed));
                });
                ui.add_space(8.0);
                if ui.add(egui::Button::new(RichText::new("Run classifier").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(280.0, 32.0))).clicked() {
                    self.run_classifier();
                }
            });
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            let Some(r) = self.xs.class.clone() else {
                ui.label(RichText::new("Assign groups (optional) and press Run classifier.").color(theme::MUTED));
                return;
            };
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                ui.label(RichText::new(format!("{} · {} samples × {} populations", r.method, r.n_samples, r.n_features)).strong());
                let rect = plot_area(ui, 380.0);
                let (mut x0, mut x1, mut y0, mut y1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
                for s in &r.scores {
                    let (a, b) = (s[0], s.get(1).copied().unwrap_or(0.0));
                    x0 = x0.min(a);
                    x1 = x1.max(a);
                    y0 = y0.min(b);
                    y1 = y1.max(b);
                }
                let (px, py) = (((x1 - x0) * 0.15).max(0.5), ((y1 - y0) * 0.15).max(0.5));
                let frame = Frame::new(rect, x0 - px, x1 + px, y0 - py, y1 + py);
                frame.draw_axes(ui);
                let p = ui.painter_at(frame.rect);
                let mut groups: Vec<String> = r.groups.iter().flatten().cloned().collect();
                groups.sort();
                groups.dedup();
                // loadings as arrows, scaled to the plot
                let span = ((x1 - x0).max(y1 - y0) / 2.0).max(1.0);
                for &j in &r.top_loading {
                    let (lx, ly) = (r.loadings[j][0], r.loadings[j].get(1).copied().unwrap_or(0.0));
                    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                    let end = frame.to_screen(cx + lx * span, cy + ly * span);
                    let o = frame.to_screen(cx, cy);
                    p.line_segment([o, end], Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(255, 255, 255, 60)));
                    p.text(end, Align2::LEFT_BOTTOM, short(&r.feature_names[j]), FontId::proportional(10.0), theme::MUTED);
                }
                let short_labels = short_all(&r.sample_names);
                for (i, s) in r.scores.iter().enumerate() {
                    let col = match &r.groups[i] {
                        Some(g) => GEN[groups.iter().position(|x| x == g).unwrap_or(0) % GEN.len()],
                        None => Color32::from_gray(160),
                    };
                    let at = frame.to_screen(s[0], s.get(1).copied().unwrap_or(0.0));
                    p.circle_filled(at, 5.0, col);
                    if r.scores.len() <= 12 {
                        p.text(at + egui::vec2(6.0, -6.0), Align2::LEFT_BOTTOM, &short_labels[i], FontId::proportional(10.0), theme::TEXT);
                    }
                }
                ui.horizontal(|ui| {
                    ui.label(format!("PC1 {:.1}%", 100.0 * r.var_explained[0]));
                    if let Some(v) = r.var_explained.get(1) {
                        ui.label(format!("· PC2 {:.1}%", 100.0 * v));
                    }
                    for (gi, g) in groups.iter().enumerate() {
                        ui.label(RichText::new(format!("● {g}")).color(GEN[gi % GEN.len()]));
                    }
                });
                if let Some(a) = r.oob_accuracy {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("Out-of-bag accuracy {:.0}%  (forest trained on {} labelled samples, scored on {})", 100.0 * a, r.rf_samples, r.oob_counted)).strong());
                    theme::caption(ui, "Each sample is predicted only by the trees that never saw it, so this is a fair estimate of how well the groups can be told apart. With few samples it is noisy, and with no real difference it can sit at or below chance.");
                    theme::section(ui, "Populations that separate the groups (permutation importance)");
                    let top = r.importance.first().map_or(1.0, |i| i.1.max(1e-9));
                    for (name, v) in &r.importance {
                        ui.horizontal(|ui| {
                            ui.add_sized([220.0, 16.0], egui::Label::new(RichText::new(name).size(12.0)));
                            let w = (240.0 * (v / top).clamp(0.0, 1.0)) as f32;
                            let (rr, _) = ui.allocate_exact_size(egui::vec2(w.max(1.0), 12.0), Sense::hover());
                            ui.painter().rect_filled(rr, 2.0, theme::ACCENT);
                            ui.label(RichText::new(format!("{v:.3}")).size(11.0).color(theme::MUTED));
                        });
                    }
                }
            });
        });
    }
}
