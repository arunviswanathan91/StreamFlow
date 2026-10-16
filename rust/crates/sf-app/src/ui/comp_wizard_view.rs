//! The compensation wizard window: assign single-stain controls (and an unstained tube), see how each control was
//! split into negative and positive events, open any control to move the limits by hand, calculate the matrix with a
//! chosen method, and check its quality (warnings, spillover spreading, signature similarity).

use super::{theme, App};
use crate::comp_auto::{detector_from_name, detector_from_signal};
use crate::comp_wizard::{auto_cut, clean_mask_for, compensated_pair, compute, CleanMode, ControlSetup, Method, NegSource, Options, SplitMethod, WizardResult};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke};

#[derive(Clone, Default)]
pub struct WizRow {
    pub detector: Option<String>,
    /// (negative up to, positive from), when set by hand.
    pub manual: Option<(f64, f64)>,
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Summary,
    Spread,
    Cosine,
}

impl Default for WizState {
    fn default() -> Self {
        WizState { open: false, rows: vec![], opt: Options::default(), clean_on: true, clean_frac: 0.12, unstained: None, result: None, inspect: None, show: Tab::Summary }
    }
}

const COF: f64 = 150.0;

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
            w.rows = (0..n).map(|i| WizRow { detector: detector_from_name(&self.session.samples()[i].name, &dets), manual: None }).collect();
            w.unstained = (0..n).find(|&i| {
                let nm = self.session.samples()[i].name.to_ascii_lowercase();
                nm.contains("unstain") || nm.contains("negative") || nm.contains("blank")
            });
            w.result = None;
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

    fn wiz_setups(&self) -> Vec<ControlSetup> {
        self.comp.wiz.rows.iter().enumerate().filter_map(|(i, r)| r.detector.clone().map(|d| ControlSetup { sample: i, detector: d, manual: r.manual })).collect()
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
        let mut open = true;
        let (mut calc, mut use_it, mut inspect_click) = (false, false, None::<usize>);
        let max_h = (ctx.screen_rect().height() - 90.0).max(300.0);
        egui::Window::new("Compensation wizard").open(&mut open).collapsible(false).resizable(true).default_size([960.0, max_h.min(640.0)]).max_height(max_h).default_pos([140.0, 50.0]).show(ctx, |ui| egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            theme::caption(ui, "Assign each single-stain control to the detector it stains, optionally an unstained tube, choose how the matrix is calculated, and check the result before using it.");
            ui.horizontal_top(|ui| {
                // ---- left: controls table
                ui.vertical(|ui| {
                    ui.set_width(470.0);
                    ui.label(RichText::new("Controls").strong());
                    egui::ScrollArea::vertical().id_salt("wiz_rows").max_height(230.0).show(ui, |ui| {
                        egui::Grid::new("wiz_grid").spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
                            for h in ["Sample", "Parameter", "Negative | Positive", ""] {
                                ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
                            }
                            ui.end_row();
                            for i in 0..n {
                                let nm = self.session.samples()[i].name.trim_end_matches(".fcs").to_string();
                                let is_un = wiz.unstained == Some(i);
                                ui.label(RichText::new(if is_un { format!("{nm}  (unstained)") } else { nm }).size(12.0));
                                let cur = wiz.rows[i].detector.clone().unwrap_or_else(|| "—".into());
                                egui::ComboBox::from_id_salt(("wiz_det", i)).width(110.0).selected_text(cur).show_ui(ui, |ui| {
                                    if ui.selectable_label(wiz.rows[i].detector.is_none(), "—").clicked() {
                                        wiz.rows[i].detector = None;
                                        wiz.rows[i].manual = None;
                                    }
                                    for d in &dets {
                                        if ui.selectable_label(wiz.rows[i].detector.as_deref() == Some(d.as_str()), d).clicked() {
                                            wiz.rows[i].detector = Some(d.clone());
                                            wiz.rows[i].manual = None;
                                        }
                                    }
                                });
                                let status = match (&wiz.result, wiz.rows[i].detector.as_ref()) {
                                    (Some(Ok(r)), Some(_)) => r.reports.iter().find(|c| c.sample == i).map(|c| {
                                        let mut t = format!("{} | {}", c.n_neg, c.n_pos);
                                        if wiz.rows[i].manual.is_some() {
                                            t += "  (manual)";
                                        }
                                        (t, c.badges.iter().any(|b| b.blocks()), !c.badges.is_empty())
                                    }),
                                    _ => None,
                                };
                                match status {
                                    Some((t, bad, warn)) => {
                                        let r = ui.label(RichText::new(t).size(11.5).color(if bad { theme::GATE_RED } else if warn { theme::WARN } else { theme::GOOD }));
                                        if let Some(Ok(res)) = &wiz.result {
                                            if let Some(c) = res.reports.iter().find(|c| c.sample == i) {
                                                let tip: Vec<String> = c.badges.iter().map(|b| b.label().to_string()).chain([format!("{} of {} events kept by the scatter clean-up", c.n_clean, c.n_events), format!("stain index {:.1}", c.stain_index)]).collect();
                                                r.on_hover_text(tip.join("\n"));
                                            }
                                        }
                                    }
                                    None => {
                                        ui.label("");
                                    }
                                }
                                if ui.add_enabled(wiz.rows[i].detector.is_some(), egui::Button::new("Inspect…").small()).on_hover_text("Open this control: see the negative and positive events and move the limits").clicked() {
                                    inspect_click = Some(i);
                                }
                                ui.end_row();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if ui.small_button("Guess from file names").clicked() {
                            for i in 0..n {
                                wiz.rows[i] = WizRow { detector: detector_from_name(&self.session.samples()[i].name, &dets), manual: None };
                            }
                        }
                        if ui.small_button("Guess from signal").on_hover_text("The detector that is brightest in each file").clicked() {
                            for i in 0..n {
                                if wiz.rows[i].detector.is_none() {
                                    wiz.rows[i] = WizRow { detector: detector_from_signal(self.session.sample(i), &dets), manual: None };
                                }
                            }
                        }
                        if ui.small_button("Clear all").clicked() {
                            wiz.rows.iter_mut().for_each(|r| *r = WizRow::default());
                        }
                    });
                });
                ui.separator();
                // ---- right: options
                ui.vertical(|ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new("Method").strong());
                    for m in Method::ALL {
                        ui.radio_value(&mut wiz.opt.method, m, m.label());
                    }
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
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(RichText::new("Calculate").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(120.0, 30.0))).clicked() {
                            calc = true;
                        }
                        let ok = matches!(wiz.result, Some(Ok(_)));
                        if ui.add_enabled(ok, egui::Button::new("Use this matrix").min_size(egui::vec2(130.0, 30.0))).on_hover_text("Puts the matrix in the Compensation screen to review, edit and apply").clicked() {
                            use_it = true;
                        }
                    });
                });
            });
            ui.separator();
            // ---- result
            match &wiz.result {
                None => theme::caption(ui, "Press Calculate."),
                Some(Err(e)) => {
                    ui.label(RichText::new(format!("✕ {e}")).color(theme::GATE_RED));
                }
                Some(Ok(r)) => {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut wiz.show, Tab::Summary, "Summary");
                        ui.selectable_value(&mut wiz.show, Tab::Spread, "Spillover spreading");
                        ui.selectable_value(&mut wiz.show, Tab::Cosine, "Signature similarity");
                    });
                    match wiz.show {
                        Tab::Summary => result_summary(ui, r, &self.session),
                        Tab::Spread => matrix_table(ui, "wiz_spread", &r.matrix.detectors, &r.spread, 1.0, [224, 120, 40], "Noise each stained control adds to every other detector after compensation (standard deviation per √intensity). Large values mean that pair is hard to resolve."),
                        Tab::Cosine => matrix_table(ui, "wiz_cos", &r.matrix.detectors, &r.cosine, 1.0, [0, 150, 200], "Cosine similarity of the spillover rows: close to 1 means two fluorochromes look alike on this instrument."),
                    }
                }
            }
        }));
        if let Some(i) = inspect_click {
            wiz.inspect = Some(i);
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

    /// One control opened: histogram of the stained detector with movable limits, the scatter clean-up and, once a
    /// matrix is calculated, the control before and after compensation.
    fn inspect_window(&mut self, ctx: &egui::Context) {
        let Some(si) = self.comp.wiz.inspect else { return };
        let Some(det) = self.comp.wiz.rows.get(si).and_then(|r| r.detector.clone()) else {
            self.comp.wiz.inspect = None;
            return;
        };
        let mut wiz = std::mem::take(&mut self.comp.wiz);
        let mut open = true;
        let s = self.session.sample(si);
        let name = s.name.clone();
        let ci = s.channel_index(&det);
        egui::Window::new(format!("Control: {name} — {det}")).id(egui::Id::new("wiz_inspect")).open(&mut open).collapsible(false).resizable(true).default_size([620.0, 700.0]).default_pos([260.0, 90.0]).show(ctx, |ui| {
            let Some(ci) = ci else {
                ui.label("This sample has no such channel.");
                return;
            };
            let d = s.channels.len();
            let keep = clean_mask_for(s, if wiz.clean_on { CleanMode::Dominant(wiz.clean_frac) } else { CleanMode::Off }, Some(ci));
            let vals: Vec<f64> = (0..s.n_events).filter(|&r| keep[r]).map(|r| s.events[r * d + ci] as f64).collect();
            let all: Vec<f64> = (0..s.n_events).map(|r| s.events[r * d + ci] as f64).collect();
            let auto = auto_cut(&vals, wiz.opt.split);
            let (neg_max, pos_min) = wiz.rows[si].manual.unwrap_or((auto, auto));
            // ---- histogram on the arcsinh scale
            let (zlo, zhi) = {
                let z: Vec<f64> = all.iter().map(|&v| z_of(v)).collect();
                (sf_core::stats::percentile(&z, 0.2), sf_core::stats::percentile(&z, 99.9))
            };
            let bins = 140usize;
            let hist = |v: &[f64]| -> Vec<f64> {
                let mut h = vec![0.0; bins];
                for &x in v {
                    let t = (z_of(x) - zlo) / (zhi - zlo);
                    if (0.0..1.0).contains(&t) {
                        h[(t * bins as f64) as usize] += 1.0;
                    }
                }
                h
            };
            let h = hist(&vals);
            let unstained = wiz.unstained.filter(|&u| u != si).map(|u| {
                let us = self.session.sample(u);
                us.channel_index(&det).map(|c| (0..us.n_events).map(|r| us.events[r * us.channels.len() + c] as f64).collect::<Vec<f64>>())
            });
            let hu = unstained.flatten().map(|v| hist(&v));
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(580.0), 210.0), Sense::click_and_drag());
            let p = ui.painter_at(rect);
            p.rect_filled(rect, 4.0, Color32::WHITE);
            let plot = Rect::from_min_max(Pos2::new(rect.left() + 34.0, rect.top() + 8.0), Pos2::new(rect.right() - 10.0, rect.bottom() - 28.0));
            let hmax = h.iter().cloned().fold(1.0, f64::max).max(hu.as_ref().map_or(0.0, |u| u.iter().cloned().fold(0.0, f64::max)));
            let x_of = |z: f64| plot.left() + ((z - zlo) / (zhi - zlo)) as f32 * plot.width();
            let z_at = |x: f32| zlo + ((x - plot.left()) / plot.width()) as f64 * (zhi - zlo);
            for (b, &c) in h.iter().enumerate() {
                let x0 = plot.left() + plot.width() * b as f32 / bins as f32;
                let hgt = (c / hmax) as f32 * plot.height();
                let z_mid = zlo + (zhi - zlo) * (b as f64 + 0.5) / bins as f64;
                let col = if raw_of(z_mid) <= neg_max { Color32::from_rgb(70, 130, 200) } else if raw_of(z_mid) >= pos_min { Color32::from_rgb(200, 80, 70) } else { Color32::from_gray(170) };
                p.rect_filled(Rect::from_min_max(Pos2::new(x0, plot.bottom() - hgt), Pos2::new(x0 + plot.width() / bins as f32, plot.bottom())), 0.0, col);
            }
            if let Some(hu) = &hu {
                let pts: Vec<Pos2> = hu.iter().enumerate().map(|(b, &c)| Pos2::new(plot.left() + plot.width() * (b as f32 + 0.5) / bins as f32, plot.bottom() - (c / hmax) as f32 * plot.height())).collect();
                p.add(egui::Shape::line(pts, Stroke::new(1.5_f32, Color32::from_rgb(40, 40, 40))));
            }
            for raw in [-1000.0, 0.0, 1000.0, 10_000.0, 100_000.0] {
                let z = z_of(raw);
                if z > zlo && z < zhi {
                    let x = x_of(z);
                    p.line_segment([Pos2::new(x, plot.bottom()), Pos2::new(x, plot.bottom() + 4.0)], Stroke::new(1.0_f32, Color32::GRAY));
                    p.text(Pos2::new(x, plot.bottom() + 6.0), Align2::CENTER_TOP, short(raw), FontId::proportional(10.0), Color32::from_gray(60));
                }
            }
            p.rect_stroke(plot, 0.0, Stroke::new(1.0_f32, Color32::GRAY));
            // limits: drag the nearer one
            let (xn, xp) = (x_of(z_of(neg_max)), x_of(z_of(pos_min)));
            p.line_segment([Pos2::new(xn, plot.top()), Pos2::new(xn, plot.bottom())], Stroke::new(2.0_f32, Color32::from_rgb(40, 90, 170)));
            p.line_segment([Pos2::new(xp, plot.top()), Pos2::new(xp, plot.bottom())], Stroke::new(2.0_f32, Color32::from_rgb(170, 50, 40)));
            if resp.dragged() || resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos().filter(|q| plot.contains(*q)) {
                    let raw = raw_of(z_at(pos.x));
                    let (mut nm, mut pm) = (neg_max, pos_min);
                    // the limit closest to the pointer moves; they keep their order
                    if (pos.x - xn).abs() <= (pos.x - xp).abs() {
                        nm = raw.min(pm);
                    } else {
                        pm = raw.max(nm);
                    }
                    wiz.rows[si].manual = Some((nm, pm));
                }
            }
            resp.on_hover_text("Drag the blue line (negative up to) or the red line (positive from)");
            let nneg = vals.iter().filter(|&&v| v <= neg_max).count();
            let npos = vals.iter().filter(|&&v| v >= pos_min).count();
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("negative ≤ {}: {nneg}", short(neg_max))).color(Color32::from_rgb(40, 90, 170)));
                ui.label(RichText::new(format!("positive ≥ {}: {npos}", short(pos_min))).color(Color32::from_rgb(170, 50, 40)));
                ui.label(RichText::new(format!("{} of {} events after clean-up", vals.len(), s.n_events)).color(theme::MUTED).size(11.5));
            });
            ui.horizontal(|ui| {
                if ui.button("Automatic").on_hover_text(format!("Back to the automatic limit ({})", short(auto))).clicked() {
                    wiz.rows[si].manual = None;
                }
                if hu.is_some() {
                    theme::caption(ui, "black line: the unstained tube");
                }
            });
            ui.separator();
            // ---- scatter clean-up
            ui.label(RichText::new("Scatter clean-up").strong().size(12.0));
            let fsc = s.channels.iter().position(|c| c.to_ascii_uppercase() == "FSC-A");
            let ssc = s.channels.iter().position(|c| c.to_ascii_uppercase() == "SSC-A");
            ui.horizontal(|ui| {
                if let (Some(f), Some(sc)) = (fsc, ssc) {
                    let pts: Vec<(f32, f32, bool)> = (0..s.n_events).step_by((s.n_events / 4000).max(1)).map(|r| (s.events[r * d + f], s.events[r * d + sc], keep[r])).collect();
                    mini_scatter(ui, &pts, [(0.0, s.ranges[f] as f32), (0.0, s.ranges[sc] as f32)], "FSC-A", "SSC-A");
                } else {
                    theme::caption(ui, "This file has no FSC-A / SSC-A channels, so there is nothing to clean up.");
                }
                // ---- before and after
                if let Some(Ok(r)) = &wiz.result {
                    let other = strongest_spill(r, &det);
                    if let Some(o) = other {
                        let step = (s.n_events / 4000).max(1);
                        if let Ok((cx, cy)) = compensated_pair(s, &r.matrix, &det, &o) {
                            let raw: Vec<(f32, f32, bool)> = (0..s.n_events).step_by(step).map(|r| (z_of(s.events[r * d + ci] as f64) as f32, z_of(s.channel_index(&o).map_or(0.0, |c| s.events[r * d + c] as f64)) as f32, true)).collect();
                            let step2 = (cx.len() / 4000).max(1);
                            let after: Vec<(f32, f32, bool)> = (0..cx.len()).step_by(step2).map(|i| (z_of(cx[i]) as f32, z_of(cy[i]) as f32, true)).collect();
                            let rng = [(z_of(-500.0) as f32, zhi as f32), (z_of(-500.0) as f32, zhi as f32)];
                            ui.vertical(|ui| {
                                ui.label(RichText::new(format!("{det} vs {o}: before").to_string()).size(11.0));
                                mini_scatter(ui, &raw, rng, &det, &o);
                            });
                            ui.vertical(|ui| {
                                ui.label(RichText::new("after compensation").size(11.0));
                                mini_scatter(ui, &after, rng, &det, &o);
                            });
                        }
                    }
                }
            });
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

fn mini_scatter(ui: &mut egui::Ui, pts: &[(f32, f32, bool)], range: [(f32, f32); 2], xl: &str, yl: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(210.0, 190.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, Color32::WHITE);
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 6.0, rect.top() + 6.0), Pos2::new(rect.right() - 6.0, rect.bottom() - 16.0));
    let (xr, yr) = ((range[0].1 - range[0].0).max(1e-6), (range[1].1 - range[1].0).max(1e-6));
    for &(x, y, kept) in pts {
        let (fx, fy) = ((x - range[0].0) / xr, (y - range[1].0) / yr);
        if (0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy) {
            p.circle_filled(Pos2::new(plot.left() + fx * plot.width(), plot.bottom() - fy * plot.height()), 1.1, if kept { Color32::from_rgba_unmultiplied(40, 90, 170, 150) } else { Color32::from_rgba_unmultiplied(170, 170, 170, 120) });
        }
    }
    p.rect_stroke(plot, 0.0, Stroke::new(1.0_f32, Color32::GRAY));
    p.text(Pos2::new(plot.center().x, rect.bottom() - 2.0), Align2::CENTER_BOTTOM, format!("{xl}  ↔   {yl} ↕"), FontId::proportional(9.5), Color32::from_gray(80));
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
    egui::ScrollArea::vertical().id_salt("wiz_summary").max_height(150.0).show(ui, |ui| {
        egui::Grid::new("wiz_sum").spacing([10.0, 3.0]).striped(true).show(ui, |ui| {
            for h in ["Control", "Detector", "Events kept", "Negative", "Positive", "Stain index", "Warnings"] {
                ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
            }
            ui.end_row();
            for c in &r.reports {
                ui.label(RichText::new(session.samples()[c.sample].name.trim_end_matches(".fcs")).size(11.5));
                ui.label(RichText::new(c.detector.trim_end_matches("-A")).size(11.5));
                ui.label(RichText::new(format!("{} / {}", c.n_clean, c.n_events)).size(11.5));
                ui.label(RichText::new(format!("{} (≤{})", c.n_neg, short(c.neg_max))).size(11.5));
                ui.label(RichText::new(format!("{} (≥{})", c.n_pos, short(c.pos_min))).size(11.5));
                ui.label(RichText::new(if c.stain_index.is_finite() { format!("{:.1}", c.stain_index) } else { "—".into() }).size(11.5));
                let w: Vec<&str> = c.badges.iter().map(|b| b.label()).collect();
                ui.label(RichText::new(if w.is_empty() { "—".to_string() } else { w.join("; ") }).size(11.0).color(if w.is_empty() { theme::MUTED } else { theme::WARN }));
                ui.end_row();
            }
        });
    });
    for n in &r.notes {
        theme::caption(ui, n.clone());
    }
}

pub(super) fn matrix_table(ui: &mut egui::Ui, id: &str, det: &[String], m: &[f64], scale: f64, col: [u8; 3], help: &str) {
    theme::caption(ui, help);
    let k = det.len();
    egui::ScrollArea::both().id_salt(id).max_height(200.0).show(ui, |ui| {
        egui::Grid::new(id).spacing([2.0, 2.0]).min_col_width(46.0).show(ui, |ui| {
            ui.label("");
            for d in det {
                ui.label(RichText::new(d.trim_end_matches("-A")).size(10.0).color(theme::MUTED));
            }
            ui.end_row();
            for i in 0..k {
                ui.label(RichText::new(det[i].trim_end_matches("-A")).size(10.0).color(theme::MUTED));
                for j in 0..k {
                    let v = m[i * k + j];
                    let r = egui::Frame::none().fill(heat(v, scale, col)).rounding(3.0).show(ui, |ui| ui.add_sized([46.0, 18.0], egui::Label::new(RichText::new(if v == 0.0 { "".to_string() } else { format!("{v:.2}") }).monospace().size(10.5)))).response;
                    let _ = r;
                }
                ui.end_row();
            }
        });
    });
}
