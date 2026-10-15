//! Stats Comparison (groups of samples, one statistic of one population, the right test chosen automatically)
//! and Kinetic (median signal against time per sample, averaged per group).

use super::analysis_view::{plot_area, Frame, GEN};
use super::{theme, App};
use crate::stats_table::{self, Metric};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, RichText};
use sf_core::htest::{compare_groups, kinetics, Comparison, KineticInput, KineticSeries};
use sf_gating::GateTree;

pub struct State {
    /// Group label per sample; empty = left out of the comparison.
    pub groups: Vec<String>,
    pub restrict: Vec<String>,
    pub metric: usize,
    pub channel: String,
    pub paired: bool,
    pub result: Option<(String, Comparison)>,
    pub k_sel: Vec<bool>,
    pub k_channel: String,
    pub k_time: String,
    pub k_bins: usize,
    pub k_result: Option<KineticRun>,
}

pub struct KineticRun {
    pub channel: String,
    pub series: Vec<KineticSeries>,
    /// Samples that could not be used, with the reason. Never dropped silently.
    pub skipped: Vec<String>,
}

impl Default for State {
    fn default() -> Self {
        State { groups: vec![], restrict: vec![], metric: 3, channel: String::new(), paired: false, result: None, k_sel: vec![], k_channel: String::new(), k_time: "Time".into(), k_bins: 100, k_result: None }
    }
}

impl App {
    fn cmp_sync(&mut self) {
        let n = self.session.samples().len();
        self.cmp.groups.resize(n, String::new());
        self.cmp.k_sel.resize(n, true);
        if n == 0 {
            return;
        }
        let chans = self.session.sample(0).channels.clone();
        let fluor: Vec<String> = chans.iter().filter(|c| !c.starts_with("FSC") && !c.starts_with("SSC") && c.as_str() != "Time" && !c.ends_with("-W") && !c.ends_with("-H")).cloned().collect();
        if self.cmp.channel.is_empty() || !chans.contains(&self.cmp.channel) {
            self.cmp.channel = fluor.first().cloned().unwrap_or_default();
        }
        if self.cmp.k_channel.is_empty() || !chans.contains(&self.cmp.k_channel) {
            self.cmp.k_channel = fluor.first().cloned().unwrap_or_default();
        }
        if !chans.contains(&self.cmp.k_time) {
            self.cmp.k_time = chans.iter().find(|c| c.eq_ignore_ascii_case("time")).cloned().unwrap_or_else(|| "Time".into());
        }
    }

    fn cmp_population_picker(&mut self, ui: &mut egui::Ui, id: &str) {
        let paths: Vec<Vec<String>> = if self.session.samples().is_empty() { vec![] } else { self.session.tree(0).self_and_descendants(GateTree::ROOT).unwrap_or_default().into_iter().skip(1).filter_map(|n| self.session.tree(0).path(n).ok()).collect() };
        let cur = if self.cmp.restrict.is_empty() { "All events".to_string() } else { self.cmp.restrict.join(" ▸ ") };
        egui::ComboBox::from_id_salt(id.to_string()).width(260.0).selected_text(format!("Population: {cur}")).show_ui(ui, |ui| {
            if ui.selectable_label(self.cmp.restrict.is_empty(), "All events").clicked() {
                self.cmp.restrict.clear();
            }
            for p in paths {
                if ui.selectable_label(self.cmp.restrict == p, p.join(" ▸ ")).clicked() {
                    self.cmp.restrict = p;
                }
            }
        });
    }

    /// Run the group comparison for the chosen population and statistic.
    pub fn run_comparison(&mut self) {
        self.cmp_sync();
        let metric = Metric::COMMON[self.cmp.metric.min(Metric::COMMON.len() - 1)];
        let channel = if metric.needs_channel() { Some(self.cmp.channel.clone()) } else { None };
        let path = self.cmp.restrict.clone();
        let mut order: Vec<String> = Vec::new();
        let mut vals: Vec<Vec<f64>> = Vec::new();
        for si in 0..self.session.samples().len() {
            let g = self.cmp.groups[si].trim().to_string();
            if g.is_empty() {
                continue;
            }
            let table = stats_table::compute(&mut self.session, &[si], &[metric], channel.as_deref());
            // a sample that lacks the population (or the channel) contributes NaN, which the comparison reports as dropped
            let v = table.rows.iter().find(|r| r.population == path).and_then(|r| r.values[0]).unwrap_or(f64::NAN);
            let gi = order.iter().position(|o| *o == g).unwrap_or_else(|| {
                order.push(g.clone());
                vals.push(Vec::new());
                order.len() - 1
            });
            vals[gi].push(v);
        }
        let raw: Vec<(String, Vec<f64>)> = order.into_iter().zip(vals).collect();
        match compare_groups(&raw, self.cmp.paired) {
            Ok(c) => {
                self.status = format!("{}: p = {:.4} ({}).", c.test, c.p, c.stars);
                let label = format!("{} · {}", stats_table::pop_label(&path), metric.label());
                self.cmp.result = Some((label, c));
            }
            Err(e) => {
                self.status = e;
                self.cmp.result = None;
            }
        }
    }

    /// Median signal against time for the selected samples.
    pub fn run_kinetics(&mut self) {
        self.cmp_sync();
        let path = self.cmp.restrict.clone();
        let (ch, tch) = (self.cmp.k_channel.clone(), self.cmp.k_time.clone());
        let mut inputs = Vec::new();
        let mut skipped = Vec::new();
        for si in 0..self.session.samples().len() {
            if !self.cmp.k_sel[si] {
                continue;
            }
            let name = self.session.sample(si).name.clone();
            let group = if self.cmp.groups[si].trim().is_empty() { name.clone() } else { self.cmp.groups[si].trim().to_string() };
            let node = if path.is_empty() {
                Some(GateTree::ROOT)
            } else {
                let refs: Vec<&str> = path.iter().map(|s| s.as_str()).collect();
                self.session.tree(si).find(&refs)
            };
            let Some(node) = node else {
                skipped.push(format!("{name}: population '{}' does not exist in this sample", path.join(" ▸ ")));
                continue;
            };
            let got = self.session.population_values(si, node, &tch).and_then(|t| self.session.population_values(si, node, &ch).map(|v| (t, v)));
            match got {
                Ok((time, value)) => inputs.push(KineticInput { sample: name, group, time, value }),
                Err(e) => skipped.push(format!("{name}: {e}")),
            }
        }
        match kinetics(&inputs, self.cmp.k_bins) {
            Ok(series) => {
                self.status = format!("Kinetics: {} sample(s) in {} group(s){}.", inputs.len(), series.len(), if skipped.is_empty() { String::new() } else { format!(", {} skipped", skipped.len()) });
                self.cmp.k_result = Some(KineticRun { channel: ch, series, skipped });
            }
            Err(e) => {
                self.status = e;
                self.cmp.k_result = None;
            }
        }
    }

    fn cmp_shell(&mut self, ui: &mut egui::Ui, id: &str, panel: impl FnOnce(&mut Self, &mut egui::Ui), body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples first.").color(theme::MUTED)));
            return;
        }
        self.cmp_sync();
        egui::SidePanel::left(format!("{id}_cfg")).resizable(false).exact_width(320.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| panel(self, ui));
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| body(self, ui));
        });
    }

    fn cmp_group_table(&mut self, ui: &mut egui::Ui, with_check: bool) {
        let n = self.session.samples().len();
        let presets: Vec<String> = {
            let mut p: Vec<String> = self.cmp.groups.iter().filter(|g| !g.trim().is_empty()).cloned().collect();
            p.sort();
            p.dedup();
            p
        };
        ui.horizontal(|ui| {
            for g in ["A", "B"] {
                if ui.small_button(format!("All → {g}")).clicked() {
                    self.cmp.groups.iter_mut().for_each(|x| *x = g.to_string());
                }
            }
            if ui.small_button("Clear").clicked() {
                self.cmp.groups.iter_mut().for_each(|x| x.clear());
            }
        });
        egui::ScrollArea::vertical().id_salt("cmp_samples").max_height(300.0).show(ui, |ui| {
            for i in 0..n {
                let name = self.session.samples()[i].name.clone();
                ui.horizontal(|ui| {
                    if with_check {
                        ui.checkbox(&mut self.cmp.k_sel[i], "");
                    }
                    ui.add(egui::TextEdit::singleline(&mut self.cmp.groups[i]).desired_width(70.0).hint_text("group"));
                    ui.label(RichText::new(name.trim_end_matches(".fcs")).size(12.0));
                });
            }
        });
        if !presets.is_empty() {
            theme::caption(ui, format!("Groups: {}", presets.join(", ")));
        }
    }

    pub(super) fn comparison_screen(&mut self, ui: &mut egui::Ui) {
        self.cmp_shell(
            ui,
            "cmp",
            |app, ui| {
                ui.label(RichText::new("Stats comparison").strong().size(16.0));
                theme::caption(ui, "Compare one statistic of one population between groups of samples. The test is chosen from the design: two groups use Mann-Whitney U (or Wilcoxon signed-rank when paired); three or more use Kruskal-Wallis with Dunn's post-hoc test.");
                theme::section(ui, "Statistic");
                app.cmp_population_picker(ui, "cmp_pop");
                let cur = Metric::COMMON[app.cmp.metric.min(Metric::COMMON.len() - 1)].label();
                egui::ComboBox::from_id_salt("cmp_metric").width(260.0).selected_text(cur).show_ui(ui, |ui| {
                    for (i, m) in Metric::COMMON.iter().enumerate() {
                        if ui.selectable_label(app.cmp.metric == i, m.label()).clicked() {
                            app.cmp.metric = i;
                        }
                    }
                });
                if Metric::COMMON[app.cmp.metric.min(Metric::COMMON.len() - 1)].needs_channel() {
                    let chans = app.session.sample(0).channels.clone();
                    egui::ComboBox::from_id_salt("cmp_chan").width(260.0).selected_text(format!("Channel: {}", app.cmp.channel)).show_ui(ui, |ui| {
                        for c in &chans {
                            if ui.selectable_label(*c == app.cmp.channel, c.as_str()).clicked() {
                                app.cmp.channel = c.clone();
                            }
                        }
                    });
                }
                theme::section(ui, "Groups");
                theme::caption(ui, "Type a group name next to each sample; blank samples are left out.");
                app.cmp_group_table(ui, false);
                ui.checkbox(&mut app.cmp.paired, "Paired (two groups, same order)");
                if app.cmp.paired {
                    theme::caption(ui, "The i-th sample of the first group is paired with the i-th of the second, in the order the samples are listed.");
                }
                ui.add_space(8.0);
                if ui.add(egui::Button::new(RichText::new("Compare").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(280.0, 32.0))).clicked() {
                    app.run_comparison();
                }
            },
            |app, ui| {
                let Some((label, c)) = app.cmp.result.as_ref() else {
                    ui.label(RichText::new("Assign samples to groups and press Compare.").color(theme::MUTED));
                    return;
                };
                ui.label(RichText::new(label).strong());
                let rect = plot_area(ui, 320.0);
                let all: Vec<f64> = c.groups.iter().flat_map(|g| g.values.iter().copied()).collect();
                let (lo, hi) = (all.iter().cloned().fold(f64::INFINITY, f64::min), all.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
                let pad = ((hi - lo) * 0.1).max(1e-9);
                let k = c.groups.len();
                let mut frame = Frame::new(rect, 0.0, k as f64, lo - pad, hi + pad);
                frame.x_ticks = false;
                frame.draw_axes(ui);
                let painter = ui.painter_at(rect);
                for (gi, g) in c.groups.iter().enumerate() {
                    let colour = GEN[gi % GEN.len()];
                    let cx = gi as f64 + 0.5;
                    for (i, v) in g.values.iter().enumerate() {
                        let jitter = ((i as f64 * 0.618_033_988_75) % 1.0 - 0.5) * 0.4;
                        painter.circle_filled(frame.to_screen(cx + jitter, *v), 4.0, colour);
                    }
                    let (a, b) = (frame.to_screen(cx - 0.3, g.median), frame.to_screen(cx + 0.3, g.median));
                    painter.line_segment([a, b], egui::Stroke::new(2.0_f32, Color32::WHITE));
                    painter.text(Pos2::new(frame.to_screen(cx, lo - pad).x, rect.bottom() - 2.0), Align2::CENTER_BOTTOM, format!("{} (n={})", g.name, g.n), FontId::proportional(11.0), theme::TEXT);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).inner_margin(egui::Margin::symmetric(16.0, 10.0)).show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&c.test).size(12.0).color(theme::MUTED));
                            ui.label(RichText::new(format!("p = {:.4}   {}", c.p, c.stars)).strong().size(22.0));
                            ui.label(RichText::new(format!("{} p-value", c.method)).size(11.0).color(theme::MUTED));
                        });
                    });
                });
                if !c.dropped.is_empty() {
                    let d: Vec<String> = c.dropped.iter().map(|(g, n)| format!("{g}: {n}")).collect();
                    ui.label(RichText::new(format!("Left out because the value could not be computed (non-finite or missing population): {}", d.join(", "))).color(theme::WARN));
                }
                if !c.posthoc.is_empty() {
                    ui.add_space(6.0);
                    theme::section(ui, "Dunn post-hoc (Bonferroni)");
                    egui::Grid::new("posthoc").num_columns(3).spacing([24.0, 4.0]).show(ui, |ui| {
                        for p in &c.posthoc {
                            ui.label(&p.pair);
                            ui.label(format!("{:.5}", p.p));
                            ui.label(p.stars);
                            ui.end_row();
                        }
                    });
                }
                ui.add_space(6.0);
                egui::Grid::new("grp_tbl").num_columns(3).spacing([24.0, 4.0]).show(ui, |ui| {
                    for h in ["Group", "n", "Median"] {
                        ui.label(RichText::new(h).color(theme::MUTED));
                    }
                    ui.end_row();
                    for g in &c.groups {
                        ui.label(&g.name);
                        ui.label(g.n.to_string());
                        ui.label(format!("{:.4}", g.median));
                        ui.end_row();
                    }
                });
                if ui.button("Copy results").clicked() {
                    let mut t = format!("{label}\ntest\t{}\nmethod\t{}\np\t{:.6}\n", c.test, c.method, c.p);
                    for g in &c.groups {
                        t += &format!("{}\tn={}\tmedian {:.4}\n", g.name, g.n, g.median);
                    }
                    for p in &c.posthoc {
                        t += &format!("{}\t{:.5}\t{}\n", p.pair, p.p, p.stars);
                    }
                    ui.ctx().copy_text(t);
                }
            },
        );
    }

    pub(super) fn kinetic_screen(&mut self, ui: &mut egui::Ui) {
        self.cmp_shell(
            ui,
            "kin",
            |app, ui| {
                ui.label(RichText::new("Kinetics").strong().size(16.0));
                theme::caption(ui, "Median of a channel in time bins for each sample, then averaged over the samples of each group (band = standard deviation between samples).");
                theme::section(ui, "Signal");
                app.cmp_population_picker(ui, "kin_pop");
                let chans = app.session.sample(0).channels.clone();
                for (id, label, which) in [("kin_chan", "Channel", 0), ("kin_time", "Time channel", 1)] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let cur = if which == 0 { app.cmp.k_channel.clone() } else { app.cmp.k_time.clone() };
                        egui::ComboBox::from_id_salt(id).width(150.0).selected_text(cur.clone()).show_ui(ui, |ui| {
                            for c in &chans {
                                if ui.selectable_label(*c == cur, c.as_str()).clicked() {
                                    if which == 0 {
                                        app.cmp.k_channel = c.clone();
                                    } else {
                                        app.cmp.k_time = c.clone();
                                    }
                                }
                            }
                        });
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Time bins");
                    ui.add(egui::DragValue::new(&mut app.cmp.k_bins).range(5..=1000));
                });
                theme::section(ui, "Samples");
                theme::caption(ui, "Tick the samples to use. A group name is optional; without one each sample is its own curve.");
                app.cmp_group_table(ui, true);
                ui.add_space(8.0);
                if ui.add(egui::Button::new(RichText::new("Plot kinetics").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(280.0, 32.0))).clicked() {
                    app.run_kinetics();
                }
            },
            |app, ui| {
                let Some(run) = app.cmp.k_result.as_ref() else {
                    ui.label(RichText::new("Choose the channel and press Plot kinetics.").color(theme::MUTED));
                    return;
                };
                ui.label(RichText::new(format!("{} against time", run.channel)).strong());
                let rect = plot_area(ui, 340.0);
                let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                for s in &run.series {
                    for ((m, d), _) in s.mfi.iter().zip(&s.sd).zip(&s.times) {
                        if m.is_finite() {
                            lo = lo.min(m - d);
                            hi = hi.max(m + d);
                        }
                    }
                }
                if !(lo.is_finite() && hi.is_finite()) {
                    ui.label("No bin has events.");
                    return;
                }
                let pad = ((hi - lo) * 0.08).max(1e-9);
                let t = &run.series[0].times;
                let frame = Frame::new(rect, t[0], *t.last().unwrap(), lo - pad, hi + pad);
                frame.draw_axes(ui);
                for (gi, s) in run.series.iter().enumerate() {
                    let colour = GEN[gi % GEN.len()];
                    let band = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 70);
                    // break the line at empty bins instead of joining across a gap
                    let mut start = 0;
                    while start < s.mfi.len() {
                        if s.mfi[start].is_nan() {
                            start += 1;
                            continue;
                        }
                        let mut end = start;
                        while end < s.mfi.len() && !s.mfi[end].is_nan() {
                            end += 1;
                        }
                        let (x, m) = (&s.times[start..end], &s.mfi[start..end]);
                        let up: Vec<f64> = m.iter().zip(&s.sd[start..end]).map(|(a, b)| a + b).collect();
                        let dn: Vec<f64> = m.iter().zip(&s.sd[start..end]).map(|(a, b)| a - b).collect();
                        frame.line(ui, x, &up, band, 1.0);
                        frame.line(ui, x, &dn, band, 1.0);
                        frame.line(ui, x, m, colour, 2.0);
                        start = end;
                    }
                }
                ui.add_space(6.0);
                for (gi, s) in run.series.iter().enumerate() {
                    ui.label(RichText::new(format!("● {}  ({} sample{})", s.group, s.n_samples, if s.n_samples == 1 { "" } else { "s" })).color(GEN[gi % GEN.len()]));
                }
                if !run.skipped.is_empty() {
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("{} sample(s) not used:", run.skipped.len())).color(theme::WARN));
                    for s in &run.skipped {
                        ui.label(RichText::new(s).size(12.0).color(theme::WARN));
                    }
                }
                ui.label(RichText::new("Gaps are bins with no events; the curve is broken there rather than drawn as zero.").size(11.0).color(theme::MUTED));
            },
        );
    }
}
