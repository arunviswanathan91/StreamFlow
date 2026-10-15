//! One graph tab: toolbar, breadcrumb, axis controls, the density canvas with gates, and the options panel.

use super::{theme, workstation::Tab, App};
use crate::axis::ticks;
use crate::density::{bin, histogram, render, render_dots, PlotKind, Range};
use crate::drawing::{Drawing, Shape, Tool};
use crate::session::{default_scale, GraphWindow};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions};
use sf_core::scale::ScaleSpec;
use sf_core::stats;
use sf_gating::{Gate, GateKind, NodeId, NodeKind};

/// Display-space <-> screen mapping of a plot area (y grows upward in data, downward on screen).
#[derive(Debug, Clone, Copy)]
pub struct Mapper {
    pub rect: Rect,
    pub range: Range,
}

impl Mapper {
    pub fn to_screen(&self, p: (f64, f64)) -> Pos2 {
        let fx = (p.0 - self.range.x0) / (self.range.x1 - self.range.x0);
        let fy = (p.1 - self.range.y0) / (self.range.y1 - self.range.y0);
        Pos2::new(self.rect.left() + fx as f32 * self.rect.width(), self.rect.bottom() - fy as f32 * self.rect.height())
    }

    pub fn to_data(&self, s: Pos2) -> (f64, f64) {
        let fx = ((s.x - self.rect.left()) / self.rect.width()) as f64;
        let fy = ((self.rect.bottom() - s.y) / self.rect.height()) as f64;
        (self.range.x0 + fx * (self.range.x1 - self.range.x0), self.range.y0 + fy * (self.range.y1 - self.range.y0))
    }
}

/// How gates are drawn on this tab.
#[derive(Debug, Clone)]
pub struct GateStyle {
    pub color: Color32,
    pub width: f32,
    pub labels: bool,
    pub fill: bool,
}

impl Default for GateStyle {
    fn default() -> Self {
        GateStyle { color: theme::GATE_RED, width: 1.8, labels: true, fill: false }
    }
}

/// Per-tab rendering cache and view state.
pub struct GraphUi {
    pub tex: Option<TextureHandle>,
    pub key: u64,
    pub range: Option<Range>,
    pub shown: usize,
    /// Set when axes changed; forces an auto range on the next frame.
    pub refit: bool,
    /// Where the plot area was last drawn on screen (for hit-testing and tests).
    pub plot_rect: Option<Rect>,
    /// Time and screen position of the previous click, to recognise a closing double-click ourselves.
    pub last_click: Option<(f64, Pos2)>,
    pub style: GateStyle,
    /// Population points cached per (version, node, axes), so resizing only re-bins.
    pts: Option<(u64, Vec<(f64, f64)>)>,
    summary: Option<(u64, Vec<(String, String)>)>,
    /// Histogram bar heights (normalised 0..1) of the last render.
    hist: Vec<f32>,
}

impl GraphUi {
    pub fn new() -> GraphUi {
        GraphUi { tex: None, key: 0, range: None, shown: 0, refit: true, plot_rect: None, last_click: None, style: GateStyle::default(), pts: None, summary: None, hist: Vec::new() }
    }
}

impl Default for GraphUi {
    fn default() -> Self {
        GraphUi::new()
    }
}

fn hash_of<T: std::hash::Hash>(t: T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn scale_name(s: &ScaleSpec) -> &'static str {
    match s {
        ScaleSpec::Identity | ScaleSpec::Linear { .. } => "Linear",
        ScaleSpec::Arcsinh { .. } => "Arcsinh",
        ScaleSpec::Logicle { .. } => "Logicle",
        ScaleSpec::Biex { .. } => "Biex",
        ScaleSpec::Log { .. } => "Log",
    }
}

/// Channel + scale pickers for one axis. Returns true when anything changed.
fn axis_controls(ui: &mut egui::Ui, id: (&str, u64), channels: &[String], ranges: &[f64], chan: &mut String, scale: &mut ScaleSpec, width: f32) -> bool {
    let (before_c, before_s) = (chan.clone(), scale.clone());
    egui::ComboBox::from_id_salt((id, "chan")).width(width).selected_text(chan.clone()).show_ui(ui, |ui| {
        for c in channels {
            ui.selectable_value(chan, c.clone(), c);
        }
    });
    if *chan != before_c {
        let r = channels.iter().position(|c| c == chan).map_or(262_144.0, |i| ranges[i]);
        *scale = default_scale(chan, r);
    }
    let range = channels.iter().position(|c| c == chan).map_or(262_144.0, |i| ranges[i]);
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt((id, "scale")).width(width - 34.0).selected_text(scale_name(scale)).show_ui(ui, |ui| {
            if ui.selectable_label(matches!(scale, ScaleSpec::Identity), "Linear").clicked() {
                *scale = ScaleSpec::Identity;
            }
            if ui.selectable_label(matches!(scale, ScaleSpec::Arcsinh { .. }), "Arcsinh").clicked() {
                *scale = ScaleSpec::Arcsinh { cofactor: 150.0 };
            }
            if ui.selectable_label(matches!(scale, ScaleSpec::Logicle { .. }), "Logicle").clicked() {
                *scale = ScaleSpec::Logicle { t: range.max(262_144.0), w: 0.5, m: 4.5, a: 0.0 };
            }
        });
        ui.menu_button("⚙", |ui| {
            ui.set_min_width(190.0);
            match scale {
                ScaleSpec::Arcsinh { cofactor } => {
                    ui.label("Cofactor");
                    ui.add(egui::DragValue::new(cofactor).speed(1.0).range(0.1..=100_000.0));
                    ui.horizontal(|ui| {
                        for c in [5.0, 150.0, 6000.0] {
                            if ui.small_button(format!("{c}")).clicked() {
                                *cofactor = c;
                            }
                        }
                    });
                }
                ScaleSpec::Logicle { t, w, m, a } => {
                    ui.label("Top of scale (T)");
                    ui.add(egui::DragValue::new(t).speed(100.0).range(1.0..=1.0e9));
                    ui.label("Linear width (W, decades)");
                    ui.add(egui::DragValue::new(w).speed(0.01).range(0.0..=2.0));
                    ui.label("Decades (M)");
                    ui.add(egui::DragValue::new(m).speed(0.05).range(1.0..=8.0));
                    ui.label("Extra negative (A)");
                    ui.add(egui::DragValue::new(a).speed(0.05).range(0.0..=3.0));
                    // Logicle needs 2W <= M
                    if 2.0 * *w > *m {
                        *w = *m / 2.0;
                    }
                }
                _ => {
                    ui.label(RichText::new("Linear axes have no parameters.").color(theme::MUTED));
                }
            }
        })
        .response
        .on_hover_text("Scale parameters");
    });
    *chan != before_c || *scale != before_s
}

impl App {
    pub(super) fn graph_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: u64) {
        let Some(w) = self.session.window(id).cloned() else { return };
        self.graph_shortcuts(ctx, id, &w);
        let w = self.session.window(id).cloned().unwrap_or(w);

        egui::TopBottomPanel::top("g_tools").frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 6.0))).show_inside(ui, |ui| self.graph_toolbar(ui, id, &w));
        egui::TopBottomPanel::top("g_crumb").frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::symmetric(12.0, 4.0))).show_inside(ui, |ui| self.breadcrumb(ui, id, &w));
        egui::TopBottomPanel::bottom("g_hint").frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 3.0))).show_inside(ui, |ui| {
            theme::caption(ui, "P polygon · R rectangle · E ellipse · I interval · Q quadrant · N pointer · Del remove gate · Ctrl+Z / Ctrl+Y undo / redo · ← → sample · wheel zoom · drag (pointer) pan");
        });
        let open = self.ws.options_open;
        egui::SidePanel::right("g_opts").resizable(true).default_width(250.0).width_range(200.0..=380.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(10.0))).show_animated_inside(ui, open, |ui| self.graph_options(ui, id, &w));
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::same(8.0))).show_inside(ui, |ui| self.graph_canvas(ui, ctx, id));
    }

    fn graph_shortcuts(&mut self, ctx: &egui::Context, id: u64, w: &GraphWindow) {
        if ctx.wants_keyboard_input() || self.ws.tab != Tab::Graph(id) {
            return;
        }
        let (undo, redo, key) = ctx.input_mut(|i| {
            let z = i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Z);
            let y = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Y);
            let u = i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z);
            let tool_key = if i.modifiers.command || i.modifiers.alt {
                None
            } else {
                [(egui::Key::P, Tool::Polygon), (egui::Key::R, Tool::Rectangle), (egui::Key::E, Tool::Ellipse), (egui::Key::I, Tool::Interval), (egui::Key::Q, Tool::Quadrant), (egui::Key::N, Tool::Pointer)]
                    .into_iter()
                    .find(|(k, _)| i.key_pressed(*k))
                    .map(|(_, t)| t)
            };
            (u, y || z, tool_key)
        });
        if undo {
            self.do_undo(w.sample);
        }
        if redo {
            self.do_redo(w.sample);
        }
        if let Some(t) = key {
            if let Some(win) = self.session.window_mut(id) {
                win.tool = t;
                win.drawing.cancel();
            }
        }
        let (left, right, del) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::Delete)));
        if left && w.sample > 0 {
            self.hop_sample(id, w.sample - 1);
        }
        if right && w.sample + 1 < self.session.samples().len() {
            self.hop_sample(id, w.sample + 1);
        }
        if del {
            if let Some((s, n)) = self.session.selected {
                let is_child = s == w.sample && self.session.tree(s).parent(n).ok().flatten() == Some(w.node);
                if is_child {
                    self.status = match self.session.delete(s, n) {
                        Ok(k) => format!("Deleted {k} population(s). Ctrl+Z to undo."),
                        Err(e) => e,
                    };
                }
            }
        }
    }

    pub(crate) fn do_undo(&mut self, sample: usize) {
        self.status = if self.session.undo(sample) { "Undone.".into() } else { "Nothing to undo.".into() };
    }

    pub(crate) fn do_redo(&mut self, sample: usize) {
        self.status = if self.session.redo(sample) { "Redone.".into() } else { "Nothing to redo.".into() };
    }

    fn hop_sample(&mut self, id: u64, sample: usize) {
        match self.session.retarget_window(id, sample) {
            Ok(()) => {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    g.refit = true;
                }
                self.ws.tree_sample = sample;
            }
            Err(e) => self.status = e,
        }
    }

    fn graph_toolbar(&mut self, ui: &mut egui::Ui, id: u64, w: &GraphWindow) {
        ui.horizontal_wrapped(|ui| {
            let n = self.session.samples().len();
            if ui.add_enabled(w.sample > 0, egui::Button::new("◀")).on_hover_text("Previous sample (←)").clicked() {
                self.hop_sample(id, w.sample - 1);
            }
            let name = self.session.samples()[w.sample].name.clone();
            let mut pick = w.sample;
            egui::ComboBox::from_id_salt(("g_sample", id)).width(190.0).selected_text(name).show_ui(ui, |ui| {
                for i in 0..n {
                    let nm = self.session.samples()[i].name.clone();
                    ui.selectable_value(&mut pick, i, nm);
                }
            });
            if pick != w.sample {
                self.hop_sample(id, pick);
            }
            if ui.add_enabled(w.sample + 1 < n, egui::Button::new("▶")).on_hover_text("Next sample (→)").clicked() {
                self.hop_sample(id, w.sample + 1);
            }
            ui.separator();
            let mut kind = w.kind;
            egui::ComboBox::from_id_salt(("g_kind", id)).width(110.0).selected_text(w.kind.label()).show_ui(ui, |ui| {
                for k in PlotKind::ALL {
                    ui.selectable_value(&mut kind, k, k.label());
                }
            });
            if kind != w.kind {
                if let Some(win) = self.session.window_mut(id) {
                    win.kind = kind;
                    if kind == PlotKind::Histogram && !matches!(win.tool, Tool::Interval | Tool::Pointer) {
                        win.tool = Tool::Interval; // a histogram has no second axis
                    }
                }
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    g.refit = true;
                }
            }
            ui.separator();
            for (tool, icon) in [(Tool::Pointer, "↖"), (Tool::Polygon, "⬠"), (Tool::Rectangle, "▭"), (Tool::Ellipse, "◯"), (Tool::Interval, "↔"), (Tool::Quadrant, "✚")] {
                let disabled = w.kind == PlotKind::Histogram && !matches!(tool, Tool::Interval | Tool::Pointer);
                let r = ui.add_enabled(!disabled, egui::SelectableLabel::new(w.tool == tool, RichText::new(format!("{icon}  {}", tool.label())).size(12.5)));
                if r.clicked() {
                    if let Some(win) = self.session.window_mut(id) {
                        win.tool = tool;
                        win.drawing.cancel();
                    }
                }
            }
            ui.separator();
            if ui.add_enabled(self.session.can_undo(w.sample), egui::Button::new("↶ Undo")).on_hover_text("Ctrl+Z").clicked() {
                self.do_undo(w.sample);
            }
            if ui.add_enabled(self.session.can_redo(w.sample), egui::Button::new("↷ Redo")).on_hover_text("Ctrl+Y").clicked() {
                self.do_redo(w.sample);
            }
            ui.separator();
            if ui.button("⤢ Fit").on_hover_text("Fit the view to the population").clicked() {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    g.refit = true;
                }
            }
            if theme::toggle(ui, self.ws.options_open, "⚙ Options").clicked() {
                self.ws.options_open = !self.ws.options_open;
            }
        });
    }

    fn breadcrumb(&mut self, ui: &mut egui::Ui, id: u64, w: &GraphWindow) {
        let crumbs = self.session.breadcrumb(w.sample, w.node);
        let mut go: Option<NodeId> = None;
        ui.horizontal_wrapped(|ui| {
            for (i, (name, node)) in crumbs.iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("›").color(theme::MUTED));
                }
                let last = i + 1 == crumbs.len();
                let r = ui.add(egui::Label::new(RichText::new(name).size(12.5).color(if last { theme::ACCENT } else { theme::TEXT })).sense(Sense::click()));
                if !last && r.on_hover_text("Open this population").clicked() {
                    go = Some(*node);
                }
            }
            let total = self.session.samples()[w.sample].n_events.max(1);
            let c = self.session.count(w.sample, w.node);
            ui.add_space(10.0);
            theme::caption(ui, format!("{c} events · {:.1}% of sample", 100.0 * c as f64 / total as f64));
        });
        if let Some(node) = go {
            let _ = id;
            if let Err(e) = self.open_tab(w.sample, node) {
                self.status = e;
            }
        }
    }

    fn graph_options(&mut self, ui: &mut egui::Ui, id: u64, w: &GraphWindow) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.label(RichText::new("Plot options").strong().size(15.0));
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Population").default_open(true).show(ui, |ui| {
                let s = self.session.sample(w.sample);
                let (xi, yi) = (s.channel_index(&w.x_chan), s.channel_index(&w.y_chan));
                let version = self.session.version(w.sample);
                let key = hash_of((version, w.node, &w.x_chan, &w.y_chan));
                let cached = self.ws.graphs.get(&id).and_then(|g| g.summary.as_ref()).filter(|(k, _)| *k == key).map(|(_, v)| v.clone());
                let rows = match cached {
                    Some(r) => r,
                    None => {
                        let total = self.session.samples()[w.sample].n_events.max(1);
                        let c = self.session.count(w.sample, w.node);
                        let parent = self.session.tree(w.sample).parent(w.node).ok().flatten().map(|p| self.session.count(w.sample, p)).unwrap_or(total).max(1);
                        let mut rows = vec![("Events".to_string(), format!("{c}")), ("% of parent".to_string(), format!("{:.2}", 100.0 * c as f64 / parent as f64)), ("% of sample".to_string(), format!("{:.2}", 100.0 * c as f64 / total as f64))];
                        for (chan, idx) in [(&w.x_chan, xi), (&w.y_chan, yi)] {
                            if idx.is_some() && !(chan == &w.y_chan && w.kind == PlotKind::Histogram) {
                                if let Ok(vals) = self.session.population_values(w.sample, w.node, chan) {
                                    if !vals.is_empty() {
                                        rows.push((format!("{chan} median"), format!("{:.1}", stats::median(&vals))));
                                        rows.push((format!("{chan} mean"), format!("{:.1}", stats::mean(&vals))));
                                        rows.push((format!("{chan} CV %"), format!("{:.1}", stats::cv(&vals))));
                                    }
                                }
                            }
                        }
                        if let Some(g) = self.ws.graphs.get_mut(&id) {
                            g.summary = Some((key, rows.clone()));
                        }
                        rows
                    }
                };
                egui::Grid::new(("g_pop", id)).num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(k).color(theme::MUTED).size(12.0));
                        ui.label(RichText::new(v).monospace().size(12.0));
                        ui.end_row();
                    }
                });
            });
            egui::CollapsingHeader::new("View").default_open(true).show(ui, |ui| {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    if let Some(r) = g.range.as_mut() {
                        for (label, scale, lo, hi) in [("X", &w.x_scale, &mut r.x0, &mut r.x1), ("Y", &w.y_scale, &mut r.y0, &mut r.y1)] {
                            if label == "Y" && w.kind == PlotKind::Histogram {
                                continue;
                            }
                            if let Ok(t) = scale.build() {
                                let (mut a, mut b) = (t.inverse(*lo), t.inverse(*hi));
                                ui.label(RichText::new(format!("{label} range (data units)")).size(12.0).color(theme::MUTED));
                                let speed = ((b - a).abs() / 200.0).max(1e-9);
                                ui.horizontal(|ui| {
                                    let ra = ui.add(egui::DragValue::new(&mut a).speed(speed));
                                    ui.label("to");
                                    let rb = ui.add(egui::DragValue::new(&mut b).speed(speed));
                                    if ra.changed() || rb.changed() {
                                        let (na, nb) = (t.forward(a), t.forward(b));
                                        if na.is_finite() && nb.is_finite() && nb > na {
                                            *lo = na;
                                            *hi = nb;
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
                if ui.button("Fit to population").clicked() {
                    if let Some(g) = self.ws.graphs.get_mut(&id) {
                        g.refit = true;
                    }
                }
            });
            egui::CollapsingHeader::new("Gates").default_open(true).show(ui, |ui| {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    ui.horizontal(|ui| {
                        ui.label("Colour");
                        ui.color_edit_button_srgba(&mut g.style.color);
                    });
                    ui.add(egui::Slider::new(&mut g.style.width, 0.8..=4.0).text("Width"));
                    ui.checkbox(&mut g.style.labels, "Show names and %");
                    ui.checkbox(&mut g.style.fill, "Tint inside");
                }
            });
            egui::CollapsingHeader::new("Compensation").default_open(false).show(ui, |ui| {
                let s = self.session.sample(w.sample);
                match &s.comp {
                    Some(c) => {
                        ui.label(format!("Applied: {} ({} detectors)", c.source, c.detectors.len()));
                        theme::caption(ui, "Pick a Comp-… channel on an axis to use the compensated values.");
                    }
                    None => {
                        ui.label("No compensation applied.");
                        if ui.button("Open Compensation").clicked() {
                            self.module = super::Module::Compensation;
                        }
                    }
                }
            });
        });
    }

    fn graph_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: u64) {
        let Some(w) = self.session.window(id).cloned() else { return };
        let (channels, ranges) = {
            let s = self.session.sample(w.sample);
            (s.channels.clone(), s.ranges.clone())
        };
        let hist = w.kind == PlotKind::Histogram;
        // ---- Y axis column
        let mut changed = false;
        let (mut nx, mut ny, mut xs, mut ys) = (w.x_chan.clone(), w.y_chan.clone(), w.x_scale.clone(), w.y_scale.clone());
        if !hist {
            egui::SidePanel::left("g_y").exact_width(124.0).resizable(false).frame(egui::Frame::none()).show_inside(ui, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("Y axis").size(11.0).strong().color(theme::MUTED));
                changed |= axis_controls(ui, ("y", id), &channels, &ranges, &mut ny, &mut ys, 116.0);
            });
        }
        // ---- X axis row
        egui::TopBottomPanel::bottom("g_x").frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(4.0, 6.0))).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("X axis").size(11.0).strong().color(theme::MUTED));
                egui::ComboBox::from_id_salt(("x", id, "chan")).width(150.0).selected_text(nx.clone()).show_ui(ui, |ui| {
                    for c in &channels {
                        ui.selectable_value(&mut nx, c.clone(), c);
                    }
                });
                if nx != w.x_chan {
                    let r = channels.iter().position(|c| *c == nx).map_or(262_144.0, |i| ranges[i]);
                    xs = default_scale(&nx, r);
                    changed = true;
                }
                // reuse the compact scale picker for X (its channel combo above is the primary one)
                let mut dummy = nx.clone();
                let c2 = axis_controls_scale_only(ui, ("x", id), &mut dummy, &mut xs, &ranges, &channels);
                changed |= c2;
            });
        });
        if changed {
            if let Some(win) = self.session.window_mut(id) {
                win.x_chan = nx;
                win.y_chan = ny;
                win.x_scale = xs;
                win.y_scale = ys;
                win.drawing.cancel();
            }
            if let Some(g) = self.ws.graphs.get_mut(&id) {
                g.refit = true;
            }
        }
        let w = self.session.window(id).cloned().unwrap_or(w);
        self.ws.graphs.entry(id).or_default();

        // ---- range
        let refit = {
            let g = &self.ws.graphs[&id];
            g.refit || g.range.is_none()
        };
        if refit {
            let r = self.fit_range(id, &w);
            let g = self.ws.graphs.get_mut(&id).unwrap();
            g.range = r;
            g.refit = false;
        }
        let Some(range) = self.ws.graphs[&id].range.filter(|r| r.is_valid()) else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("This population has no events on these axes.").color(theme::MUTED)));
            return;
        };

        // ---- plot rectangle
        let avail = ui.available_rect_before_wrap();
        let plot = Rect::from_min_max(Pos2::new(avail.left() + 52.0, avail.top() + 8.0), Pos2::new(avail.right() - 12.0, avail.bottom() - 34.0));
        if plot.width() < 80.0 || plot.height() < 80.0 {
            return;
        }
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());
        let painter = ui.painter_at(avail);
        let mut map = Mapper { rect: plot, range };
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.plot_rect = Some(plot);
        }
        painter.rect_filled(plot, 0.0, Color32::WHITE);
        self.paint_population(ctx, id, &w, &map, &painter);
        draw_axes(&painter, &map, &w, hist);
        let style = self.ws.graphs[&id].style.clone();
        self.draw_child_gates(&painter, &map, &w, &style);
        draw_in_progress(&painter, &map, &w.drawing, w.tool);

        // ---- zoom & pan
        let hovered_in_plot = resp.hover_pos().filter(|p| plot.contains(*p));
        if let Some(p) = hovered_in_plot {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                let f = (1.0 - scroll as f64 * 0.0015).clamp(0.5, 1.6);
                let c = map.to_data(p);
                let r = &mut self.ws.graphs.get_mut(&id).unwrap().range;
                if let Some(r) = r.as_mut() {
                    r.x0 = c.0 + (r.x0 - c.0) * f;
                    r.x1 = c.0 + (r.x1 - c.0) * f;
                    if !hist {
                        r.y0 = c.1 + (r.y0 - c.1) * f;
                        r.y1 = c.1 + (r.y1 - c.1) * f;
                    }
                    map.range = *r;
                }
            }
        }
        if w.tool == Tool::Pointer && resp.dragged() {
            let d = resp.drag_delta();
            let (dx, dy) = ((d.x / plot.width()) as f64 * (range.x1 - range.x0), (d.y / plot.height()) as f64 * (range.y1 - range.y0));
            if let Some(r) = self.ws.graphs.get_mut(&id).unwrap().range.as_mut() {
                r.x0 -= dx;
                r.x1 -= dx;
                if !hist {
                    r.y0 += dy;
                    r.y1 += dy;
                }
            }
        }

        // ---- hover read-out
        if let Some(pos) = hovered_in_plot {
            let d = map.to_data(pos);
            if let Some(win) = self.session.window_mut(id) {
                win.drawing.move_to(d);
            }
            let tx = w.x_scale.build().map(|t| t.inverse(d.0)).unwrap_or(f64::NAN);
            let ty = w.y_scale.build().map(|t| t.inverse(d.1)).unwrap_or(f64::NAN);
            let text = if hist { format!("{}: {:.0}", w.x_chan, tx) } else { format!("{}: {:.0}    {}: {:.0}", w.x_chan, tx, w.y_chan, ty) };
            painter.text(Pos2::new(plot.right() - 6.0, plot.top() + 4.0), Align2::RIGHT_TOP, text, FontId::proportional(11.0), Color32::from_gray(70));
            if w.tool == Tool::Quadrant {
                let s = Stroke::new(1.2_f32, theme::ACCENT);
                painter.line_segment([Pos2::new(pos.x, plot.top()), Pos2::new(pos.x, plot.bottom())], s);
                painter.line_segment([Pos2::new(plot.left(), pos.y), Pos2::new(plot.right(), pos.y)], s);
            }
        }

        // ---- gate drawing
        let pointer = resp.interact_pointer_pos().filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
        let mut finished: Option<Shape> = None;
        let mut quadrant_at: Option<(f64, f64)> = None;
        let mut clicked_gate: Option<(Option<NodeId>, bool)> = None;
        if let Some(win) = self.session.window_mut(id) {
            match win.tool {
                Tool::Pointer => {
                    if resp.clicked() || resp.double_clicked() {
                        if let Some(p) = pointer {
                            clicked_gate = Some((None, resp.double_clicked()));
                            let _ = p;
                        }
                    }
                }
                Tool::Quadrant => {
                    if resp.clicked() {
                        quadrant_at = pointer;
                    }
                }
                Tool::Polygon => {
                    if resp.clicked() {
                        // egui calls any two quick clicks a double click regardless of distance, which would close the
                        // polygon while vertices are still being placed; require the same spot as well
                        let (now, at) = (ctx.input(|i| i.time), resp.interact_pointer_pos());
                        let last = self.ws.graphs.get(&id).and_then(|g| g.last_click);
                        let closing = matches!((last, at), (Some((t, q)), Some(p)) if now - t < 0.4 && q.distance(p) < 6.0);
                        if closing {
                            finished = win.drawing.finish_polygon();
                        } else if let Some(p) = pointer {
                            win.drawing.press(Tool::Polygon, p);
                        }
                        if let Some(g) = self.ws.graphs.get_mut(&id) {
                            g.last_click = at.map(|p| (now, p));
                        }
                    }
                }
                tool => {
                    if resp.drag_started() {
                        // egui reports the start only after the pointer has moved past a small threshold; the gate must
                        // begin where the button went DOWN
                        let origin = ctx.input(|i| i.pointer.press_origin()).filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
                        if let Some(p) = origin.or(pointer) {
                            win.drawing.press(tool, p);
                        }
                    }
                    if resp.drag_stopped() {
                        if let Some(p) = resp.interact_pointer_pos().map(|p| map.to_data(Pos2::new(p.x.clamp(plot.left(), plot.right()), p.y.clamp(plot.top(), plot.bottom())))) {
                            finished = win.drawing.release(tool, p);
                        }
                    }
                }
            }
        }
        if let Some((_, dbl)) = clicked_gate {
            if let Some(pos) = resp.interact_pointer_pos() {
                self.pick_gate(&w, map.to_data(pos), dbl);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if let Some(win) = self.session.window_mut(id) {
                win.drawing.cancel();
            }
        }
        if w.tool == Tool::Polygon && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Some(win) = self.session.window_mut(id) {
                finished = win.drawing.finish_polygon();
            }
        }
        if let Some((cx, cy)) = quadrant_at {
            self.status = match self.session.commit_quadrants(id, cx, cy) {
                Ok(ids) => format!("Added {} quadrant populations. Ctrl+Z to undo.", ids.len()),
                Err(e) => format!("Could not add the quadrant: {e}"),
            };
            if let Some(g) = self.ws.graphs.get_mut(&id) {
                g.key = 0;
            }
        }
        if let Some(shape) = finished {
            let name = self.session.next_name(w.sample, w.node);
            match self.session.commit_shape(id, &shape, &name) {
                Ok(nid) => {
                    let c = self.session.count(w.sample, nid);
                    self.session.selected = Some((w.sample, nid));
                    self.status = format!("Added '{name}' — {c} events. Double-click the gate to open it; Ctrl+Z to undo.");
                    if let Some(g) = self.ws.graphs.get_mut(&id) {
                        g.key = 0;
                    }
                }
                Err(e) => self.status = format!("Could not add the gate: {e}"),
            }
        }
        // the in-progress hint under the cursor
        if self.session.window(id).map_or(false, |w| w.drawing.is_active()) {
            ctx.request_repaint();
        }
    }

    /// Select (and on double-click open) the topmost child gate under a display-space point.
    fn pick_gate(&mut self, w: &GraphWindow, at: (f64, f64), open: bool) {
        let kids: Vec<NodeId> = self.session.tree(w.sample).children(w.node).map(|c| c.to_vec()).unwrap_or_default();
        let mut hit = None;
        for k in kids {
            if let Ok(NodeKind::Gate(g)) = self.session.tree(w.sample).kind(k) {
                if gate_on_axes(g, w) && g.contains(at.0, at.1) != g.invert {
                    hit = Some(k);
                }
            }
        }
        match hit {
            Some(k) => {
                self.session.selected = Some((w.sample, k));
                if open {
                    if let Err(e) = self.open_tab(w.sample, k) {
                        self.status = e;
                    }
                }
            }
            None if !open => self.session.selected = Some((w.sample, w.node)),
            None => {}
        }
    }

    fn fit_range(&mut self, id: u64, w: &GraphWindow) -> Option<Range> {
        let pts = self.session.window_points(id).ok()?;
        let finite: Vec<&(f64, f64)> = pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        let (x0, x1) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (y0, y1) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        let pad = |lo: f64, hi: f64| {
            let span = if hi > lo { hi - lo } else { 1.0 };
            (lo - 0.01 * span, hi + 0.01 * span)
        };
        let ((x0, x1), (y0, y1)) = (pad(x0, x1), pad(y0, y1));
        let _ = w;
        Some(Range { x0, x1, y0, y1 })
    }

    /// Density / dots / histogram of the window's population into the plot rectangle.
    fn paint_population(&mut self, ctx: &egui::Context, id: u64, w: &GraphWindow, map: &Mapper, painter: &egui::Painter) {
        let range = map.range;
        let version = self.session.version(w.sample);
        let pkey = hash_of((id, version, w.node, &w.x_chan, &w.y_chan, w.x_scale.to_text(), w.y_scale.to_text(), w.sample));
        let have = self.ws.graphs.get(&id).and_then(|g| g.pts.as_ref()).map_or(false, |(k, _)| *k == pkey);
        if !have {
            match self.session.window_points(id) {
                Ok(p) => {
                    if let Some(g) = self.ws.graphs.get_mut(&id) {
                        g.pts = Some((pkey, p));
                    }
                }
                Err(e) => {
                    self.status = e;
                    return;
                }
            }
        }
        let gw = ((map.rect.width() / 2.0) as usize / 16 * 16).clamp(128, 512);
        let gh = ((map.rect.height() / 2.0) as usize / 16 * 16).clamp(128, 512);
        let key = hash_of((pkey, range.x0.to_bits(), range.x1.to_bits(), range.y0.to_bits(), range.y1.to_bits(), gw, gh, w.kind as u8));
        let stale = self.ws.graphs.get(&id).map_or(true, |g| g.key != key);
        if stale {
            let Some(g) = self.ws.graphs.get_mut(&id) else { return };
            let pts = &g.pts.as_ref().unwrap().1;
            if w.kind == PlotKind::Histogram {
                let h = histogram(pts.iter().map(|p| p.0), range.x0, range.x1, gw);
                // light smoothing so a noisy histogram reads as a distribution
                let sm: Vec<f32> = (0..h.len())
                    .map(|i| {
                        let (a, b) = (i.saturating_sub(1), (i + 1).min(h.len() - 1));
                        (h[a] as f32 + 2.0 * h[i] as f32 + h[b] as f32) / 4.0
                    })
                    .collect();
                let mx = sm.iter().cloned().fold(0.0f32, f32::max).max(1.0);
                g.hist = sm.into_iter().map(|v| v / mx).collect();
                g.shown = h.iter().map(|&c| c as usize).sum();
                g.tex = None;
            } else {
                let grid = bin(pts.iter().copied(), range, gw, gh);
                let px = if w.kind == PlotKind::Dot { render_dots(&grid, [31, 111, 235]) } else { render(&grid) };
                let pixels: Vec<Color32> = px.into_iter().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
                let tex = ctx.load_texture(format!("graph{id}"), egui::ColorImage { size: [gw, gh], pixels }, TextureOptions::NEAREST);
                g.tex = Some(tex);
                g.shown = grid.inside;
                g.hist.clear();
            }
            g.key = key;
        }
        let Some(g) = self.ws.graphs.get(&id) else { return };
        if w.kind == PlotKind::Histogram {
            let n = g.hist.len().max(1);
            let mut pts: Vec<Pos2> = Vec::with_capacity(n + 2);
            pts.push(Pos2::new(map.rect.left(), map.rect.bottom()));
            for (i, v) in g.hist.iter().enumerate() {
                let x = map.rect.left() + (i as f32 + 0.5) / n as f32 * map.rect.width();
                pts.push(Pos2::new(x, map.rect.bottom() - v * 0.94 * map.rect.height()));
            }
            pts.push(Pos2::new(map.rect.right(), map.rect.bottom()));
            painter.add(egui::Shape::convex_polygon(vec![], Color32::TRANSPARENT, Stroke::NONE)); // no-op placeholder keeps z-order stable
            // filled as vertical strips (a concave outline cannot use convex_polygon)
            for i in 0..g.hist.len() {
                let x0 = map.rect.left() + i as f32 / n as f32 * map.rect.width();
                let x1 = map.rect.left() + (i + 1) as f32 / n as f32 * map.rect.width();
                let top = map.rect.bottom() - g.hist[i] * 0.94 * map.rect.height();
                painter.rect_filled(Rect::from_min_max(Pos2::new(x0, top), Pos2::new(x1 + 0.5, map.rect.bottom())), 0.0, Color32::from_rgba_unmultiplied(0, 140, 200, 120));
            }
            painter.add(egui::Shape::line(pts[1..pts.len() - 1].to_vec(), Stroke::new(1.4_f32, Color32::from_rgb(0, 100, 160))));
        } else if let Some(tex) = &g.tex {
            painter.image(tex.id(), map.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
    }

    fn draw_child_gates(&mut self, painter: &egui::Painter, map: &Mapper, w: &GraphWindow, style: &GateStyle) {
        let kids: Vec<NodeId> = self.session.tree(w.sample).children(w.node).map(|c| c.to_vec()).unwrap_or_default();
        let parent_count = self.session.count(w.sample, w.node).max(1);
        let painter = painter.with_clip_rect(map.rect.expand(2.0));
        for k in kids {
            let (name, gate) = match (self.session.tree(w.sample).name(k), self.session.tree(w.sample).kind(k)) {
                (Ok(n), Ok(NodeKind::Gate(g))) => (n.to_string(), g.clone()),
                _ => continue,
            };
            // a gate is only drawn on the axes (and scales) it was made on
            if !gate_on_axes(&gate, w) {
                continue;
            }
            let selected = self.session.selected == Some((w.sample, k));
            let stroke = Stroke::new(if selected { style.width + 1.2 } else { style.width }, if selected { Color32::from_rgb(255, 214, 10) } else { style.color });
            let anchor = draw_gate(&painter, map, &gate, stroke, w.kind == PlotKind::Histogram, style.fill);
            if style.labels {
                if let Some(a) = anchor {
                    let count = self.session.count(w.sample, k);
                    let text = format!("{name}\n{:.1}%  ({count})", 100.0 * count as f64 / parent_count as f64);
                    painter.text(a + egui::vec2(3.0, -3.0), Align2::LEFT_BOTTOM, text, FontId::proportional(11.0), Color32::from_rgb(120, 15, 15));
                }
            }
        }
    }
}

/// Is this gate drawn on exactly the window's current axes and scales?
fn gate_on_axes(g: &Gate, w: &GraphWindow) -> bool {
    if matches!(g.kind, GateKind::Indexed(_)) {
        return false;
    }
    if g.x_chan != w.x_chan || g.x_scale != w.x_scale {
        return false;
    }
    match &g.y_chan {
        Some(y) => w.kind != PlotKind::Histogram && *y == w.y_chan && g.y_scale == w.y_scale,
        None => true,
    }
}

fn axis_controls_scale_only(ui: &mut egui::Ui, id: (&str, u64), _chan: &mut String, scale: &mut ScaleSpec, ranges: &[f64], channels: &[String]) -> bool {
    let before = scale.clone();
    let range = channels.first().map_or(262_144.0, |_| ranges.iter().cloned().fold(0.0, f64::max).max(262_144.0));
    egui::ComboBox::from_id_salt((id, "scale")).width(86.0).selected_text(scale_name(scale)).show_ui(ui, |ui| {
        if ui.selectable_label(matches!(scale, ScaleSpec::Identity), "Linear").clicked() {
            *scale = ScaleSpec::Identity;
        }
        if ui.selectable_label(matches!(scale, ScaleSpec::Arcsinh { .. }), "Arcsinh").clicked() {
            *scale = ScaleSpec::Arcsinh { cofactor: 150.0 };
        }
        if ui.selectable_label(matches!(scale, ScaleSpec::Logicle { .. }), "Logicle").clicked() {
            *scale = ScaleSpec::Logicle { t: range, w: 0.5, m: 4.5, a: 0.0 };
        }
    });
    ui.menu_button("⚙", |ui| {
        ui.set_min_width(190.0);
        match scale {
            ScaleSpec::Arcsinh { cofactor } => {
                ui.label("Cofactor");
                ui.add(egui::DragValue::new(cofactor).speed(1.0).range(0.1..=100_000.0));
            }
            ScaleSpec::Logicle { t, w, m, a } => {
                ui.label("Top of scale (T)");
                ui.add(egui::DragValue::new(t).speed(100.0).range(1.0..=1.0e9));
                ui.label("Linear width (W)");
                ui.add(egui::DragValue::new(w).speed(0.01).range(0.0..=2.0));
                ui.label("Decades (M)");
                ui.add(egui::DragValue::new(m).speed(0.05).range(1.0..=8.0));
                ui.label("Extra negative (A)");
                ui.add(egui::DragValue::new(a).speed(0.05).range(0.0..=3.0));
                if 2.0 * *w > *m {
                    *w = *m / 2.0;
                }
            }
            _ => {
                ui.label(RichText::new("Linear axes have no parameters.").color(theme::MUTED));
            }
        }
    });
    *scale != before
}

fn draw_axes(p: &egui::Painter, map: &Mapper, w: &GraphWindow, hist: bool) {
    let axis = Stroke::new(1.0_f32, Color32::from_gray(150));
    let grid = Stroke::new(0.5_f32, Color32::from_gray(228));
    let text = theme::TEXT;
    let r = map.rect;
    for t in ticks(&w.x_scale, map.range.x0, map.range.x1) {
        let x = map.to_screen((t.pos, map.range.y0)).x;
        if t.major {
            p.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], grid);
            p.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x, r.bottom() + 5.0)], axis);
            p.text(Pos2::new(x, r.bottom() + 7.0), Align2::CENTER_TOP, &t.label, FontId::proportional(10.5), text);
        } else {
            p.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x, r.bottom() + 2.5)], axis);
        }
    }
    if !hist {
        for t in ticks(&w.y_scale, map.range.y0, map.range.y1) {
            let y = map.to_screen((map.range.x0, t.pos)).y;
            if t.major {
                p.line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], grid);
                p.line_segment([Pos2::new(r.left() - 5.0, y), Pos2::new(r.left(), y)], axis);
                p.text(Pos2::new(r.left() - 8.0, y), Align2::RIGHT_CENTER, &t.label, FontId::proportional(10.5), text);
            } else {
                p.line_segment([Pos2::new(r.left() - 2.5, y), Pos2::new(r.left(), y)], axis);
            }
        }
    } else {
        p.text(Pos2::new(r.left() - 8.0, r.center().y), Align2::RIGHT_CENTER, "count", FontId::proportional(10.5), theme::MUTED);
    }
    p.rect_stroke(r, 0.0, axis);
    p.text(Pos2::new(r.center().x, r.bottom() + 20.0), Align2::CENTER_TOP, &w.x_chan, FontId::proportional(11.5), text);
    if !hist {
        p.text(Pos2::new(r.left() + 4.0, r.top() + 3.0), Align2::LEFT_TOP, &w.y_chan, FontId::proportional(11.5), Color32::from_gray(60));
    }
}

/// Draw a gate; returns a good anchor for its label.
fn draw_gate(p: &egui::Painter, map: &Mapper, g: &Gate, stroke: Stroke, hist: bool, fill: bool) -> Option<Pos2> {
    let lo = |v: &[f64]| v.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = |v: &[f64]| v.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let tint = Color32::from_rgba_unmultiplied(stroke.color.r(), stroke.color.g(), stroke.color.b(), 28);
    match &g.kind {
        GateKind::Rectangle => {
            let (a, b) = (map.to_screen((lo(&g.xs), lo(&g.ys))), map.to_screen((hi(&g.xs), hi(&g.ys))));
            let r = Rect::from_two_pos(a, b);
            if fill {
                p.rect_filled(r, 0.0, tint);
            }
            p.rect_stroke(r, 0.0, stroke);
            Some(r.left_top())
        }
        GateKind::Interval => {
            let (a, b) = (map.to_screen((lo(&g.xs), map.range.y0)), map.to_screen((hi(&g.xs), map.range.y1)));
            if fill || hist {
                p.rect_filled(Rect::from_min_max(Pos2::new(a.x, map.rect.top()), Pos2::new(b.x, map.rect.bottom())), 0.0, tint);
            }
            p.line_segment([Pos2::new(a.x, map.rect.top()), Pos2::new(a.x, map.rect.bottom())], stroke);
            p.line_segment([Pos2::new(b.x, map.rect.top()), Pos2::new(b.x, map.rect.bottom())], stroke);
            Some(Pos2::new(a.x, map.rect.top() + 14.0))
        }
        GateKind::Ellipse => {
            let (cx, cy) = ((lo(&g.xs) + hi(&g.xs)) / 2.0, (lo(&g.ys) + hi(&g.ys)) / 2.0);
            let (rx, ry) = ((hi(&g.xs) - lo(&g.xs)) / 2.0, (hi(&g.ys) - lo(&g.ys)) / 2.0);
            let (c, s) = (g.angle.cos(), g.angle.sin());
            let pts: Vec<Pos2> = (0..72)
                .map(|k| {
                    let t = k as f64 * std::f64::consts::TAU / 72.0;
                    let (ex, ey) = (rx * t.cos(), ry * t.sin());
                    map.to_screen((cx + ex * c - ey * s, cy + ex * s + ey * c))
                })
                .collect();
            if fill {
                p.add(egui::Shape::convex_polygon(pts.clone(), tint, Stroke::NONE));
            }
            p.add(egui::Shape::closed_line(pts.clone(), stroke));
            pts.iter().min_by(|a, b| a.y.partial_cmp(&b.y).unwrap()).copied()
        }
        GateKind::Polygon => {
            let pts: Vec<Pos2> = g.xs.iter().zip(&g.ys).map(|(&x, &y)| map.to_screen((x, y))).collect();
            p.add(egui::Shape::closed_line(pts.clone(), stroke));
            pts.iter().min_by(|a, b| a.y.partial_cmp(&b.y).unwrap()).copied()
        }
        GateKind::Quadrant(q) => {
            let c = map.to_screen((g.xs[0], g.ys[0]));
            let (r, t) = (map.rect, stroke);
            p.line_segment([Pos2::new(c.x, r.top()), Pos2::new(c.x, r.bottom())], t);
            p.line_segment([Pos2::new(r.left(), c.y), Pos2::new(r.right(), c.y)], t);
            use sf_gating::Quad;
            Some(match q {
                Quad::Q1 => Pos2::new(c.x + 6.0, r.top() + 26.0),
                Quad::Q2 => Pos2::new(r.left() + 6.0, r.top() + 26.0),
                Quad::Q3 => Pos2::new(r.left() + 6.0, r.bottom() - 4.0),
                Quad::Q4 => Pos2::new(c.x + 6.0, r.bottom() - 4.0),
            })
        }
        GateKind::Indexed(_) => None,
    }
}

fn draw_in_progress(p: &egui::Painter, map: &Mapper, d: &Drawing, tool: Tool) {
    let stroke = Stroke::new(1.5_f32, Color32::from_rgb(10, 124, 255));
    let pts = d.points();
    match tool {
        Tool::Polygon => {
            let mut s: Vec<Pos2> = pts.iter().map(|&q| map.to_screen(q)).collect();
            if let Some(h) = d.hover().filter(|_| !pts.is_empty()) {
                s.push(map.to_screen(h));
            }
            if s.len() >= 2 {
                p.add(egui::Shape::line(s.clone(), stroke));
            }
            for q in s {
                p.circle_filled(q, 2.5, stroke.color);
            }
        }
        Tool::Rectangle | Tool::Ellipse | Tool::Interval if pts.len() == 2 => {
            let (a, b) = (map.to_screen(pts[0]), map.to_screen(pts[1]));
            match tool {
                Tool::Rectangle => {
                    p.rect_stroke(Rect::from_two_pos(a, b), 0.0, stroke);
                }
                Tool::Interval => {
                    p.line_segment([Pos2::new(a.x, map.rect.top()), Pos2::new(a.x, map.rect.bottom())], stroke);
                    p.line_segment([Pos2::new(b.x, map.rect.top()), Pos2::new(b.x, map.rect.bottom())], stroke);
                }
                _ => {
                    let r = Rect::from_two_pos(a, b);
                    let pts: Vec<Pos2> = (0..72)
                        .map(|k| {
                            let t = k as f32 * std::f32::consts::TAU / 72.0;
                            r.center() + egui::vec2(r.width() / 2.0 * t.cos(), r.height() / 2.0 * t.sin())
                        })
                        .collect();
                    p.add(egui::Shape::closed_line(pts, stroke));
                }
            }
        }
        _ => {}
    }
}
