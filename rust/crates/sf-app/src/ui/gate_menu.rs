//! Right-click menus and their dialogs: the gate menu on a plot, the plot's own menu, the population tree menu.
//! (The previous app's menus: open in a new window, rename, convert to polygon, add node, add statistic, colour,
//! gate history, copy / paste gate, apply to all samples, exclusion gate, delete, sub sample, FCS keywords and
//! export FCS.) Every action is an `App` method that returns nothing but sets the status line, so tests can drive
//! them without a menu.

use super::{theme, App, Module};
use crate::session::GraphWindow;
use crate::stats_table::{node_stat, Metric};
use eframe::egui::{self, Color32, RichText};
use sf_gating::{Gate, GateKind, GateTree, NodeId, NodeKind};

/// Where a right-click happened in a plot: the data-space point, and the gate under it, if any.
pub type MenuTarget = ((f64, f64), Option<NodeId>);

/// A statistic shown under a gate's name on the plot.
#[derive(Debug, Clone, PartialEq)]
pub struct GateStat {
    pub metric: Metric,
    pub channel: String,
}

impl GateStat {
    pub fn label(&self) -> String {
        if self.metric.needs_channel() {
            format!("{} {}", self.metric.label(), self.channel)
        } else {
            self.metric.label()
        }
    }
}

/// Open dialogs of the right-click menus.
#[derive(Default)]
pub struct Dialogs {
    pub color: Option<(usize, NodeId, [u8; 3])>,
    /// (sample, node, metric index into `STAT_CHOICES`, channel)
    pub stat: Option<(usize, NodeId, usize, String)>,
    pub subsample: Option<(usize, NodeId, usize)>,
    pub keywords: Option<usize>,
    pub history: Option<(usize, NodeId)>,
    /// How many times a plot menu has been laid out (so tests can tell a menu opened).
    pub plot_menus_shown: u32,
}

pub const STAT_CHOICES: [Metric; 13] = Metric::COMMON;

fn gate_type_name(g: &Gate) -> &'static str {
    match g.kind {
        GateKind::Rectangle => "Rectangle",
        GateKind::Interval => "Interval",
        GateKind::Ellipse => "Ellipse",
        GateKind::Polygon => "Polygon",
        GateKind::Quadrant(_) => "Quadrant",
        GateKind::Indexed(_) => "Event list",
    }
}

impl App {
    // ---- actions ------------------------------------------------------------------------------------

    pub fn gate_copy(&mut self, si: usize, node: NodeId) {
        match self.session.gate_of(si, node) {
            Some(g) if !matches!(g.kind, GateKind::Indexed(_)) => {
                let name = self.session.tree(si).name(node).unwrap_or("gate").to_string();
                self.status = format!("Copied gate '{name}'. Right-click a population or a plot and choose Paste gate.");
                self.ws.gate_clip = Some((g, name));
            }
            Some(_) => self.status = "Subsample and plugin populations hold event numbers of one file and cannot be copied as a gate.".into(),
            None => self.status = "Only a gate can be copied.".into(),
        }
    }

    pub fn gate_paste(&mut self, si: usize, parent: NodeId) {
        let Some((g, name)) = self.ws.gate_clip.clone() else {
            self.status = "Nothing to paste: copy a gate first.".into();
            return;
        };
        // a pasted gate never replaces a sibling of the same name: it gets "name (2)", "name (3)", ...
        let taken: Vec<String> = self.session.tree(si).children(parent).map(|c| c.iter().filter_map(|&k| self.session.tree(si).name(k).ok().map(str::to_string)).collect()).unwrap_or_default();
        let name = if taken.contains(&name) { (2..).map(|i| format!("{name} ({i})")).find(|n| !taken.contains(n)).unwrap() } else { name };
        self.status = match self.session.paste_gate(si, parent, g, &name) {
            Ok(n) => {
                let c = self.session.count(si, n);
                format!("Pasted '{name}' ({c} events). Ctrl+Z to undo.")
            }
            Err(e) => e,
        };
    }

    pub fn gate_to_polygon(&mut self, si: usize, node: NodeId) {
        self.status = match self.session.convert_to_polygon(si, node) {
            Ok(c) => format!("Converted to a polygon ({c} events). Drag its corners; right-click an edge and choose Add node. Ctrl+Z to undo."),
            Err(e) => e,
        };
    }

    pub fn gate_add_node(&mut self, si: usize, node: NodeId, at: (f64, f64)) {
        self.status = match self.session.add_polygon_node(si, node, at) {
            Ok(c) => format!("Added a corner ({c} events). Ctrl+Z to undo."),
            Err(e) => e,
        };
    }

    pub fn gate_exclusion(&mut self, si: usize, node: NodeId) {
        let name = self.session.tree(si).name(node).unwrap_or("population").to_string();
        self.status = match self.session.toggle_exclusion(si, node) {
            Ok(true) => format!("'{name}' is now an exclusion gate: it holds what is outside the gate. Ctrl+Z to undo."),
            Ok(false) => format!("'{name}' is a normal gate again."),
            Err(e) => e,
        };
    }

    pub fn gate_apply_all(&mut self, si: usize, node: NodeId) {
        let name = self.session.tree(si).name(node).unwrap_or("population").to_string();
        self.status = match self.session.apply_to_all(si, node) {
            Ok(n) => format!("Applied '{name}' to {n} other sample(s)."),
            Err(e) => e,
        };
    }

    pub fn gate_delete(&mut self, si: usize, node: NodeId) {
        self.status = match self.session.delete(si, node) {
            Ok(n) => format!("Deleted {n} population(s). Ctrl+Z to undo."),
            Err(e) => e,
        };
    }

    pub fn gate_set_color(&mut self, si: usize, node: NodeId, c: Option<[u8; 3]>) {
        match c {
            Some(c) => {
                self.ws.gate_colors.insert((si, node), c);
            }
            None => {
                self.ws.gate_colors.remove(&(si, node));
            }
        }
    }

    pub fn gate_add_stat(&mut self, si: usize, node: NodeId, metric: Metric, channel: &str) {
        let s = GateStat { metric, channel: channel.to_string() };
        let list = self.ws.gate_stats.entry((si, node)).or_default();
        if !list.contains(&s) {
            list.push(s);
        }
    }

    pub fn gate_remove_stat(&mut self, si: usize, node: NodeId, i: usize) {
        if let Some(l) = self.ws.gate_stats.get_mut(&(si, node)) {
            if i < l.len() {
                l.remove(i);
            }
        }
    }

    /// The text lines of the statistics chosen for a gate, computed once per edit of the sample.
    pub fn gate_stat_lines(&mut self, si: usize, node: NodeId) -> Vec<String> {
        let Some(list) = self.ws.gate_stats.get(&(si, node)).cloned() else { return Vec::new() };
        let version = self.session.version(si);
        list.iter()
            .map(|st| {
                let key = (si, node, format!("{:?}|{}", st.metric, st.channel));
                let v = match self.ws.stat_cache.get(&key) {
                    Some((ver, v)) if *ver == version => *v,
                    _ => {
                        let v = node_stat(&mut self.session, si, node, st.metric, &st.channel);
                        self.ws.stat_cache.insert(key, (version, v));
                        v
                    }
                };
                let text = match (st.metric, v) {
                    (_, None) => "–".to_string(),
                    (Metric::Count, Some(v)) => format!("{v:.0}"),
                    (Metric::PctParent | Metric::PctTotal | Metric::Cv | Metric::RobustCv, Some(v)) => format!("{v:.1}"),
                    (_, Some(v)) => format!("{v:.1}"),
                };
                format!("{}: {text}", st.label())
            })
            .collect()
    }

    /// The population as an FCS file (the events it holds, all channels, the file's own keywords).
    pub fn population_fcs_bytes(&mut self, si: usize, node: NodeId) -> Result<Vec<u8>, String> {
        let rows: Vec<usize> = {
            let ev = self.session.evaluation(si)?;
            ev.masks.get(node).and_then(|m| m.as_ref()).ok_or("That population no longer exists")?.indices()
        };
        let s = self.session.sample(si);
        let d = s.channels.len();
        let params: Vec<sf_fcs::Param> = (0..d).map(|i| sf_fcs::Param { name: s.channels[i].clone(), stain: s.labels.get(i).cloned().unwrap_or_default(), range: s.ranges[i], bits: 32, exponent: "0,0".into() }).collect();
        let mut events = Vec::with_capacity(rows.len() * d);
        for r in &rows {
            events.extend_from_slice(&s.events[r * d..(r + 1) * d]);
        }
        let mut extra: Vec<(String, String)> = s.keywords.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        extra.sort();
        sf_fcs::write_fcs(&params, &events, &extra)
    }

    fn export_population_fcs(&mut self, si: usize, node: NodeId) {
        let stem = format!("{}_{}", self.session.sample(si).name.trim_end_matches(".fcs"), self.session.tree(si).name(node).unwrap_or("population")).replace(['/', '\\', ':', ' ', '>'], "_");
        let Some(p) = rfd::FileDialog::new().add_filter("FCS", &["fcs"]).set_file_name(format!("{stem}.fcs")).save_file() else { return };
        self.status = match self.population_fcs_bytes(si, node).and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string())) {
            Ok(()) => format!("Saved {}.", p.display()),
            Err(e) => format!("Could not export the population: {e}"),
        };
    }

    fn goto_module(&mut self, m: Module, si: usize, node: NodeId) {
        self.session.selected = Some((si, node));
        self.module = m;
    }

    /// The topmost child gate of the window's population that contains a display-space point.
    pub fn gate_under(&self, w: &GraphWindow, at: (f64, f64)) -> Option<NodeId> {
        let kids = self.session.tree(w.sample).children(w.node).ok()?;
        kids.iter().rev().copied().find(|&k| matches!(self.session.tree(w.sample).kind(k), Ok(NodeKind::Gate(g)) if super::graph::gate_on_axes(g, w) && !matches!(g.kind, GateKind::Indexed(_)) && g.contains(at.0, at.1) != g.invert))
    }

    // ---- the plot's menu ----------------------------------------------------------------------------

    /// The menu opened by a right-click on a plot.
    pub(super) fn plot_menu(&mut self, ui: &mut egui::Ui, id: u64) {
        let Some(w) = self.session.window(id).cloned() else { return };
        self.dlg.plot_menus_shown += 1;
        let target: Option<MenuTarget> = self.ws.graphs.get(&id).and_then(|g| g.menu);
        let si = w.sample;
        if let Some((at, Some(node))) = target {
            let (name, gate) = (self.session.tree(si).name(node).unwrap_or("gate").to_string(), self.session.gate_of(si, node));
            ui.label(RichText::new(&name).strong().color(theme::MUTED));
            ui.separator();
            if ui.button("Open in new window").clicked() {
                ui.close_menu();
                if let Err(e) = self.open_tab(si, node) {
                    self.status = e;
                }
            }
            if ui.button("Rename…").clicked() {
                ui.close_menu();
                self.ws.pops_open = true;
                self.ws.tree_sample = si;
                self.ws.rename = Some((si, node, name.clone()));
            }
            let kind = gate.as_ref().map(|g| g.kind.clone());
            let convertible = matches!(kind, Some(GateKind::Rectangle | GateKind::Ellipse));
            if ui.add_enabled(convertible, egui::Button::new("Convert to polygon")).on_hover_text("Rectangles and ellipses only: lets you shape the gate corner by corner").clicked() {
                ui.close_menu();
                self.gate_to_polygon(si, node);
            }
            if ui.add_enabled(matches!(kind, Some(GateKind::Polygon)), egui::Button::new("Add node")).on_hover_text("Add a corner to a polygon, on the edge nearest the click").clicked() {
                ui.close_menu();
                self.gate_add_node(si, node, at);
            }
            if ui.button("Add Statistic…").clicked() {
                ui.close_menu();
                self.dlg.stat = Some((si, node, 3, w.x_chan.clone()));
            }
            if ui.button("Color…").clicked() {
                ui.close_menu();
                let c = self.ws.gate_colors.get(&(si, node)).copied().unwrap_or([0xC0, 0x39, 0x2B]);
                self.dlg.color = Some((si, node, c));
            }
            if ui.button("Gate history…").clicked() {
                ui.close_menu();
                self.dlg.history = Some((si, node));
            }
            ui.separator();
            if ui.button("Copy gate").clicked() {
                ui.close_menu();
                self.gate_copy(si, node);
            }
            if ui.button("Apply gate → all samples").clicked() {
                ui.close_menu();
                self.gate_apply_all(si, node);
            }
            let inverted = gate.as_ref().map_or(false, |g| g.invert);
            if ui.add_enabled(gate.is_some(), egui::Button::new(if inverted { "Clear exclusion gate" } else { "Set as exclusion gate" })).clicked() {
                ui.close_menu();
                self.gate_exclusion(si, node);
            }
            ui.separator();
            if ui.button(RichText::new("Delete").color(theme::GATE_RED)).clicked() {
                ui.close_menu();
                self.gate_delete(si, node);
            }
            return;
        }
        // not on a gate: the plot itself
        if ui.add_enabled(self.ws.gate_clip.is_some(), egui::Button::new("Paste gate here")).on_hover_text("Adds the copied gate under this population").clicked() {
            ui.close_menu();
            self.gate_paste(si, w.node);
        }
        ui.separator();
        if ui.button("Export editor…").clicked() {
            ui.close_menu();
            self.open_plot_export(id);
        }
        let canvas = self.ws.graphs.get(&id).and_then(|g| g.canvas_rect);
        if ui.add_enabled(canvas.is_some(), egui::Button::new("Copy image")).clicked() {
            ui.close_menu();
            if let Some(rect) = canvas {
                self.begin_capture(rect, super::CaptureDest::Clipboard);
            }
        }
        ui.separator();
        if ui.button("Fit view to population").clicked() {
            ui.close_menu();
            self.refit_window(id);
        }
        let hist = w.kind == crate::density::PlotKind::Histogram;
        ui.menu_button("Plot type", |ui| {
            for k in crate::density::PlotKind::ALL {
                if ui.selectable_label(w.kind == k, k.label()).clicked() {
                    ui.close_menu();
                    if let Some(win) = self.session.window_mut(id) {
                        win.kind = k;
                    }
                    self.refit_window(id);
                }
            }
        });
        ui.menu_button("Axes", |ui| {
            let mut act: Option<(bool, i32)> = None;
            if ui.add_enabled(!hist, egui::Button::new("Transpose axes")).clicked() {
                act = Some((true, 0));
            }
            for (label, x, d) in [("Next X", true, 1), ("Previous X", true, -1), ("Next Y", false, 1), ("Previous Y", false, -1)] {
                if ui.add_enabled(x || !hist, egui::Button::new(label)).clicked() {
                    act = Some((x, d));
                }
            }
            if let Some((x, d)) = act {
                ui.close_menu();
                self.axis_action(id, x, d);
            }
        });
        let fmo_set = self.session.fmo(&w.x_chan).is_some() || (!hist && self.session.fmo(&w.y_chan).is_some());
        if ui.button(if fmo_set { "Clear FMO" } else { "Set FMO from this population" }).clicked() {
            ui.close_menu();
            self.toggle_fmo_public(id);
        }
        let open = self.ws.graphs.get(&id).map_or(false, |g| g.strip_open);
        if ui.button(if open { "Hide compare strip" } else { "Compare across samples" }).clicked() {
            ui.close_menu();
            self.set_strip_open(id, !open);
        }
    }

    // ---- the population tree's menu -----------------------------------------------------------------

    /// Extra entries of a population's tree menu, after its open / rename entries.
    pub(super) fn tree_menu_items(&mut self, ui: &mut egui::Ui, si: usize, node: NodeId) {
        ui.separator();
        if ui.button("Compute statistics…").clicked() {
            ui.close_menu();
            let n = self.session.samples().len();
            self.stats.selected = (0..n).map(|i| i == si).collect();
            self.goto_module(Module::Statistics, si, node);
        }
        if ui.button("Cell cycle analysis…").clicked() {
            ui.close_menu();
            self.goto_module(Module::CellCycle, si, node);
        }
        ui.separator();
        let is_gate = self.session.gate_of(si, node).map_or(false, |g| !matches!(g.kind, GateKind::Indexed(_)));
        if node != GateTree::ROOT {
            if ui.add_enabled(is_gate, egui::Button::new("Copy gate")).clicked() {
                ui.close_menu();
                self.gate_copy(si, node);
            }
        }
        let paste = format!("Paste gate{}", if node == GateTree::ROOT { " (under All Events)" } else { " here" });
        if ui.add_enabled(self.ws.gate_clip.is_some(), egui::Button::new(paste)).clicked() {
            ui.close_menu();
            self.gate_paste(si, node);
        }
        if node != GateTree::ROOT {
            let inverted = self.session.gate_of(si, node).map_or(false, |g| g.invert);
            if ui.add_enabled(self.session.gate_of(si, node).is_some(), egui::Button::new(if inverted { "Clear exclusion gate" } else { "Set as exclusion gate" })).clicked() {
                ui.close_menu();
                self.gate_exclusion(si, node);
            }
        }
        ui.separator();
        if ui.button("Sub sample…").clicked() {
            ui.close_menu();
            let have = self.session.count(si, node);
            self.dlg.subsample = Some((si, node, (have / 2).clamp(1, 50_000)));
        }
        if node == GateTree::ROOT && ui.button("FCS keywords…").clicked() {
            ui.close_menu();
            self.dlg.keywords = Some(si);
        }
        if ui.button("Export FCS…").on_hover_text("Save this population's events as a new FCS file").clicked() {
            ui.close_menu();
            self.export_population_fcs(si, node);
        }
    }

    // ---- dialogs ------------------------------------------------------------------------------------

    /// Open one of the right-click dialogs by name (command line, screenshots).
    pub fn set_dialog(&mut self, name: &str, si: usize, node: NodeId, channel: &str) {
        match name {
            "stat" => self.dlg.stat = Some((si, node, 3, channel.to_string())),
            "history" => self.dlg.history = Some((si, node)),
            "keywords" => self.dlg.keywords = Some(si),
            "color" => self.dlg.color = Some((si, node, [0xC0, 0x39, 0x2B])),
            "subsample" => self.dlg.subsample = Some((si, node, 1000)),
            _ => {}
        }
    }

    pub(crate) fn context_dialogs(&mut self, ctx: &egui::Context) {
        // colour
        if let Some((si, node, mut c)) = self.dlg.color {
            let mut open = true;
            let mut done: Option<&str> = None;
            egui::Window::new("Gate colour").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                let mut col = Color32::from_rgb(c[0], c[1], c[2]);
                egui::color_picker::color_picker_color32(ui, &mut col, egui::color_picker::Alpha::Opaque);
                c = [col.r(), col.g(), col.b()];
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        done = Some("apply");
                    }
                    if ui.button("Default colour").clicked() {
                        done = Some("reset");
                    }
                    if ui.button("Cancel").clicked() {
                        done = Some("cancel");
                    }
                });
            });
            match done {
                Some("apply") => {
                    self.gate_set_color(si, node, Some(c));
                    self.dlg.color = None;
                }
                Some("reset") => {
                    self.gate_set_color(si, node, None);
                    self.dlg.color = None;
                }
                Some(_) => self.dlg.color = None,
                None => self.dlg.color = if open { Some((si, node, c)) } else { None },
            }
        }
        // statistic on the plot
        if let Some((si, node, mut mi, mut ch)) = self.dlg.stat.clone() {
            let mut open = true;
            let mut add = false;
            let channels = self.session.sample(si).channels.clone();
            let name = self.session.tree(si).name(node).unwrap_or("gate").to_string();
            egui::Window::new(format!("Statistics on '{name}'")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                theme::caption(ui, "Shown under the gate's name on every plot of it.");
                egui::ComboBox::from_id_salt("gs_metric").width(200.0).selected_text(STAT_CHOICES[mi].label()).show_ui(ui, |ui| {
                    for (i, m) in STAT_CHOICES.iter().enumerate() {
                        ui.selectable_value(&mut mi, i, m.label());
                    }
                });
                ui.add_enabled_ui(STAT_CHOICES[mi].needs_channel(), |ui| {
                    egui::ComboBox::from_id_salt("gs_chan").width(200.0).selected_text(ch.clone()).show_ui(ui, |ui| {
                        for c in &channels {
                            ui.selectable_value(&mut ch, c.clone(), c);
                        }
                    });
                });
                if ui.button("Add").clicked() {
                    add = true;
                }
                ui.separator();
                let list = self.ws.gate_stats.get(&(si, node)).cloned().unwrap_or_default();
                if list.is_empty() {
                    theme::caption(ui, "No statistics yet.");
                }
                let mut remove = None;
                for (i, s) in list.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(s.label());
                        if ui.small_button("✕").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    self.gate_remove_stat(si, node, i);
                }
            });
            if add {
                self.gate_add_stat(si, node, STAT_CHOICES[mi], &ch);
            }
            self.dlg.stat = if open { Some((si, node, mi, ch)) } else { None };
        }
        // sub sample
        if let Some((si, node, mut n)) = self.dlg.subsample {
            let mut open = true;
            let mut go = false;
            let have = self.session.count(si, node);
            egui::Window::new("Sub sample").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                theme::caption(ui, &format!("A random subset of this population's {have} events, as a new population. Seeded: the same number gives the same events."));
                ui.add(egui::DragValue::new(&mut n).range(1..=have.max(1)).speed(50.0).suffix(" events"));
                if ui.button("Create").clicked() {
                    go = true;
                }
            });
            if go {
                self.status = match self.session.subsample(si, node, n, 1, &format!("{n} events")) {
                    Ok(_) => format!("Created a subsample of {n} events."),
                    Err(e) => e,
                };
                open = false;
            }
            self.dlg.subsample = if open { Some((si, node, n)) } else { None };
        }
        // FCS keywords
        if let Some(si) = self.dlg.keywords {
            let mut open = true;
            let mut kws: Vec<(String, String)> = self.session.sample(si).keywords.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            kws.sort();
            egui::Window::new(format!("FCS keywords — {}", self.session.sample(si).name)).open(&mut open).default_size([520.0, 420.0]).show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    egui::Grid::new("kw_grid").striped(true).num_columns(2).show(ui, |ui| {
                        for (k, v) in &kws {
                            ui.label(RichText::new(k).monospace().color(theme::MUTED));
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                });
            });
            if !open {
                self.dlg.keywords = None;
            }
        }
        // gate history
        if let Some((si, node)) = self.dlg.history {
            let mut open = true;
            let mut undo = false;
            let name = self.session.tree(si).name(node).unwrap_or("gate").to_string();
            let gate = self.session.gate_of(si, node);
            let (count, pct) = (self.session.count(si, node), {
                let parent = self.session.tree(si).parent(node).ok().flatten();
                let denom = parent.map(|p| self.session.count(si, p)).unwrap_or(self.session.sample(si).n_events);
                if denom > 0 { 100.0 * self.session.count(si, node) as f64 / denom as f64 } else { f64::NAN }
            });
            let can_undo = self.session.can_undo(si);
            egui::Window::new(format!("Gate history — {name}")).open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                match &gate {
                    Some(g) => {
                        egui::Grid::new("gh_grid").num_columns(2).show(ui, |ui| {
                            let mut row = |a: &str, b: String| {
                                ui.label(RichText::new(a).color(theme::MUTED));
                                ui.label(b);
                                ui.end_row();
                            };
                            row("Type", format!("{}{}", gate_type_name(g), if g.invert { " (exclusion)" } else { "" }));
                            row("X", format!("{}  [{}]", g.x_chan, g.x_scale.to_text()));
                            if let Some(y) = &g.y_chan {
                                row("Y", format!("{y}  [{}]", g.y_scale.to_text()));
                            }
                            row("X values", g.xs.iter().map(|v| format!("{v:.1}")).collect::<Vec<_>>().join(", "));
                            if !g.ys.is_empty() {
                                row("Y values", g.ys.iter().map(|v| format!("{v:.1}")).collect::<Vec<_>>().join(", "));
                            }
                            row("Events", format!("{count}  ({pct:.2}% of parent)"));
                        });
                    }
                    None => {
                        ui.label("This population is not defined by a single gate.");
                    }
                }
                ui.separator();
                theme::caption(ui, "Every change to the gates of this sample (drawing, moving, resizing, converting, deleting) is one undo step.");
                if ui.add_enabled(can_undo, egui::Button::new("Undo the last change to this sample")).clicked() {
                    undo = true;
                }
            });
            if undo {
                self.do_undo(si);
            }
            if !open {
                self.dlg.history = None;
            }
        }
    }
}

