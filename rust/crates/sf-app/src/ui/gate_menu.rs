//! Right-click menus and their dialogs: the gate menu on a plot, the plot's own menu, the population tree menu.
//! (The previous app's menus: open in a new window, rename, convert to polygon, add node, add statistic, colour,
//! gate history, copy / paste gate, apply to all samples, exclusion gate, delete, sub sample, FCS keywords and
//! export FCS.) Every action is an `App` method that returns nothing but sets the status line, so tests can drive
//! them without a menu.

use super::{theme, App, Module};
use crate::session::{DrawTarget, GroupDelete, ALL_SAMPLES};
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
    /// "What is this population?": (sample, node, the comparison, the suggested name).
    pub describe: Option<(usize, NodeId, crate::describe::Description, String)>,
    /// "Carry to all events": the population, what was learned from, the suggested gating path, the last result.
    pub carry: Option<CarryDlg>,
    /// A group-owned gate whose deletion is being decided.
    pub group_delete: Option<(usize, NodeId)>,
    pub mgr: GroupMgr,
    /// How many times a plot menu has been laid out (so tests can tell a menu opened).
    pub plot_menus_shown: u32,
}

#[derive(Clone)]
pub struct CarryDlg {
    pub sample: usize,
    pub training: crate::carry::Training,
    pub path: Vec<crate::carry::PathStep>,
    pub result: Option<String>,
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
        if self.session.gate_owner(si, node).is_some() {
            self.dlg.group_delete = Some((si, node));
            return;
        }
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
                self.start_rename(si, node);
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
            ui.menu_button("Apply gate → group", |ui| self.apply_group_menu(ui, si, node));
            if let Some(o) = self.session.gate_owner(si, node) {
                ui.separator();
                ui.label(RichText::new(format!("Owned by {}{}", o.group, if o.edited { " — edited here" } else { "" })).size(12.0).color(if o.edited { theme::WARN } else { theme::MUTED }));
                if ui.add_enabled(o.edited, egui::Button::new("Update group from this gate")).on_hover_text("Make this version the group's and give it to every member").clicked() {
                    ui.close_menu();
                    self.status = match self.session.update_group_from(si, node) {
                        Ok(n) => format!("The group's gate is now this version ({n} other sample(s) updated)."),
                        Err(e) => e,
                    };
                }
                if ui.add_enabled(o.edited, egui::Button::new("Revert to group version")).clicked() {
                    ui.close_menu();
                    self.status = match self.session.revert_to_group(si, node) {
                        Ok(c) => format!("Back to the group's version ({c} events)."),
                        Err(e) => e,
                    };
                }
                if ui.button("Release from group").on_hover_text("Every sample keeps its own copy; the group stops owning it").clicked() {
                    ui.close_menu();
                    self.status = match self.session.release_gate(si, node) {
                        Ok(()) => "Released: each sample now owns its copy.".into(),
                        Err(e) => e,
                    };
                }
                if ui.button("Delete from group…").clicked() {
                    ui.close_menu();
                    self.dlg.group_delete = Some((si, node));
                }
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
        if w.node != GateTree::ROOT && self.session.gate_of(si, w.node).is_some() {
            let pop = self.session.tree(si).name(w.node).unwrap_or("population").to_string();
            ui.menu_button(format!("Apply gate '{pop}' → group"), |ui| self.apply_group_menu(ui, si, w.node));
            if ui.button(format!("Copy gate '{pop}'")).clicked() {
                ui.close_menu();
                self.gate_copy(si, w.node);
            }
        }
        ui.separator();
        if ui.button("Export editor…").clicked() {
            ui.close_menu();
            self.open_plot_export(id);
        }
        if ui.button("Copy image").on_hover_text("Copies the plot with your export settings (Export editor, Use as my default)").clicked() {
            ui.close_menu();
            self.quick_copy_plot(id);
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

    /// Learn what a map population looks like on the map's markers and open the two ways of carrying it over.
    pub fn carry_open(&mut self, si: usize, node: NodeId) {
        let features = if self.session.map_features.len() >= 2 { self.session.map_features.clone() } else { super::discover_view::default_features(self.session.sample(si)) };
        let cofactor = self.session.map_cofactor;
        match crate::carry::training(&mut self.session, si, node, &features, cofactor, 30_000) {
            Ok(training) => {
                let path = crate::carry::suggest_path(&training, 4);
                self.status = format!("Learned '{}' from {} pooled events inside and {} outside, in {} sample(s).", training.name(), training.n_in, training.n_out, training.samples);
                self.dlg.carry = Some(CarryDlg { sample: si, training, path, result: None });
            }
            Err(e) => self.status = format!("Cannot carry this population over: {e}"),
        }
    }

    /// Put every event of every sample inside or outside the population with a classifier.
    pub fn carry_with_classifier(&mut self) {
        let Some(dlg) = self.dlg.carry.clone() else { return };
        let msg = match crate::carry::carry_classifier(&mut self.session, &dlg.training, 42) {
            Ok(c) => {
                let total: usize = c.per_sample.iter().map(|p| p.1).sum();
                let weak = c.quality.recall < 0.8 || c.quality.precision < 0.8;
                format!("'{}' added to {} sample(s), {total} events in all. On pooled events held back from training the classifier {}{}.", c.name, c.per_sample.len(), c.quality.words(), if weak { " — that is weak: the population is not well defined by these markers, so treat the result with care" } else { "" })
            }
            Err(e) => format!("Could not carry the population over: {e}"),
        };
        self.status = msg.clone();
        if let Some(d) = self.dlg.carry.as_mut() {
            d.result = Some(msg);
        }
    }

    /// Draw the suggested gating path as ordinary nested gates on every sample.
    pub fn carry_with_gates(&mut self) {
        let Some(dlg) = self.dlg.carry.clone() else { return };
        let labels: std::collections::HashMap<String, String> = dlg.path.iter().map(|s| (s.channel.clone(), self.marker_word(&s.channel))).collect();
        let msg = match crate::carry::make_path_gates(&mut self.session, dlg.sample, &dlg.training, &dlg.path, &|c| labels.get(c).cloned().unwrap_or_else(|| c.to_string())) {
            Ok(last) => {
                let q = dlg.path.last().map(|s| s.quality.words()).unwrap_or_default();
                let n = self.session.count(dlg.sample, last);
                format!("Drew {} nested gate(s) on every sample, ending in '{} (gated)' ({n} events in this sample). On the pooled events the path {q}. They are ordinary gates: adjust them on the plot.", dlg.path.len(), dlg.training.name())
            }
            Err(e) => format!("Could not draw the gating path: {e}"),
        };
        self.status = msg.clone();
        if let Some(d) = self.dlg.carry.as_mut() {
            d.result = Some(msg);
        }
    }

    fn carry_window(&mut self, ctx: &egui::Context) {
        let Some(dlg) = self.dlg.carry.clone() else { return };
        let (mut open, mut classify, mut gates) = (true, false, false);
        let tr = &dlg.training;
        egui::Window::new(format!("Carry '{}' to all events", tr.name())).id(egui::Id::new("carry_pop")).open(&mut open).collapsible(false).resizable(true).default_width(560.0).show(ctx, |ui| {
            theme::caption(ui, format!("A population gated on the map holds only the pooled events. It was learned here from {} pooled events inside it and {} outside, in {} sample(s), on: {}.", tr.n_in, tr.n_out, tr.samples, tr.features.iter().map(|f| self.marker_word(f)).collect::<Vec<_>>().join(", ")));
            ui.add_space(6.0);
            theme::section(ui, "1 · Classifier");
            theme::caption(ui, "A random forest on those markers puts every event of every sample inside or outside. Closest to the map, but a black box: the result is a list of events, not a gate you can redraw. It is tested on a fifth of the pooled events held back from training.");
            if ui.button(format!("Add '{} (all events)' to every sample", tr.name())).clicked() {
                classify = true;
            }
            ui.add_space(8.0);
            theme::section(ui, "2 · Gating path");
            theme::caption(ui, "A few one-marker cuts in sequence that best reproduce the population. Coarser than the classifier, but ordinary gates: readable, adjustable on the plot, and something a methods section can state.");
            if dlg.path.is_empty() {
                ui.label(RichText::new("No single-marker cut reproduces this population any better than taking everything. Use the classifier, or check with “What is this population?” whether it is a distinct population at all.").color(theme::WARN));
            } else {
                egui::Grid::new("carry_path").striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                    for h in ["Step", "Cut", "At (raw units)", "Finds", "Purity"] {
                        ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
                    }
                    ui.end_row();
                    for (k, s) in dlg.path.iter().enumerate() {
                        ui.label(format!("{}", k + 1));
                        ui.label(RichText::new(format!("{}{}", self.marker_word(&s.channel), if s.above { "+" } else { "-" })).strong());
                        ui.label(RichText::new(format!("{} {:.0}", if s.above { "≥" } else { "<" }, s.threshold)).monospace());
                        ui.label(RichText::new(format!("{:.0} %", 100.0 * s.quality.recall)).monospace());
                        ui.label(RichText::new(format!("{:.0} %", 100.0 * s.quality.precision)).monospace());
                        ui.end_row();
                    }
                });
                theme::caption(ui, "Finds = share of the population still inside after this step. Purity = share of the selected events that are the population. Both on the pooled events.");
                if ui.button("Draw these gates on every sample").clicked() {
                    gates = true;
                }
            }
            if let Some(r) = &dlg.result {
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new(r).color(if r.contains("weak") || r.starts_with("Could not") { theme::WARN } else { theme::GOOD }));
            }
        });
        if classify {
            self.run_busy("Classifying every event…", |app| app.carry_with_classifier());
        }
        if gates {
            self.carry_with_gates();
        }
        if !open {
            self.dlg.carry = None;
        }
    }

    /// A marker as it is written in a population name: its marker name if it has one, else the short channel name.
    fn marker_word(&self, channel: &str) -> String {
        self.session.marker(channel).unwrap_or_else(|| crate::session::short_name(channel))
    }

    /// Compare a population with the rest of its parent on the fluorescence markers and open the result.
    pub fn describe_population(&mut self, si: usize, node: NodeId) {
        let channels = super::discover_view::default_features(self.session.sample(si));
        match crate::describe::describe(&mut self.session, si, node, &channels, 150.0) {
            Ok(d) => {
                let name = crate::describe::suggest_name(&d, &|c| self.marker_word(c));
                let clear = d.markers.iter().filter(|m| !m.call().is_empty()).count();
                self.status = format!("'{}': {clear} marker(s) clearly set it apart from the rest of its parent{}.", self.session.tree(si).name(node).unwrap_or("population"), if name.is_empty() { String::new() } else { format!(" ({name})") });
                self.dlg.describe = Some((si, node, d, name));
            }
            Err(e) => self.status = format!("Cannot describe this population: {e}"),
        }
    }

    fn describe_window(&mut self, ctx: &egui::Context) {
        let Some((si, node, d, suggested)) = self.dlg.describe.clone() else { return };
        let Ok(name) = self.session.tree(si).name(node).map(|n| n.to_string()) else {
            self.dlg.describe = None;
            return;
        };
        let parent = self.session.tree(si).parent(node).ok().flatten().and_then(|p| self.session.tree(si).name(p).ok().map(|n| n.to_string())).unwrap_or_default();
        let (mut open, mut rename) = (true, false);
        egui::Window::new(format!("What is '{name}'?")).id(egui::Id::new("describe_pop")).open(&mut open).collapsible(false).resizable(true).default_width(520.0).show(ctx, |ui| {
            theme::caption(ui, format!("{} events compared with the other {} events of '{parent}', in {}. \"Higher in\" is the chance that an event of this population is higher on the marker than one outside it: 50 % means no difference.", d.n_in, d.n_out, self.session.samples()[si].name));
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                egui::Grid::new("describe_grid").striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                    for h in ["Marker", "", "Median inside", "Median outside", "Higher in"] {
                        ui.label(RichText::new(h).size(11.0).color(theme::MUTED));
                    }
                    ui.end_row();
                    for m in &d.markers {
                        let call = m.call();
                        ui.label(self.session.channel_label(&m.channel));
                        ui.label(RichText::new(call).strong().color(if call == "+" { theme::GOOD } else { theme::WARN }));
                        ui.label(RichText::new(format!("{:.0}", m.inside)).monospace());
                        ui.label(RichText::new(format!("{:.0}", m.outside)).monospace());
                        let (r, _) = ui.allocate_exact_size(egui::vec2(120.0, 12.0), egui::Sense::hover());
                        ui.painter().rect_filled(r, 2.0, theme::PANEL_2);
                        let mid = r.center().x;
                        let x = r.left() + m.auc as f32 * r.width();
                        ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(mid.min(x), r.top()), egui::pos2(mid.max(x), r.bottom())), 2.0, if call.is_empty() { theme::MUTED } else if call == "+" { theme::GOOD } else { theme::WARN });
                        ui.painter().text(egui::pos2(r.right() + 6.0, r.center().y), egui::Align2::LEFT_CENTER, format!("{:.0} %", 100.0 * m.auc), egui::FontId::proportional(11.0), theme::TEXT);
                        ui.end_row();
                    }
                });
            });
            ui.add_space(6.0);
            if suggested.is_empty() {
                ui.label(RichText::new("No marker clearly sets this population apart from the rest of its parent. It may be a region of a continuous cloud and not a distinct population.").color(theme::WARN));
            } else {
                ui.horizontal(|ui| {
                    ui.label("Suggested name:");
                    ui.label(RichText::new(&suggested).strong());
                    if ui.add_enabled(suggested != name, egui::Button::new("Rename to this")).clicked() {
                        rename = true;
                    }
                });
                theme::caption(ui, "A + or − is given only where at least 85 % of comparisons agree. It describes this sample; check it in the others before relying on it.");
            }
        });
        if rename {
            self.status = match self.session.rename(si, node, &suggested) {
                Ok(()) => format!("Renamed '{name}' to '{suggested}'."),
                Err(e) => e,
            };
        }
        if !open {
            self.dlg.describe = None;
        }
    }

    /// Extra entries of a population's tree menu, after its open / rename entries.
    pub(super) fn tree_menu_items(&mut self, ui: &mut egui::Ui, si: usize, node: NodeId) {
        ui.separator();
        if ui.button("Compute statistics…").clicked() {
            ui.close_menu();
            let n = self.session.samples().len();
            self.stats.selected = (0..n).map(|i| i == si).collect();
            self.goto_module(Module::Statistics, si, node);
        }
        if ui.add_enabled(node != GateTree::ROOT, egui::Button::new("What is this population?")).on_hover_text("The markers that set it apart from the rest of its parent, and a name suggested from them").clicked() {
            ui.close_menu();
            self.describe_population(si, node);
        }
        let on_map = self.session.tree(si).path(node).map_or(false, |p| p.iter().position(|x| x == crate::embed::MAP_POPULATION).map_or(false, |i| i + 1 < p.len()));
        if ui.add_enabled(on_map, egui::Button::new("Carry to all events…")).on_hover_text("A population gated on the map holds only the pooled events. This reaches every event of every sample: with a classifier, or with a short gating path you can read and redraw.").clicked() {
            ui.close_menu();
            self.carry_open(si, node);
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
        self.group_dialogs(ctx);
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

/// The group manager window's state.
#[derive(Default)]
pub struct GroupMgr {
    pub open: bool,
    /// The group being viewed or edited (`None` while making a new one).
    pub sel: Option<String>,
    pub creating: bool,
    pub name: String,
    pub members: Vec<bool>,
    pub color: [u8; 3],
    pub synced: bool,
    pub keyword: String,
    pub msg: String,
    pub del_mode: usize,
}

impl GroupMgr {
    pub(super) fn load(&mut self, s: &crate::session::Session, name: &str) {
        self.sel = Some(name.to_string());
        self.creating = false;
        self.name = name.to_string();
        let idx = s.member_indices(name);
        self.members = (0..s.samples().len()).map(|i| idx.contains(&i)).collect();
        match s.group(name) {
            Some(g) => {
                self.color = g.color;
                self.synced = g.synced;
            }
            None => {
                self.color = [0x80, 0x80, 0x80];
                self.synced = s.all_synced;
            }
        }
    }
}

impl App {
    // ---- group actions ----------------------------------------------------------------------------

    /// Close every right-click dialog and the group manager (screenshots).
    pub fn close_dialogs(&mut self) {
        self.dlg = Dialogs::default();
    }

    /// Open the group manager showing one group.
    pub fn open_group_manager(&mut self, group: &str) {
        self.dlg.mgr.open = true;
        let name = if group == ALL_SAMPLES || self.session.group(group).is_some() { group } else { ALL_SAMPLES };
        let sess = &self.session;
        self.dlg.mgr.load(sess, name);
    }

    pub fn gate_apply_group(&mut self, si: usize, node: NodeId, group: &str) {
        let name = self.session.tree(si).name(node).unwrap_or("population").to_string();
        self.status = match self.session.apply_to_group(si, node, group) {
            Ok(n) => format!("'{name}' now belongs to {group}: applied to {n} other sample(s). Editing it in one sample marks that sample's copy as edited."),
            Err(e) => e,
        };
    }

    /// The next sample (`dir` = +1 / -1) that is in the current group, if there is one.
    pub fn step_sample(&self, sample: usize, dir: i32) -> Option<usize> {
        let members = self.ws.cur_group.as_deref().map(|g| self.session.member_indices(g));
        let n = self.session.samples().len() as i64;
        let mut i = sample as i64 + dir as i64;
        while i >= 0 && i < n {
            if members.as_ref().map_or(true, |m| m.contains(&(i as usize))) {
                return Some(i as usize);
            }
            i += dir as i64;
        }
        None
    }

    /// The "Apply gate → group" submenu: All Samples and every group the sample is in.
    pub(super) fn apply_group_menu(&mut self, ui: &mut egui::Ui, si: usize, node: NodeId) {
        let mut pick: Option<String> = None;
        if ui.button(ALL_SAMPLES).clicked() {
            pick = Some(ALL_SAMPLES.to_string());
        }
        for g in self.session.groups_of(si) {
            if ui.button(&g).clicked() {
                pick = Some(g);
            }
        }
        if let Some(g) = pick {
            ui.close_menu();
            self.gate_apply_group(si, node, &g);
        }
    }

    /// Where a newly drawn gate goes: this sample only, All Samples, or a group.
    pub(super) fn draw_target_combo(&mut self, ui: &mut egui::Ui) {
        let cur = self.session.draw_target.clone();
        let label = match &cur {
            DrawTarget::ThisSample => "this sample only".to_string(),
            DrawTarget::Group(g) => g.clone(),
        };
        egui::ComboBox::from_id_salt("draw_target").width(150.0).selected_text(format!("New gates: {label}")).show_ui(ui, |ui| {
            if ui.selectable_label(cur == DrawTarget::ThisSample, "This sample only").clicked() {
                self.session.draw_target = DrawTarget::ThisSample;
            }
            let all = DrawTarget::Group(ALL_SAMPLES.to_string());
            if ui.selectable_label(cur == all, ALL_SAMPLES).on_hover_text("A gate drawn in one sample appears in every sample").clicked() {
                self.session.draw_target = all;
            }
            for g in self.session.groups.clone() {
                let t = DrawTarget::Group(g.name.clone());
                if ui.selectable_label(cur == t, &g.name).clicked() {
                    self.session.draw_target = t;
                }
            }
        })
        .response
        .on_hover_text("Gates owned by a group are the same gate in every member sample. Editing one in a single sample marks it as edited; synchronized groups move together.");
    }

    pub(crate) fn group_dialogs(&mut self, ctx: &egui::Context) {
        // delete a group-owned gate
        if let Some((si, node)) = self.dlg.group_delete {
            let mut open = true;
            let mut choice: Option<GroupDelete> = None;
            let owner = self.session.gate_owner(si, node);
            let name = self.session.tree(si).name(node).unwrap_or("gate").to_string();
            egui::Window::new("Delete group gate?").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
                let g = owner.as_ref().map_or("a group".to_string(), |o| o.group.clone());
                ui.label(format!("'{name}' is shared by the samples of '{g}'."));
                theme::caption(ui, "Choose what happens to the copies:");
                if ui.button("Delete from every sample").on_hover_text("Removes all copies, except in samples where it was edited").clicked() {
                    choice = Some(GroupDelete::Entirely);
                }
                if ui.button("Keep only in the first sample").on_hover_text("Removes it from the other samples and leaves one copy").clicked() {
                    choice = Some(GroupDelete::KeepInFirstSample);
                }
                if ui.button("Give a copy to each sample").on_hover_text("The group stops owning it; every sample keeps its own gate").clicked() {
                    choice = Some(GroupDelete::ToSamples);
                }
            });
            match choice {
                Some(mode) => {
                    self.status = match self.session.delete_from_group(si, node, mode) {
                        Ok(n) => match mode {
                            GroupDelete::ToSamples => format!("'{name}' is no longer shared; every sample keeps its own copy."),
                            _ => format!("Removed {n} copy(ies) of '{name}'. Ctrl+Z undoes it in the current sample."),
                        },
                        Err(e) => e,
                    };
                    self.dlg.group_delete = None;
                }
                None if !open => self.dlg.group_delete = None,
                None => {}
            }
        }
        self.group_manager(ctx);
        self.markers_window(ctx);
        self.describe_window(ctx);
        self.carry_window(ctx);
    }

    fn group_manager(&mut self, ctx: &egui::Context) {
        if !self.dlg.mgr.open {
            return;
        }
        let mut open = true;
        let n = self.session.samples().len();
        if self.dlg.mgr.members.len() != n {
            self.dlg.mgr.members.resize(n, false);
        }
        let mut keywords: Vec<String> = self.session.samples().iter().flat_map(|s| s.keywords.keys().cloned()).filter(|k| !k.starts_with("$P") && !k.starts_with("$BEGIN") && !k.starts_with("$END")).collect();
        keywords.sort();
        keywords.dedup();
        let names: Vec<String> = self.session.samples().iter().map(|s| s.name.clone()).collect();
        let mut mgr = std::mem::take(&mut self.dlg.mgr);
        egui::Window::new("Groups").open(&mut open).default_size([700.0, 480.0]).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(210.0);
                    ui.label(RichText::new("Groups").strong());
                    let mut load: Option<String> = None;
                    let all_label = format!("{ALL_SAMPLES}  ({n})");
                    if ui.selectable_label(mgr.sel.as_deref() == Some(ALL_SAMPLES) && !mgr.creating, all_label).clicked() {
                        load = Some(ALL_SAMPLES.to_string());
                    }
                    for g in &self.session.groups {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 2.0, Color32::from_rgb(g.color[0], g.color[1], g.color[2]));
                            let cur = self.ws.cur_group.as_deref() == Some(g.name.as_str());
                            let text = format!("{}  ({}){}", g.name, g.members.len(), if cur { "  ◀ current" } else { "" });
                            if ui.selectable_label(mgr.sel.as_deref() == Some(g.name.as_str()) && !mgr.creating, text).clicked() {
                                load = Some(g.name.clone());
                            }
                        });
                    }
                    if let Some(g) = load {
                        mgr.load(&self.session, &g);
                    }
                    ui.separator();
                    if ui.button("＋ New group…").clicked() {
                        mgr.creating = true;
                        mgr.sel = None;
                        mgr.name = format!("Group {}", self.session.groups.len() + 1);
                        mgr.members = vec![false; n];
                        mgr.color = [0x1F, 0x6F, 0xEB];
                        mgr.synced = false;
                    }
                    if ui.button("Export group…").clicked() {
                        self.gx.group = mgr.sel.clone().filter(|g| g != "" && !mgr.creating).unwrap_or_else(|| ALL_SAMPLES.to_string());
                        self.gx.open = true;
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("From a keyword").size(12.0).color(theme::MUTED));
                    egui::ComboBox::from_id_salt("grp_kw").width(190.0).selected_text(if mgr.keyword.is_empty() { "Choose keyword…".to_string() } else { mgr.keyword.clone() }).show_ui(ui, |ui| {
                        for k in &keywords {
                            ui.selectable_value(&mut mgr.keyword, k.clone(), k);
                        }
                    });
                    if ui.add_enabled(!mgr.keyword.is_empty(), egui::Button::new("One group per value")).on_hover_text("Makes a group for each distinct value of the keyword").clicked() {
                        mgr.msg = match self.session.groups_from_keyword(&mgr.keyword) {
                            Ok(made) => format!("Made {} group(s): {}", made.len(), made.join(", ")),
                            Err(e) => e,
                        };
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    let is_all = mgr.sel.as_deref() == Some(ALL_SAMPLES) && !mgr.creating;
                    if mgr.sel.is_none() && !mgr.creating {
                        theme::caption(ui, "Choose a group, or make a new one.");
                    } else if is_all {
                        theme::caption(ui, "All Samples holds every loaded sample. It cannot be edited or deleted.");
                        if ui.checkbox(&mut mgr.synced, "Synchronized").on_hover_text("Gates owned by All Samples move together in every sample").changed() {
                            self.session.all_synced = mgr.synced;
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            ui.text_edit_singleline(&mut mgr.name);
                            ui.color_edit_button_srgb(&mut mgr.color).on_hover_text("Tint of the gates this group owns");
                        });
                        ui.checkbox(&mut mgr.synced, "Synchronized").on_hover_text("When a gate this group owns is moved in one sample, every other sample's copy moves too");
                    }
                    if mgr.sel.is_some() || mgr.creating {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Samples").strong());
                            if !is_all {
                                if ui.small_button("All").clicked() {
                                    mgr.members.iter_mut().for_each(|m| *m = true);
                                }
                                if ui.small_button("None").clicked() {
                                    mgr.members.iter_mut().for_each(|m| *m = false);
                                }
                                if let Some(c) = Some(self.ws.tree_sample).filter(|c| *c < n) {
                                    if ui.small_button("This sample").clicked() {
                                        mgr.members[c] = true;
                                    }
                                }
                            }
                        });
                        egui::ScrollArea::vertical().id_salt("grp_members").max_height(230.0).show(ui, |ui| {
                            for (i, nm) in names.iter().enumerate() {
                                ui.add_enabled(!is_all, egui::Checkbox::new(&mut mgr.members[i], nm));
                            }
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            if mgr.creating {
                                if ui.button(RichText::new("Create group").strong()).clicked() {
                                    let idx: Vec<usize> = (0..n).filter(|&i| mgr.members[i]).collect();
                                    match self.session.create_group(&mgr.name, &idx) {
                                        Ok(()) => {
                                            let nm = mgr.name.trim().to_string();
                                            let (c, sy) = (mgr.color, mgr.synced);
                                            let _ = self.session.modify_group(&nm, &nm, &idx, c, sy);
                                            mgr.load(&self.session, &nm);
                                            mgr.msg = format!("Created '{nm}' with {} sample(s).", idx.len());
                                        }
                                        Err(e) => mgr.msg = e,
                                    }
                                }
                            } else if !is_all {
                                let old = mgr.sel.clone().unwrap_or_default();
                                if ui.button(RichText::new("Apply changes").strong()).clicked() {
                                    let idx: Vec<usize> = (0..n).filter(|&i| mgr.members[i]).collect();
                                    match self.session.modify_group(&old, &mgr.name, &idx, mgr.color, mgr.synced) {
                                        Ok(()) => {
                                            let nm = mgr.name.trim().to_string();
                                            if self.ws.cur_group.as_deref() == Some(old.as_str()) {
                                                self.ws.cur_group = Some(nm.clone());
                                            }
                                            mgr.load(&self.session, &nm);
                                            mgr.msg = format!("Saved '{nm}'.");
                                        }
                                        Err(e) => mgr.msg = e,
                                    }
                                }
                                let cur = self.ws.cur_group.as_deref() == Some(old.as_str());
                                if ui.button(if cur { "Stop using as current group" } else { "Use as current group" }).on_hover_text("Arrow keys and the sample arrows step through this group's samples; layouts and group export start from it").clicked() {
                                    self.ws.cur_group = if cur { None } else { Some(old.clone()) };
                                }
                                let owns = self.session.applied.iter().filter(|a| a.group == old).count();
                                egui::ComboBox::from_id_salt("grp_del_mode").width(190.0).selected_text(["Delete its gates from the samples", "Keep its gates in the first sample", "Give its gates to the samples"][mgr.del_mode.min(2)]).show_ui(ui, |ui| {
                                    for (i, t) in ["Delete its gates from the samples", "Keep its gates in the first sample", "Give its gates to the samples"].iter().enumerate() {
                                        ui.selectable_value(&mut mgr.del_mode, i, *t);
                                    }
                                });
                                if ui.button(RichText::new(format!("Delete group ({owns} gate(s))")).color(theme::GATE_RED)).clicked() {
                                    let mode = [GroupDelete::Entirely, GroupDelete::KeepInFirstSample, GroupDelete::ToSamples][mgr.del_mode.min(2)];
                                    mgr.msg = match self.session.delete_group(&old, mode) {
                                        Ok(k) => format!("Deleted '{old}' ({k} gate copies removed)."),
                                        Err(e) => e,
                                    };
                                    if self.ws.cur_group.as_deref() == Some(old.as_str()) {
                                        self.ws.cur_group = None;
                                    }
                                    mgr.sel = None;
                                }
                            }
                        });
                    }
                    if !mgr.msg.is_empty() {
                        ui.add_space(4.0);
                        theme::caption(ui, mgr.msg.clone());
                    }
                });
            });
        });
        mgr.open = open;
        self.dlg.mgr = mgr;
    }
}
