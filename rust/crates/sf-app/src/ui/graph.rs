//! One graph tab: toolbar, breadcrumb, axis controls, the density canvas with gates, and the options panel.

use super::{cross_view::short_all, theme, workstation::Tab, App};
use crate::axis::ticks;
use crate::density::{bin, hist_path, hist_ticks, histogram, histogram_curve, render_kind, HistCurve, HistStyle, PlotKind, PlotStyle, Range};
use crate::drawing::{Drawing, Shape, Tool};
use crate::gate_edit::{self, Handle, Side};
use crate::session::{default_scale, GraphWindow};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions};
use sf_core::scale::ScaleSpec;
use sf_core::stats;
use sf_gating::{Gate, GateKind, GateTree, NodeId, NodeKind};
use std::collections::HashMap;

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
    /// Draw the FMO reference lines.
    pub fmo: bool,
    /// Show each gate's % of parent and event count / its median(s).
    pub freqs: bool,
    pub medians: bool,
}

impl Default for GateStyle {
    fn default() -> Self {
        GateStyle { color: theme::GATE_RED, width: 1.8, labels: true, fill: false, fmo: true, freqs: true, medians: false }
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
    /// A gate being moved or resized: nothing is committed until the button is released.
    pub edit: Option<EditDrag>,
    /// Dragging an FMO line: `true` = the X channel's line.
    pub fmo_drag: Option<bool>,
    /// Dragging a quadrant gate's crosshair (or one of its two lines).
    pub quad_drag: Option<QuadDrag>,
    /// The compare strip under the plot is open.
    pub strip_open: bool,
    pub strip: HashMap<usize, (u64, StripThumb)>,
    /// Where each sample's card was drawn last frame (for hit-testing and tests).
    pub strip_rects: Vec<(usize, Rect)>,
    /// The whole canvas (plot and axis labels) as last drawn, for exporting what is shown.
    pub canvas_rect: Option<Rect>,
    /// Histogram, contour and density options of this plot.
    pub plot: PlotStyle,
    /// Where the last right-click on the plot landed, and the gate under it.
    pub menu: Option<super::gate_menu::MenuTarget>,
    /// The histogram as last computed (heights, Y axis), for drawing and exporting.
    pub curve: Option<HistCurve>,
    /// Where each child gate's name was drawn last frame: a right-click or double-click there means that gate.
    pub label_rects: Vec<(NodeId, Rect)>,
    /// The two-parameter plot type shown before switching to a histogram, to go back to.
    pub last_2d: Option<PlotKind>,
}

/// One sample's picture in the compare strip.
#[derive(Clone)]
pub enum StripThumb {
    Tex(TextureHandle),
    Hist(Vec<f32>),
}

/// One in-progress move of a quadrant crosshair: all four populations follow when the button is released.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadDrag {
    pub node: NodeId,
    /// The crosshair before the drag, and where it would be now.
    pub orig: (f64, f64),
    pub now: (f64, f64),
    /// Which of the two coordinates this drag changes (a line moves one, the centre both).
    pub moves: (bool, bool),
}

/// One in-progress move / resize of a gate.
pub struct EditDrag {
    pub node: NodeId,
    pub handle: Handle,
    pub start: (f64, f64),
    pub orig: Gate,
    /// The gate as it would be at the pointer's current position (`None` while the result would be degenerate).
    pub preview: Option<Gate>,
}

impl GraphUi {
    pub fn new() -> GraphUi {
        GraphUi { tex: None, key: 0, range: None, shown: 0, refit: true, plot_rect: None, last_click: None, style: GateStyle::default(), pts: None, summary: None, edit: None, fmo_drag: None, quad_drag: None, strip_open: false, strip: HashMap::new(), strip_rects: Vec::new(), canvas_rect: None, plot: PlotStyle::default(), menu: None, curve: None, label_rects: Vec::new(), last_2d: None }
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

/// The channel list of an axis menu, in sections so derived channels are easy to find: unmixed (spectral), then
/// compensated, then the measured channels. Headings appear only when a section is not the only one.
/// `labels[i]` is what is shown for `channels[i]` (the channel with its marker).
pub(super) fn channel_list(ui: &mut egui::Ui, chan: &mut String, channels: &[String], labels: &[String]) {
    let shown_as = |c: &String| channels.iter().position(|x| x == c).and_then(|i| labels.get(i)).cloned().unwrap_or_else(|| c.clone());
    use crate::sample::{COMP_PREFIX, UNMIX_PREFIX};
    let sections: [(&str, Vec<&String>); 3] = [
        ("Unmixed", channels.iter().filter(|c| c.starts_with(UNMIX_PREFIX)).collect()),
        ("Compensated", channels.iter().filter(|c| c.starts_with(COMP_PREFIX)).collect()),
        ("Measured", channels.iter().filter(|c| !c.starts_with(UNMIX_PREFIX) && !c.starts_with(COMP_PREFIX)).collect()),
    ];
    let shown = sections.iter().filter(|(_, v)| !v.is_empty()).count();
    for (title, list) in &sections {
        if list.is_empty() {
            continue;
        }
        if shown > 1 {
            ui.label(RichText::new(format!("{title}  ({})", list.len())).size(10.5).strong().color(if *title == "Unmixed" { theme::ACCENT } else { theme::MUTED }));
        }
        for c in list {
            ui.selectable_value(chan, (*c).clone(), shown_as(c));
        }
    }
}

/// Channel + scale pickers for one axis. Returns true when anything changed.
/// `histogram`, when given, adds a "Histogram" entry above the channels and is set when it is picked.
#[allow(clippy::too_many_arguments)]
fn axis_controls(ui: &mut egui::Ui, id: (&str, u64), channels: &[String], labels: &[String], ranges: &[f64], chan: &mut String, scale: &mut ScaleSpec, width: f32, histogram: Option<&mut bool>) -> bool {
    let (before_c, before_s) = (chan.clone(), scale.clone());
    let shown = channels.iter().position(|c| c == chan).and_then(|i| labels.get(i)).cloned().unwrap_or_else(|| chan.clone());
    egui::ComboBox::from_id_salt((id, "chan")).width(width).truncate().selected_text(shown).show_ui(ui, |ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            if let Some(h) = histogram {
                if ui.selectable_label(false, "Histogram").on_hover_text("Show the X channel alone, as a histogram").clicked() {
                    *h = true;
                }
                ui.separator();
            }
            channel_list(ui, chan, channels, labels)
        });
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

        egui::TopBottomPanel::top(egui::Id::new(("g_tools", id))).resizable(false).exact_height(78.0).frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 6.0))).show_inside(ui, |ui| self.graph_toolbar(ui, id, &w));
        egui::TopBottomPanel::top(egui::Id::new(("g_crumb", id))).frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::symmetric(12.0, 4.0))).show_inside(ui, |ui| self.breadcrumb(ui, id, &w));
        egui::TopBottomPanel::bottom(egui::Id::new(("g_hint", id))).frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(10.0, 3.0))).show_inside(ui, |ui| {
            ui.add(egui::Label::new(RichText::new("P polygon · R rectangle · E ellipse · I interval · Q quadrant · N pointer · Del remove gate · Ctrl+Z / Ctrl+Y undo / redo · ← → sample · wheel zoom · drag a gate or its handles to move or resize it · drag empty space to pan").size(11.0).color(theme::MUTED)).truncate());
        });
        let strip_open = self.ws.graphs.get(&id).map_or(false, |g| g.strip_open);
        egui::TopBottomPanel::bottom(egui::Id::new(("g_strip", id))).resizable(false).exact_height(236.0).frame(egui::Frame::none().fill(theme::BG).inner_margin(egui::Margin::symmetric(10.0, 6.0))).show_animated_inside(ui, strip_open, |ui| self.compare_strip(ui, ctx, id, &w));
        let open = self.ws.options_open;
        if open {
            // floats over the right edge instead of taking width from the plot, so showing it never reshapes the graph
            let r = ui.available_rect_before_wrap();
            let width = 250.0;
            egui::Area::new(egui::Id::new(("g_opts", id))).order(egui::Order::Middle).fixed_pos(Pos2::new((r.right() - width - 24.0).max(r.left()), r.top() + 6.0)).show(ui.ctx(), |ui| {
                egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).stroke(Stroke::new(1.0_f32, Color32::from_gray(70))).inner_margin(egui::Margin::same(10.0)).show(ui, |ui| {
                    ui.set_width(width);
                    egui::ScrollArea::vertical().id_salt(("g_opts_scroll", id)).max_height((r.height() - 36.0).max(160.0)).show(ui, |ui| self.graph_options(ui, id, &w));
                });
            });
        }
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
        let alt = ctx.input_mut(|i| {
            let m = egui::Modifiers::ALT;
            (i.consume_key(m, egui::Key::T), i.consume_key(m, egui::Key::ArrowRight), i.consume_key(m, egui::Key::ArrowLeft), i.consume_key(m, egui::Key::ArrowDown), i.consume_key(m, egui::Key::ArrowUp))
        });
        match alt {
            (true, ..) => self.axis_action(id, true, 0),
            (_, true, ..) => self.axis_action(id, true, 1),
            (_, _, true, ..) => self.axis_action(id, true, -1),
            (_, _, _, true, _) => self.axis_action(id, false, 1),
            (.., true) => self.axis_action(id, false, -1),
            _ => {}
        }
        let (left, right, del) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight), i.key_pressed(egui::Key::Delete)));
        if left {
            if let Some(p) = self.step_sample(w.sample, -1) {
                self.hop_sample(id, p);
            }
        }
        if right {
            if let Some(p) = self.step_sample(w.sample, 1) {
                self.hop_sample(id, p);
            }
        }
        if del {
            if let Some((s, n)) = self.session.selected {
                let is_child = s == w.sample && self.session.tree(s).parent(n).ok().flatten() == Some(w.node);
                if is_child {
                    self.gate_delete(s, n);
                }
            }
        }
    }

    /// The child gate whose name label (as drawn last frame) is under a screen position.
    pub fn gate_label_at(&self, id: u64, p: Pos2) -> Option<NodeId> {
        self.ws.graphs.get(&id)?.label_rects.iter().rev().find(|(_, r)| r.contains(p)).map(|(k, _)| *k)
    }

    pub(crate) fn do_undo(&mut self, sample: usize) {
        self.status = if self.session.undo(sample) { "Undone.".into() } else { "Nothing to undo.".into() };
    }

    pub(crate) fn do_redo(&mut self, sample: usize) {
        self.status = if self.session.redo(sample) { "Redone.".into() } else { "Nothing to redo.".into() };
    }

    /// Transpose (`dir == 0`) or step an axis to its next/previous channel, then refit the view.
    pub fn axis_action(&mut self, id: u64, x_axis: bool, dir: i32) {
        let r = if dir == 0 { self.session.transpose_axes(id).map(|_| "Axes swapped".to_string()) } else { self.session.step_axis(id, x_axis, dir).map(|n| format!("{} axis: {n}", if x_axis { "X" } else { "Y" })) };
        match r {
            Ok(m) => {
                self.status = m;
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    g.refit = true;
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Put this window on unmixed markers: the first two, then each press steps to the next pair.
    fn unmixed_axes(&mut self, id: u64) {
        let Some(w) = self.session.window(id).cloned() else { return };
        let chans: Vec<String> = self.session.sample(w.sample).channels.iter().filter(|c| c.starts_with(crate::sample::UNMIX_PREFIX) && !c.ends_with("Residual") && !c.ends_with("-AF")).cloned().collect();
        if chans.len() < 2 {
            self.status = "This sample has fewer than two unmixed markers.".into();
            return;
        }
        let at = chans.iter().position(|c| *c == w.x_chan).map_or(0, |i| (i + 1) % chans.len());
        let (cx, cy) = (chans[at].clone(), chans[(at + 1) % chans.len()].clone());
        let (xr, yr) = {
            let s = self.session.sample(w.sample);
            (s.range_of(&cx), s.range_of(&cy))
        };
        if let Some(win) = self.session.window_mut(id) {
            win.x_scale = crate::session::default_scale(&cx, xr);
            win.y_scale = crate::session::default_scale(&cy, yr);
            win.x_chan = cx.clone();
            win.y_chan = cy.clone();
            win.kind = PlotKind::Pseudocolor;
            win.drawing.cancel();
        }
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.refit = true;
        }
        self.status = format!("Showing {cx} against {cy}.");
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
        ui.horizontal(|ui| {
            let n = self.session.samples().len();
            let (prev, next) = (self.step_sample(w.sample, -1), self.step_sample(w.sample, 1));
            if ui.add_enabled(prev.is_some(), egui::Button::new("◀")).on_hover_text("Previous sample (←)").clicked() {
                if let Some(p) = prev {
                    self.hop_sample(id, p);
                }
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
            if ui.add_enabled(next.is_some(), egui::Button::new("▶")).on_hover_text("Next sample (→)").clicked() {
                if let Some(p) = next {
                    self.hop_sample(id, p);
                }
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
                let r = ui.add_enabled(!disabled, egui::SelectableLabel::new(w.tool == tool, RichText::new(icon).size(15.0))).on_hover_text(tool.label());
                if r.clicked() {
                    if let Some(win) = self.session.window_mut(id) {
                        win.tool = tool;
                        win.drawing.cancel();
                    }
                }
            }
        });
        ui.horizontal(|ui| {
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
            self.draw_target_combo(ui);
            ui.menu_button("⇄ Axes", |ui| {
                let mut act: Option<(bool, i32)> = None;
                let hist = w.kind == PlotKind::Histogram;
                if ui.add_enabled(!hist, egui::Button::new("Transpose axes")).on_hover_text("Swap X and Y (Alt+T)").clicked() {
                    act = Some((true, 0));
                }
                ui.separator();
                for (label, x, d, tip) in [("Next X", true, 1, "Alt+→"), ("Previous X", true, -1, "Alt+←"), ("Next Y", false, 1, "Alt+↓"), ("Previous Y", false, -1, "Alt+↑")] {
                    if ui.add_enabled(x || !hist, egui::Button::new(label).shortcut_text(tip)).clicked() {
                        act = Some((x, d));
                    }
                }
                if let Some((x, d)) = act {
                    self.axis_action(id, x, d);
                    ui.close_menu();
                }
            });
            if self.session.sample(w.sample).unmixed && ui.button("◈ Unmixed").on_hover_text("Plot this sample's unmixed markers (Unmix-…): the first two, then press again for the next pair").clicked() {
                self.unmixed_axes(id);
            }
            let fmo_set = self.session.fmo(&w.x_chan).is_some() || (w.kind != PlotKind::Histogram && self.session.fmo(&w.y_chan).is_some());
            if ui.button(if fmo_set { "⌖ Clear FMO" } else { "⌖ Set FMO" }).on_hover_text("Use this population as the FMO control: its 95th percentile becomes a dashed reference line on every plot of the channel").clicked() {
                self.toggle_fmo(id, w);
            }
            let strip_open = self.ws.graphs.get(&id).map_or(false, |g| g.strip_open);
            if theme::toggle(ui, strip_open, "⇔ Compare").on_hover_text("This population on the same axes and view across every sample").clicked() {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    g.strip_open = !g.strip_open;
                }
            }
            ui.menu_button("⤓ Export", |ui| self.export_menu(ui, id, w));
            if theme::toggle(ui, self.ws.options_open, "⚙ Options").clicked() {
                self.ws.options_open = !self.ws.options_open;
            }
            let det = self.ws.detached.contains(&id);
            if theme::toggle(ui, det, "⧉ Detach").on_hover_text("Open this graph in its own window; click again to bring it back").clicked() {
                if det {
                    self.ws.detached.remove(&id);
                } else {
                    self.ws.detached.insert(id);
                }
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
        {
            ui.label(RichText::new("Plot options").strong().size(15.0));
            ui.add_space(4.0);
            egui::CollapsingHeader::new("Population").default_open(true).show(ui, |ui| {
                let s = self.session.sample(w.sample);
                let (xi, yi) = (s.channel_index(&w.x_chan), s.channel_index(&w.y_chan));
                let version = self.session.version(w.sample);
                let key = hash_of((version, w.node, &w.x_chan, &w.y_chan, self.session.fmo(&w.x_chan).map(|f| f.level.to_bits()), self.session.fmo(&w.y_chan).map(|f| f.level.to_bits())));
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
                                        if let Some(frac) = self.session.fraction_above_fmo(w.sample, w.node, chan) {
                                            rows.push((format!("{chan} ≥ FMO"), format!("{:.2} %", 100.0 * frac)));
                                        }
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
            egui::CollapsingHeader::new("Plot style").default_open(true).show(ui, |ui| {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    super::plot_style_ui::plot_style_controls(ui, w.kind, &mut g.plot, "gw_style");
                }
            });
            egui::CollapsingHeader::new("Gates").default_open(true).show(ui, |ui| {
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    ui.horizontal(|ui| {
                        ui.label("Colour");
                        ui.color_edit_button_srgba(&mut g.style.color);
                    });
                    ui.add(egui::Slider::new(&mut g.style.width, 0.8..=4.0).text("Width"));
                    ui.checkbox(&mut g.style.labels, "Show gate names");
                    ui.checkbox(&mut g.style.freqs, "Show gate frequencies");
                    ui.checkbox(&mut g.style.medians, "Show gate medians");
                    ui.checkbox(&mut g.style.fill, "Tint inside");
                }
            });
            egui::CollapsingHeader::new("FMO reference").default_open(false).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Percentile");
                    ui.add(egui::DragValue::new(&mut self.ws.fmo_pct).range(50.0..=100.0).speed(0.1));
                });
                ui.horizontal(|ui| {
                    if ui.button("Set from this population").clicked() {
                        let pct = self.ws.fmo_pct;
                        self.status = match self.session.set_fmo_from_window(id, pct) {
                            Ok(done) => format!("FMO set at the {pct:.0}th percentile: {}.", done.iter().map(|(c, l)| format!("{c} = {l:.0}")).collect::<Vec<_>>().join(", ")),
                            Err(e) => format!("Could not set the FMO: {e}"),
                        };
                        if let Some(g) = self.ws.graphs.get_mut(&id) {
                            g.summary = None;
                        }
                    }
                    if ui.button("Clear all").clicked() {
                        let keys: Vec<String> = self.session.fmo_all().keys().cloned().collect();
                        keys.iter().for_each(|k| self.session.clear_fmo(k));
                        if let Some(g) = self.ws.graphs.get_mut(&id) {
                            g.summary = None;
                        }
                    }
                });
                if let Some(g) = self.ws.graphs.get_mut(&id) {
                    ui.checkbox(&mut g.style.fmo, "Show FMO lines");
                }
                let levels: Vec<(String, String)> = self.session.fmo_all().iter().map(|(c, f)| (c.clone(), format!("{:.0}   (P{:.0} of {} events, {})", f.level, f.percentile, f.n, f.source))).collect();
                if levels.is_empty() {
                    theme::caption(ui, "No FMO set. Open the FMO control, then press Set FMO.");
                }
                for (c, t) in levels {
                    ui.label(RichText::new(format!("{c}: {t}")).size(11.5));
                }
                theme::caption(ui, "A visual guide for placing positive gates. Drag a dashed line to adjust it. It does not gate anything by itself.");
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
        }
    }

    fn graph_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: u64) {
        let Some(w) = self.session.window(id).cloned() else { return };
        let (channels, ranges) = {
            let s = self.session.sample(w.sample);
            (s.channels.clone(), s.ranges.clone())
        };
        let hist = w.kind == PlotKind::Histogram;
        let labels: Vec<String> = channels.iter().map(|c| self.session.channel_label(c)).collect();
        if !hist {
            if let Some(g) = self.ws.graphs.get_mut(&id) {
                g.last_2d = Some(w.kind);
            }
        }
        // ---- Y axis column. Always there, so the plot does not jump sideways between plot types: on a histogram it
        // says "Histogram", and picking a channel goes back to a two-parameter plot.
        let mut changed = false;
        let (mut to_hist, mut to_2d) = (false, false);
        let (mut nx, mut ny, mut xs, mut ys) = (w.x_chan.clone(), w.y_chan.clone(), w.x_scale.clone(), w.y_scale.clone());
        egui::SidePanel::left("g_y").exact_width(124.0).resizable(false).frame(egui::Frame::none()).show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("Y axis").size(11.0).strong().color(theme::MUTED));
            if hist {
                let mut pick = String::new();
                egui::ComboBox::from_id_salt(("y", id, "chan_hist")).width(116.0).selected_text("Histogram").show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                        let _ = ui.selectable_label(true, "Histogram");
                        ui.separator();
                        channel_list(ui, &mut pick, &channels, &labels);
                    });
                });
                if !pick.is_empty() {
                    let r = channels.iter().position(|c| *c == pick).map_or(262_144.0, |i| ranges[i]);
                    ys = default_scale(&pick, r);
                    ny = pick;
                    to_2d = true;
                    changed = true;
                }
            } else {
                changed |= axis_controls(ui, ("y", id), &channels, &labels, &ranges, &mut ny, &mut ys, 116.0, Some(&mut to_hist));
            }
        });
        // ---- X axis row
        egui::TopBottomPanel::bottom("g_x").frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(4.0, 6.0))).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("X axis").size(11.0).strong().color(theme::MUTED));
                let x_shown = channels.iter().position(|c| *c == nx).map_or_else(|| nx.clone(), |i| labels[i].clone());
                egui::ComboBox::from_id_salt(("x", id, "chan")).width(190.0).truncate().selected_text(x_shown).show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| channel_list(ui, &mut nx, &channels, &labels));
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
        if to_hist || to_2d {
            let back = self.ws.graphs.get(&id).and_then(|g| g.last_2d).unwrap_or(PlotKind::Pseudocolor);
            if let Some(win) = self.session.window_mut(id) {
                win.kind = if to_hist { PlotKind::Histogram } else { back };
                if to_hist && !matches!(win.tool, Tool::Interval | Tool::Pointer) {
                    win.tool = Tool::Interval; // a histogram has no second axis
                }
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
        // The plot's size follows the window height only (square), never the width left over by side panels, so
        // opening or closing the options, populations or any other pane never stretches or shrinks the graph.
        let full = Rect::from_min_max(Pos2::new(avail.left() + 52.0, avail.top() + 8.0), Pos2::new(avail.right() - 12.0, avail.bottom() - 34.0));
        let side = full.height().min(full.width());
        let plot = Rect::from_min_size(full.min, egui::vec2(side, side));
        if plot.width() < 80.0 || plot.height() < 80.0 {
            return;
        }
        let resp = ui.allocate_rect(avail, Sense::click_and_drag());
        let painter = ui.painter_at(avail);
        let mut map = Mapper { rect: plot, range };
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.plot_rect = Some(plot);
            g.canvas_rect = Some(avail);
        }
        painter.rect_filled(plot, 0.0, Color32::WHITE);
        self.paint_population(ctx, id, &w, &map, &painter);
        let curve = self.ws.graphs.get(&id).and_then(|g| g.curve.clone());
        let st = self.ws.graphs.get(&id).map(|g| g.plot.clone()).unwrap_or_default();
        draw_axes(&painter, &map, &w, hist, curve.as_ref(), &st, &self.session.channel_label(&w.x_chan), &self.session.channel_label(&w.y_chan));
        let style = self.ws.graphs[&id].style.clone();
        self.gate_edit_input(ctx, id, &w, &resp, &map, hist);
        self.draw_child_gates(&painter, &map, &w, &style, id);
        if style.fmo {
            self.draw_fmo(&painter, &map, &w, true);
        }
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
        if w.tool == Tool::Pointer && resp.dragged() && self.ws.graphs[&id].edit.is_none() && self.ws.graphs[&id].fmo_drag.is_none() && self.ws.graphs[&id].quad_drag.is_none() {
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

        // ---- right-click: the gate under the pointer, or the plot itself
        if resp.secondary_clicked() {
            let at = resp.interact_pointer_pos().filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
            // a gate's name sits outside its outline: a click on the name means that gate too
            let on_label = resp.interact_pointer_pos().and_then(|p| self.gate_label_at(id, p));
            let hit = on_label.or_else(|| at.and_then(|a| self.gate_under(&w, a)));
            if let Some(k) = hit {
                self.session.selected = Some((w.sample, k));
            }
            if let Some(g) = self.ws.graphs.get_mut(&id) {
                g.menu = Some((at.unwrap_or((0.0, 0.0)), hit));
            }
        }
        resp.context_menu(|ui| self.plot_menu(ui, id));
        // A gate's name behaves like its body: click selects, double-click OPENS the population. It must not rename:
        // the name sits a few pixels from the outline, so whether a double-click opened or renamed would depend on
        // where exactly it landed. Renaming is on the right-click menu.
        if w.tool == Tool::Pointer && (resp.clicked() || resp.double_clicked()) {
            if let Some(k) = resp.interact_pointer_pos().and_then(|p| self.gate_label_at(id, p)) {
                self.session.selected = Some((w.sample, k));
                if resp.double_clicked() {
                    if let Err(e) = self.open_tab(w.sample, k) {
                        self.status = e;
                    }
                }
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
            // a click on a gate's name was handled above (it means that gate, though it lies outside the outline)
            if let Some(pos) = resp.interact_pointer_pos().filter(|p| self.gate_label_at(id, *p).is_none()) {
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
                Ok(ids) => {
                    if let Some(win) = self.session.window_mut(id) {
                        win.tool = Tool::Pointer;
                    }
                    format!("Added {} quadrant populations. Double-click one to open it; Ctrl+Z to undo.", ids.len())
                }
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
                    // the natural next move is to double-click the new gate, so go back to the pointer
                    if let Some(win) = self.session.window_mut(id) {
                        win.tool = Tool::Pointer;
                    }
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

    /// The graph window's Export menu: this plot as PDF / SVG (vector), or as the pixels shown (PNG / clipboard).
    fn export_menu(&mut self, ui: &mut egui::Ui, id: u64, w: &GraphWindow) {
        let stem = format!("{}_{}_{}", w.x_chan, w.y_chan, self.session.sample(w.sample).name.trim_end_matches(".fcs")).replace(['/', '\\', ':', ' '], "_");
        if ui.button("Plot as PDF…").on_hover_text("Vector graphics with live text; the current view, gates and FMO lines").clicked() {
            ui.close_menu();
            if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(format!("{stem}.pdf")).save_file() {
                let set = super::plot_export::Settings { format: super::plot_export::Format::Pdf, ..self.quick_export_settings(id) };
                self.status = match self.export_bytes(id, &set, &stem).and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string())) {
                    Ok(()) => format!("Saved the plot to {}.", p.display()),
                    Err(e) => format!("Could not export the plot: {e}"),
                };
            }
        }
        if ui.button("Plot as SVG…").on_hover_text("Vector graphics for Illustrator / Inkscape / PowerPoint").clicked() {
            ui.close_menu();
            if let Some(p) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).set_file_name(format!("{stem}.svg")).save_file() {
                let set = super::plot_export::Settings { format: super::plot_export::Format::Svg, ..self.quick_export_settings(id) };
                self.status = match self.export_bytes(id, &set, &stem).and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string())) {
                    Ok(()) => format!("Saved the plot to {}.", p.display()),
                    Err(e) => format!("Could not export the plot: {e}"),
                };
            }
        }
        if ui.button("Plot as PNG…").on_hover_text("Saved at your export size and resolution").clicked() {
            ui.close_menu();
            self.quick_save_plot(id, super::plot_export::Format::Png);
        }
        ui.separator();
        if ui.button("Export editor…").on_hover_text("Choose the size, resolution, format and what appears on the plot, with a preview").clicked() {
            ui.close_menu();
            self.open_plot_export(id);
        }
        let canvas = self.ws.graphs.get(&id).and_then(|g| g.canvas_rect);
        if ui.add_enabled(canvas.is_some(), egui::Button::new("Plot as PNG (as shown)…")).on_hover_text("The pixels on screen, at screen resolution").clicked() {
            ui.close_menu();
            if let (Some(rect), Some(p)) = (canvas, rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{stem}.png")).save_file()) {
                self.begin_capture(rect, super::CaptureDest::File(p));
            }
        }
        if ui.button("Copy image to clipboard").on_hover_text("With your export settings (Export editor, Use as my default)").clicked() {
            ui.close_menu();
            self.quick_copy_plot(id);
        }
    }

    /// The histogram / contour / density options of a graph window (created if the window has not been drawn yet).
    pub fn plot_style_mut(&mut self, id: u64) -> &mut PlotStyle {
        &mut self.ws.graphs.entry(id).or_insert_with(GraphUi::new).plot
    }

    /// Open or close the compare strip of a graph window.
    pub fn set_strip_open(&mut self, id: u64, open: bool) {
        self.ws.graphs.entry(id).or_insert_with(GraphUi::new).strip_open = open;
    }

    /// Make a graph window fit its view to the population on its next frame.
    pub fn refit_window(&mut self, id: u64) {
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.refit = true;
        }
    }

    /// The same as the toolbar's Set FMO / Clear FMO button, for the right-click menu.
    pub(super) fn toggle_fmo_public(&mut self, id: u64) {
        if let Some(w) = self.session.window(id).cloned() {
            self.toggle_fmo(id, &w);
        }
    }

    /// Set the FMO reference from this population, or clear it when one is already set for the shown channel(s).
    fn toggle_fmo(&mut self, id: u64, w: &GraphWindow) {
        let mut shown = vec![w.x_chan.clone()];
        if w.kind != PlotKind::Histogram {
            shown.push(w.y_chan.clone());
        }
        if shown.iter().any(|c| self.session.fmo(c).is_some()) {
            shown.iter().for_each(|c| self.session.clear_fmo(c));
            self.status = format!("FMO reference cleared for {}.", shown.join(", "));
        } else {
            let pct = self.ws.fmo_pct;
            self.status = match self.session.set_fmo_from_window(id, pct) {
                Ok(done) => format!("FMO set at the {pct:.0}th percentile: {}.", done.iter().map(|(c, l)| format!("{c} = {l:.0}")).collect::<Vec<_>>().join(", ")),
                Err(e) => format!("Could not set the FMO: {e}"),
            };
        }
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.summary = None;
        }
    }

    /// Dashed FMO reference lines for the window's channels, on the window's own scales.
    fn draw_fmo(&self, painter: &egui::Painter, map: &Mapper, w: &GraphWindow, handles: bool) {
        let colour = Color32::from_rgb(142, 68, 173);
        let stroke = Stroke::new(1.5_f32, colour);
        let painter = painter.with_clip_rect(map.rect.expand(3.0));
        let square = |c: Pos2| {
            if handles {
                let r = Rect::from_center_size(c, egui::vec2(8.0, 8.0));
                painter.rect_filled(r, 1.0, colour);
            }
        };
        if let (Some(f), Ok(t)) = (self.session.fmo(&w.x_chan), w.x_scale.build()) {
            let x = t.forward(f.level);
            let sx = map.to_screen((x, map.range.y0)).x;
            if x.is_finite() && sx >= map.rect.left() && sx <= map.rect.right() {
                painter.extend(egui::Shape::dashed_line(&[Pos2::new(sx, map.rect.top()), Pos2::new(sx, map.rect.bottom())], stroke, 6.0, 4.0));
                if handles {
                    painter.text(Pos2::new(sx + 3.0, map.rect.top() + 12.0), Align2::LEFT_TOP, "FMO", FontId::proportional(10.5), colour);
                }
                square(Pos2::new(sx, map.rect.top()));
                square(Pos2::new(sx, map.rect.bottom()));
            }
        }
        if w.kind != PlotKind::Histogram {
            if let (Some(f), Ok(t)) = (self.session.fmo(&w.y_chan), w.y_scale.build()) {
                let y = t.forward(f.level);
                let sy = map.to_screen((map.range.x0, y)).y;
                if y.is_finite() && sy >= map.rect.top() && sy <= map.rect.bottom() {
                    painter.extend(egui::Shape::dashed_line(&[Pos2::new(map.rect.left(), sy), Pos2::new(map.rect.right(), sy)], stroke, 6.0, 4.0));
                    if handles {
                        painter.text(Pos2::new(map.rect.left() + 4.0, sy - 2.0), Align2::LEFT_BOTTOM, "FMO", FontId::proportional(10.5), colour);
                    }
                    square(Pos2::new(map.rect.left(), sy));
                    square(Pos2::new(map.rect.right(), sy));
                }
            }
        }
    }

    /// Which FMO line (`true` = the X channel's) is within a few pixels of a screen point.
    fn fmo_hit(&self, w: &GraphWindow, map: &Mapper, p: Pos2) -> Option<bool> {
        if let (Some(f), Ok(t)) = (self.session.fmo(&w.x_chan), w.x_scale.build()) {
            let sx = map.to_screen((t.forward(f.level), map.range.y0)).x;
            if (p.x - sx).abs() <= 6.0 {
                return Some(true);
            }
        }
        if w.kind != PlotKind::Histogram {
            if let (Some(f), Ok(t)) = (self.session.fmo(&w.y_chan), w.y_scale.build()) {
                let sy = map.to_screen((map.range.x0, t.forward(f.level))).y;
                if (p.y - sy).abs() <= 6.0 {
                    return Some(false);
                }
            }
        }
        None
    }

    /// The compare strip: this window's population on the same axes and view range in every sample, each with its
    /// own gates. Clicking a card shows that sample in the plot above.
    fn compare_strip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: u64, w: &GraphWindow) {
        let Some(range) = self.ws.graphs.get(&id).and_then(|g| g.range).filter(|r| r.is_valid()) else {
            ui.horizontal(|ui| {
                if ui.button("✕ Close").clicked() {
                    self.set_strip_open(id, false);
                }
                theme::caption(ui, "Nothing to compare yet: this plot has no view range.");
            });
            return;
        };
        let path = self.session.tree(w.sample).path(w.node).unwrap_or_default();
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        let n = self.session.samples().len();
        let names = short_all(&self.session.samples().iter().map(|s| s.name.clone()).collect::<Vec<_>>());
        let pop = if path.is_empty() { "All Events".to_string() } else { path.join(" ▸ ") };
        let mut fit_all = false;
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button("✕ Close").on_hover_text("Hide the compare strip").clicked() {
                close = true;
            }
            ui.label(RichText::new("Compare").strong());
            if ui.small_button("⤢ Fit all").on_hover_text("Widen the shared view so every sample's population fits").clicked() {
                fit_all = true;
            }
            theme::caption(ui, format!("{pop} across {n} samples · same axes, scales and view as the plot above · click a sample to show it"));
        });
        if close {
            self.set_strip_open(id, false);
            return;
        }
        if fit_all {
            self.fit_all_samples(id, w);
        }
        let mut jump: Option<usize> = None;
        let mut rects: Vec<(usize, Rect)> = Vec::new();
        let hist = w.kind == PlotKind::Histogram;
        egui::ScrollArea::horizontal().id_salt(("strip", id)).auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for si in 0..n {
                    let node = if refs.is_empty() { Some(GateTree::ROOT) } else { self.session.tree(si).find(&refs) };
                    let current = si == w.sample;
                    egui::Frame::none().fill(theme::PANEL_2).rounding(8.0).stroke(Stroke::new(if current { 2.0_f32 } else { 0.0_f32 }, theme::ACCENT)).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(200.0);
                            ui.label(RichText::new(&names[si]).size(11.5).strong());
                            let Some(node) = node else {
                                ui.add_space(64.0);
                                theme::caption(ui, "no such population");
                                ui.add_space(64.0);
                                return;
                            };
                            let count = self.session.count(si, node);
                            let parent = self.session.tree(si).parent(node).ok().flatten().map(|p| self.session.count(si, p)).unwrap_or(self.session.sample(si).n_events).max(1);
                            match self.strip_thumb(ctx, id, si, node, w, range) {
                                Some(thumb) => {
                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(200.0, 150.0), Sense::click());
                                    rects.push((si, rect));
                                    let p = ui.painter_at(rect);
                                    p.rect_filled(rect, 0.0, Color32::WHITE);
                                    match &thumb {
                                        StripThumb::Tex(t) => {
                                            p.image(t.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                                        }
                                        StripThumb::Hist(h) => {
                                            let bw = rect.width() / h.len().max(1) as f32;
                                            for (i, v) in h.iter().enumerate() {
                                                let x = rect.left() + i as f32 * bw;
                                                p.rect_filled(Rect::from_min_max(Pos2::new(x, rect.bottom() - v * 0.94 * rect.height()), Pos2::new(x + bw + 0.5, rect.bottom())), 0.0, Color32::from_rgba_unmultiplied(0, 140, 200, 160));
                                            }
                                        }
                                    }
                                    let map = Mapper { rect, range };
                                    let kids: Vec<Gate> = self.session.tree(si).children(node).map(|c| c.to_vec()).unwrap_or_default().into_iter().filter_map(|k| match self.session.tree(si).kind(k) {
                                        Ok(NodeKind::Gate(g)) if gate_on_axes(g, w) => Some(g.clone()),
                                        _ => None,
                                    }).collect();
                                    for g in &kids {
                                        draw_gate(&p, &map, g, Stroke::new(1.3_f32, theme::GATE_RED), hist, false);
                                    }
                                    self.draw_fmo(&p, &map, w, false);
                                    if resp.on_hover_text("Click to show this sample").clicked() {
                                        jump = Some(si);
                                    }
                                }
                                None => {
                                    ui.add_space(64.0);
                                    theme::caption(ui, "channel not in this sample");
                                    ui.add_space(64.0);
                                }
                            }
                            theme::caption(ui, format!("{count} events · {:.1}% of parent", 100.0 * count as f64 / parent as f64));
                        });
                    });
                }
            });
        });
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.strip_rects = rects;
        }
        if let Some(si) = jump {
            if si != w.sample {
                match self.session.retarget_window(id, si) {
                    Ok(()) => {
                        if let Some(g) = self.ws.graphs.get_mut(&id) {
                            g.key = 0;
                        }
                    }
                    Err(e) => self.status = e,
                }
            }
        }
    }

    /// Set the window's view to the union of every sample's population on these axes.
    pub fn fit_all_samples(&mut self, id: u64, w: &GraphWindow) {
        let path = self.session.tree(w.sample).path(w.node).unwrap_or_default();
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for si in 0..self.session.samples().len() {
            let node = if refs.is_empty() { Some(GateTree::ROOT) } else { self.session.tree(si).find(&refs) };
            let Some(node) = node else { continue };
            let Ok(pts) = self.session.population_points(si, node, &w.x_chan, &w.y_chan, &w.x_scale, &w.y_scale) else { continue };
            for (a, b) in pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()) {
                x0 = x0.min(*a);
                x1 = x1.max(*a);
                y0 = y0.min(*b);
                y1 = y1.max(*b);
            }
        }
        if !(x1 > x0 && y1 > y0) {
            self.status = "No sample has events on these axes.".into();
            return;
        }
        let (px, py) = (0.01 * (x1 - x0), 0.01 * (y1 - y0));
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.range = Some(Range { x0: x0 - px, x1: x1 + px, y0: y0 - py, y1: y1 + py });
            g.refit = false;
            g.key = 0;
        }
    }

    /// A cached picture of one sample's population on the window's axes and view range.
    fn strip_thumb(&mut self, ctx: &egui::Context, id: u64, si: usize, node: NodeId, w: &GraphWindow, range: Range) -> Option<StripThumb> {
        let key = hash_of((self.session.version(si), node, &w.x_chan, &w.y_chan, w.x_scale.to_text(), w.y_scale.to_text(), range.x0.to_bits(), range.x1.to_bits(), range.y0.to_bits(), range.y1.to_bits(), w.kind as u8));
        if let Some((k, t)) = self.ws.graphs.get(&id).and_then(|g| g.strip.get(&si)) {
            if *k == key {
                return Some(t.clone());
            }
        }
        let pts = self.session.population_points(si, node, &w.x_chan, &w.y_chan, &w.x_scale, &w.y_scale).ok()?;
        let thumb = if w.kind == PlotKind::Histogram {
            let h = histogram(pts.iter().map(|p| p.0), range.x0, range.x1, 64);
            let mx = h.iter().copied().max().unwrap_or(1).max(1) as f32;
            StripThumb::Hist(h.into_iter().map(|c| c as f32 / mx).collect())
        } else {
            let grid = bin(pts.iter().copied(), range, 100, 75);
            let style = self.ws.graphs.get(&id).map(|g| g.plot.clone()).unwrap_or_default();
            let px = render_kind(&grid, w.kind, &style);
            let pixels: Vec<Color32> = px.into_iter().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
            StripThumb::Tex(ctx.load_texture(format!("strip{id}_{si}"), egui::ColorImage { size: [100, 75], pixels }, TextureOptions::NEAREST))
        };
        self.ws.graphs.get_mut(&id)?.strip.insert(si, (key, thumb.clone()));
        Some(thumb)
    }

    /// Move / resize the gates of this window with the Pointer tool: press on a handle (or inside a gate) and drag.
    /// The result is previewed live and committed as one undo step when the button is released; Esc cancels.
    fn gate_edit_input(&mut self, ctx: &egui::Context, id: u64, w: &GraphWindow, resp: &egui::Response, map: &Mapper, hist: bool) {
        let _ = hist;
        if w.tool != Tool::Pointer {
            if let Some(g) = self.ws.graphs.get_mut(&id) {
                g.edit = None; // choosing another tool abandons a half-done edit
            }
            return;
        }
        let (plot, range) = (map.rect, map.range);
        let tol = (7.0 / plot.width() as f64 * (range.x1 - range.x0), 7.0 / plot.height() as f64 * (range.y1 - range.y0));
        let y_mid = (range.y0 + range.y1) / 2.0;
        // an FMO line is grabbed before any gate
        let show_fmo = self.ws.graphs.get(&id).map_or(true, |g| g.style.fmo);
        if resp.drag_started() && show_fmo {
            if let Some(o) = ctx.input(|i| i.pointer.press_origin()).filter(|p| plot.contains(*p)) {
                if let Some(is_x) = self.fmo_hit(w, map, o) {
                    self.ws.graphs.get_mut(&id).unwrap().fmo_drag = Some(is_x);
                }
            }
        }
        if let Some(is_x) = self.ws.graphs.get(&id).and_then(|g| g.fmo_drag) {
            let (chan, scale) = if is_x { (w.x_chan.clone(), w.x_scale.clone()) } else { (w.y_chan.clone(), w.y_scale.clone()) };
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let d = map.to_data(Pos2::new(p.x.clamp(plot.left(), plot.right()), p.y.clamp(plot.top(), plot.bottom())));
                    if let Ok(t) = scale.build() {
                        self.session.move_fmo(&chan, t.inverse(if is_x { d.0 } else { d.1 }));
                    }
                }
                ctx.request_repaint();
            }
            if resp.drag_stopped() {
                self.ws.graphs.get_mut(&id).unwrap().fmo_drag = None;
                if let Some(f) = self.session.fmo(&chan) {
                    self.status = format!("FMO level for {chan} moved to {:.0}.", f.level);
                }
                self.ws.graphs.get_mut(&id).unwrap().summary = None;
            }
            return;
        }
        // a quadrant's crosshair: grab the centre to move both lines, or one line to move just that one
        if resp.drag_started() {
            let origin = ctx.input(|i| i.pointer.press_origin()).filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
            if let Some((node, orig, moves)) = origin.and_then(|o| self.quadrant_hit(w, o, tol)) {
                self.session.selected = Some((w.sample, node));
                self.ws.graphs.get_mut(&id).unwrap().quad_drag = Some(QuadDrag { node, orig, now: orig, moves });
            }
        }
        if let Some(q) = self.ws.graphs.get(&id).and_then(|g| g.quad_drag) {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.ws.graphs.get_mut(&id).unwrap().quad_drag = None;
                self.status = "Edit cancelled.".into();
                return;
            }
            if resp.dragged() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let cur = map.to_data(Pos2::new(p.x.clamp(plot.left(), plot.right()), p.y.clamp(plot.top(), plot.bottom())));
                    let now = (if q.moves.0 { cur.0 } else { q.orig.0 }, if q.moves.1 { cur.1 } else { q.orig.1 });
                    self.ws.graphs.get_mut(&id).unwrap().quad_drag = Some(QuadDrag { now, ..q });
                }
                ctx.request_repaint();
            }
            if resp.drag_stopped() {
                self.ws.graphs.get_mut(&id).unwrap().quad_drag = None;
                if q.now != q.orig {
                    self.status = match self.session.move_quadrants(w.sample, q.node, q.now.0, q.now.1) {
                        Ok(n) => format!("Moved the quadrant gate ({n} populations). Ctrl+Z to undo."),
                        Err(e) => format!("Could not move the quadrant: {e}"),
                    };
                    if let Some(g) = self.ws.graphs.get_mut(&id) {
                        g.key = 0;
                        g.summary = None;
                    }
                }
            }
            return;
        }
        if resp.drag_started() {
            let origin = ctx.input(|i| i.pointer.press_origin()).filter(|p| plot.contains(*p)).map(|p| map.to_data(p));
            if let Some(o) = origin {
                self.begin_gate_edit(id, w, o, tol, y_mid);
            }
        }
        let editing = self.ws.graphs.get(&id).map_or(false, |g| g.edit.is_some());
        if !editing {
            // show what a press here would grab
            if let Some(pos) = resp.hover_pos().filter(|p| plot.contains(*p)) {
                if show_fmo {
                    if let Some(is_x) = self.fmo_hit(w, map, pos) {
                        ctx.set_cursor_icon(if is_x { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical });
                        return;
                    }
                }
                let at = map.to_data(pos);
                if let Some((_, _, moves)) = self.quadrant_hit(w, at, tol) {
                    ctx.set_cursor_icon(match moves {
                        (true, true) => egui::CursorIcon::Move,
                        (true, false) => egui::CursorIcon::ResizeHorizontal,
                        _ => egui::CursorIcon::ResizeVertical,
                    });
                    return;
                }
                let selected = self.session.selected.filter(|(s, _)| *s == w.sample).map(|(_, n)| n);
                let icon = selected.and_then(|n| match self.session.tree(w.sample).kind(n) {
                    Ok(NodeKind::Gate(g)) if gate_on_axes(g, w) && self.session.tree(w.sample).parent(n).ok().flatten() == Some(w.node) => gate_edit::hit(g, at, tol, y_mid),
                    _ => None,
                });
                match icon {
                    Some(Handle::Body) => ctx.set_cursor_icon(egui::CursorIcon::Grab),
                    Some(Handle::Edge(Side::Left | Side::Right)) => ctx.set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
                    Some(Handle::Edge(_)) => ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical),
                    Some(Handle::Corner { right, top }) => ctx.set_cursor_icon(if right == top { egui::CursorIcon::ResizeNeSw } else { egui::CursorIcon::ResizeNwSe }),
                    Some(_) => ctx.set_cursor_icon(egui::CursorIcon::Crosshair),
                    None => {}
                }
            }
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ws.graphs.get_mut(&id).unwrap().edit = None;
            self.status = "Edit cancelled.".into();
            return;
        }
        if resp.dragged() {
            if let Some(p) = resp.interact_pointer_pos() {
                let cur = map.to_data(Pos2::new(p.x.clamp(plot.left(), plot.right()), p.y.clamp(plot.top(), plot.bottom())));
                if let Some(e) = self.ws.graphs.get_mut(&id).and_then(|g| g.edit.as_mut()) {
                    e.preview = gate_edit::apply(&e.orig, e.handle, e.start, cur);
                }
            }
            ctx.request_repaint();
        }
        if resp.drag_stopped() {
            let Some(e) = self.ws.graphs.get_mut(&id).and_then(|g| g.edit.take()) else { return };
            match e.preview {
                Some(new) if new != e.orig => {
                    let name = self.session.tree(w.sample).name(e.node).unwrap_or("population").to_string();
                    let before = self.session.count(w.sample, e.node);
                    self.status = match self.session.edit_gate(w.sample, e.node, new) {
                        Ok(after) => format!("Changed '{name}': {before} → {after} events. Ctrl+Z to undo."),
                        Err(err) => format!("Could not change the gate: {err}"),
                    };
                    if let Some(g) = self.ws.graphs.get_mut(&id) {
                        g.key = 0;
                    }
                }
                Some(_) => {}
                None => self.status = "That would leave the gate with no size, so it was not changed.".into(),
            }
        }
    }

    /// A quadrant crosshair under `at`: the population to move, its centre, and which coordinates a drag from here
    /// changes (the centre moves both lines, a point on one line moves that line only).
    pub fn quadrant_hit(&self, w: &GraphWindow, at: (f64, f64), tol: (f64, f64)) -> Option<(NodeId, (f64, f64), (bool, bool))> {
        if w.kind == PlotKind::Histogram {
            return None;
        }
        let kids = self.session.tree(w.sample).children(w.node).ok()?;
        kids.iter().rev().find_map(|&k| match self.session.tree(w.sample).kind(k) {
            Ok(NodeKind::Gate(g)) if matches!(g.kind, GateKind::Quadrant(_)) && g.xs.len() == 1 && gate_on_axes(g, w) => {
                let (nx, ny) = (((at.0 - g.xs[0]) / tol.0).abs() <= 1.0, ((at.1 - g.ys[0]) / tol.1).abs() <= 1.0);
                (nx || ny).then_some((k, (g.xs[0], g.ys[0]), (nx, ny)))
            }
            _ => None,
        })
    }

    /// Pick what a press at `at` grabs: handles of the selected gate, then handles of the others, then bodies.
    fn begin_gate_edit(&mut self, id: u64, w: &GraphWindow, at: (f64, f64), tol: (f64, f64), y_mid: f64) {
        let mut order: Vec<NodeId> = self.session.tree(w.sample).children(w.node).map(|c| c.iter().rev().copied().collect()).unwrap_or_default();
        if let Some((s, n)) = self.session.selected {
            if s == w.sample && order.contains(&n) {
                order.retain(|k| *k != n);
                order.insert(0, n);
            }
        }
        let gates: Vec<(NodeId, Gate)> = order
            .into_iter()
            .filter_map(|k| match self.session.tree(w.sample).kind(k) {
                Ok(NodeKind::Gate(g)) if gate_on_axes(g, w) => Some((k, g.clone())),
                _ => None,
            })
            .collect();
        let found = gates.iter().find_map(|(k, g)| gate_edit::hit_handle(g, at, tol, y_mid).map(|h| (*k, g.clone(), h))).or_else(|| gates.iter().find_map(|(k, g)| gate_edit::hit(g, at, tol, y_mid).map(|h| (*k, g.clone(), h))));
        if let Some((k, g, h)) = found {
            self.session.selected = Some((w.sample, k));
            if let Some(gr) = self.ws.graphs.get_mut(&id) {
                gr.edit = Some(EditDrag { node: k, handle: h, start: at, orig: g, preview: None });
            }
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
        let style_key = self.ws.graphs.get(&id).map(|g| format!("{:?}", g.plot)).unwrap_or_default();
        let key = hash_of((pkey, range.x0.to_bits(), range.x1.to_bits(), range.y0.to_bits(), range.y1.to_bits(), gw, gh, w.kind as u8, style_key));
        let stale = self.ws.graphs.get(&id).map_or(true, |g| g.key != key);
        if stale {
            let Some(g) = self.ws.graphs.get_mut(&id) else { return };
            let pts = &g.pts.as_ref().unwrap().1;
            if w.kind == PlotKind::Histogram {
                let curve = histogram_curve(pts.iter().map(|p| p.0), range.x0, range.x1, gw, &g.plot);
                g.shown = curve.total as usize;
                g.curve = Some(curve);
                g.tex = None;
            } else {
                let grid = bin(pts.iter().copied(), range, gw, gh);
                let px = render_kind(&grid, w.kind, &g.plot);
                let pixels: Vec<Color32> = px.into_iter().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
                let tex = ctx.load_texture(format!("graph{id}"), egui::ColorImage { size: [gw, gh], pixels }, TextureOptions::NEAREST);
                g.tex = Some(tex);
                g.shown = grid.inside;
                g.curve = None;
            }
            g.key = key;
        }
        let Some(g) = self.ws.graphs.get(&id) else { return };
        if w.kind == PlotKind::Histogram {
            if let Some(c) = &g.curve {
                draw_histogram(painter, map, c, &g.plot);
            }
        } else if let Some(tex) = &g.tex {
            painter.image(tex.id(), map.rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        }
    }

    fn draw_child_gates(&mut self, painter: &egui::Painter, map: &Mapper, w: &GraphWindow, style: &GateStyle, id: u64) {
        let kids: Vec<NodeId> = self.session.tree(w.sample).children(w.node).map(|c| c.to_vec()).unwrap_or_default();
        let parent_count = self.session.count(w.sample, w.node).max(1);
        let painter = painter.with_clip_rect(map.rect.expand(2.0));
        let mut label_rects: Vec<(NodeId, Rect)> = Vec::new();
        for k in kids {
            let (name, mut gate) = match (self.session.tree(w.sample).name(k), self.session.tree(w.sample).kind(k)) {
                (Ok(n), Ok(NodeKind::Gate(g))) => (n.to_string(), g.clone()),
                _ => continue,
            };
            // a gate is only drawn on the axes (and scales) it was made on
            if !gate_on_axes(&gate, w) {
                continue;
            }
            // while it is being dragged, show where it would land
            if let Some(p) = self.ws.graphs.get(&id).and_then(|g| g.edit.as_ref()).filter(|e| e.node == k).and_then(|e| e.preview.clone()) {
                gate = p;
            }
            // a quadrant being dragged: all of its sectors are drawn where the crosshair would land
            if let Some(q) = self.ws.graphs.get(&id).and_then(|g| g.quad_drag) {
                if matches!(gate.kind, GateKind::Quadrant(_)) && gate.xs.len() == 1 && (gate.xs[0], gate.ys[0]) == q.orig {
                    gate.xs[0] = q.now.0;
                    gate.ys[0] = q.now.1;
                }
            }
            let selected = self.session.selected == Some((w.sample, k));
            let owner = self.session.gate_owner(w.sample, k);
            let own_tint = owner.as_ref().and_then(|o| o.color).map(|c| Color32::from_rgb(c[0], c[1], c[2]));
            let stroke = Stroke::new(if selected { style.width + 1.2 } else { style.width }, if selected { Color32::from_rgb(255, 214, 10) } else { self.ws.gate_colors.get(&(w.sample, k)).map_or(own_tint.unwrap_or(style.color), |c| Color32::from_rgb(c[0], c[1], c[2])) });
            let anchor = draw_gate(&painter, map, &gate, stroke, w.kind == PlotKind::Histogram, style.fill);
            if selected && w.tool == Tool::Pointer {
                let y_mid = (map.range.y0 + map.range.y1) / 2.0;
                for (_, at) in gate_edit::handles(&gate, y_mid) {
                    let c = map.to_screen(at);
                    let r = Rect::from_center_size(c, egui::vec2(8.0, 8.0));
                    painter.rect_filled(r, 1.0, Color32::WHITE);
                    painter.rect_stroke(r, 1.0, Stroke::new(1.2_f32, Color32::from_rgb(40, 40, 40)));
                }
            }
            if let (Some(o), Some(a)) = (&owner, anchor) {
                if o.edited {
                    // the group's gate, changed in this sample since: say so on the plot
                    let at = a + egui::vec2(3.0, -3.0 - 13.0 * (style.labels as u8 + style.freqs as u8 + 2 * style.medians as u8) as f32);
                    let text = format!("✎ edited · {}", o.group);
                    let r = painter.text(at, Align2::LEFT_BOTTOM, &text, FontId::proportional(10.5), Color32::TRANSPARENT);
                    label_rects.push((k, r.expand(2.0)));
                    painter.rect_filled(r.expand(2.0), 3.0, Color32::from_rgba_unmultiplied(255, 244, 214, 235));
                    painter.text(at, Align2::LEFT_BOTTOM, text, FontId::proportional(10.5), Color32::from_rgb(160, 80, 0));
                }
            }
            if style.labels || style.freqs || style.medians {
                if let Some(a) = anchor {
                    let count = self.session.count(w.sample, k);
                    let mut text = String::new();
                    if style.labels {
                        text.push_str(&name);
                    }
                    if style.freqs {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(&format!("{:.1}%  ({count})", 100.0 * count as f64 / parent_count as f64));
                    }
                    if style.medians {
                        let mut chans = vec![w.x_chan.clone()];
                        if w.kind != PlotKind::Histogram {
                            chans.push(w.y_chan.clone());
                        }
                        for ch in chans {
                            let m = crate::stats_table::node_stat(&mut self.session, w.sample, k, crate::stats_table::Metric::Median, &ch);
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&format!("Median {ch}: {}", m.map_or("–".to_string(), |v| format!("{v:.0}"))));
                        }
                    }
                    for line in self.gate_stat_lines(w.sample, k) {
                        text.push('\n');
                        text.push_str(&line);
                    }
                    let r = painter.text(a + egui::vec2(3.0, -3.0), Align2::LEFT_BOTTOM, text, FontId::proportional(11.0), Color32::from_rgb(120, 15, 15));
                    label_rects.push((k, r.expand(2.0)));
                }
            }
        }
        if let Some(g) = self.ws.graphs.get_mut(&id) {
            g.label_rects = label_rects;
        }
    }
}

/// Is this gate drawn on exactly the window's current axes and scales?
pub(super) fn gate_on_axes(g: &Gate, w: &GraphWindow) -> bool {
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

/// The histogram curve: filled strips (or bars) under the path, then the outline, in the style's colours.
fn draw_histogram(painter: &egui::Painter, map: &Mapper, c: &HistCurve, style: &PlotStyle) {
    let (r, n) = (map.rect, c.heights.len().max(1));
    let to = |xf: f64, h: f64| Pos2::new(r.left() + xf as f32 * r.width(), r.bottom() - h as f32 * r.height());
    if style.hist_style != HistStyle::Line {
        let fill = Color32::from_rgba_unmultiplied(style.hist_fill[0], style.hist_fill[1], style.hist_fill[2], style.hist_fill_alpha);
        for (i, h) in c.heights.iter().enumerate() {
            let (x0, x1) = (r.left() + i as f32 / n as f32 * r.width(), r.left() + (i + 1) as f32 / n as f32 * r.width());
            painter.rect_filled(Rect::from_min_max(Pos2::new(x0, r.bottom() - *h as f32 * r.height()), Pos2::new(x1 + 0.5, r.bottom())), 0.0, fill);
        }
    }
    let path: Vec<Pos2> = hist_path(c, style.hist_style).into_iter().map(|(xf, h)| to(xf, h)).collect();
    if path.len() > 1 {
        painter.add(egui::Shape::line(path, Stroke::new(1.4_f32, Color32::from_rgb(style.hist_line[0], style.hist_line[1], style.hist_line[2]))));
    }
}

/// Which axis labels to draw so none overlap: `items` are (position along the axis, text). Labels are taken in
/// order of closeness to the zero tick (or the first label when there is none), and one that would touch an
/// already kept label (within `gap` points) is dropped, so changing decades or zooming never prints numbers on top
/// of each other.
pub(super) fn declutter(p: &egui::Painter, items: &[(f32, String)], font: &FontId, horizontal: bool, zero: Option<usize>, gap: f32) -> Vec<bool> {
    let extent: Vec<f32> = items
        .iter()
        .map(|(_, t)| {
            if t.is_empty() {
                0.0
            } else {
                let g = p.layout_no_wrap(t.clone(), font.clone(), Color32::BLACK);
                if horizontal { g.size().x } else { g.size().y }
            }
        })
        .collect();
    let origin = zero.map_or_else(|| items.iter().find(|i| !i.1.is_empty()).map_or(0.0, |i| i.0), |z| items[z].0);
    let mut order: Vec<usize> = (0..items.len()).filter(|&i| !items[i].1.is_empty()).collect();
    order.sort_by(|&a, &b| (items[a].0 - origin).abs().partial_cmp(&(items[b].0 - origin).abs()).unwrap_or(std::cmp::Ordering::Equal));
    let mut keep = vec![false; items.len()];
    let mut taken: Vec<(f32, f32)> = Vec::new();
    for i in order {
        let (lo, hi) = (items[i].0 - extent[i] / 2.0 - gap / 2.0, items[i].0 + extent[i] / 2.0 + gap / 2.0);
        if taken.iter().all(|&(a, b)| hi <= a || lo >= b) {
            taken.push((lo, hi));
            keep[i] = true;
        }
    }
    keep
}

#[allow(clippy::too_many_arguments)]
fn draw_axes(p: &egui::Painter, map: &Mapper, w: &GraphWindow, hist: bool, curve: Option<&HistCurve>, st: &PlotStyle, x_title: &str, y_title: &str) {
    let (show_grid, sci) = (st.grid, st.sci);
    let lab = |l: &str| if sci { crate::axis::sci_label(l) } else { l.to_string() };
    let axis = Stroke::new(1.0_f32, Color32::from_gray(150));
    let grid = Stroke::new(if show_grid { 0.5_f32 } else { 0.0 }, if show_grid { Color32::from_gray(228) } else { Color32::TRANSPARENT });
    let text = theme::TEXT;
    let r = map.rect;
    let font = FontId::proportional(10.5);
    let xticks = ticks(&w.x_scale, map.range.x0, map.range.x1);
    let xlabels: Vec<(f32, String)> = xticks.iter().map(|t| (map.to_screen((t.pos, map.range.y0)).x, if t.major { lab(&t.label) } else { String::new() })).collect();
    let xshow = declutter(p, &xlabels, &font, true, xticks.iter().position(|t| t.major && t.label == "0"), 6.0);
    for (ti, t) in xticks.iter().enumerate() {
        let x = map.to_screen((t.pos, map.range.y0)).x;
        if t.major {
            p.line_segment([Pos2::new(x, r.top()), Pos2::new(x, r.bottom())], grid);
            p.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x, r.bottom() + 5.0)], axis);
            if xshow[ti] {
                p.text(Pos2::new(x, r.bottom() + 7.0), Align2::CENTER_TOP, &xlabels[ti].1, font.clone(), text);
            }
        } else {
            p.line_segment([Pos2::new(x, r.bottom()), Pos2::new(x, r.bottom() + 2.5)], axis);
        }
    }
    if !hist {
        let yticks = ticks(&w.y_scale, map.range.y0, map.range.y1);
        let ylabels: Vec<(f32, String)> = yticks.iter().map(|t| (map.to_screen((map.range.x0, t.pos)).y, if t.major { lab(&t.label) } else { String::new() })).collect();
        let yshow = declutter(p, &ylabels, &font, false, yticks.iter().position(|t| t.major && t.label == "0"), 2.0);
        for (ti, t) in yticks.iter().enumerate() {
            let y = map.to_screen((map.range.x0, t.pos)).y;
            if t.major {
                p.line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], grid);
                p.line_segment([Pos2::new(r.left() - 5.0, y), Pos2::new(r.left(), y)], axis);
                if yshow[ti] {
                    p.text(Pos2::new(r.left() - 8.0, y), Align2::RIGHT_CENTER, &ylabels[ti].1, font.clone(), text);
                }
            } else {
                p.line_segment([Pos2::new(r.left() - 2.5, y), Pos2::new(r.left(), y)], axis);
            }
        }
    } else {
        if let Some(c) = curve {
            for (frac, label) in hist_ticks(c) {
                let y = r.bottom() - frac as f32 * r.height();
                p.line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], grid);
                p.line_segment([Pos2::new(r.left() - 5.0, y), Pos2::new(r.left(), y)], axis);
                p.text(Pos2::new(r.left() - 8.0, y), Align2::RIGHT_CENTER, lab(&label), FontId::proportional(10.5), text);
            }
            p.text(Pos2::new(r.left() + 4.0, r.top() + 3.0), Align2::LEFT_TOP, c.y_label, FontId::proportional(11.5), Color32::from_gray(60));
        }
    }
    p.rect_stroke(r, 0.0, axis);
    p.text(Pos2::new(r.center().x, r.bottom() + 20.0), Align2::CENTER_TOP, x_title, FontId::proportional(11.5), text);
    if !hist {
        p.text(Pos2::new(r.left() + 4.0, r.top() + 3.0), Align2::LEFT_TOP, y_title, FontId::proportional(11.5), Color32::from_gray(60));
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
