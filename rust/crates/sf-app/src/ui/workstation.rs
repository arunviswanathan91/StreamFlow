//! The Workstation: a collapsible Populations panel on the left, and a tabbed area (Overview + one tab per open
//! population) in the middle. Double-click any population, at any depth, to open it.

use super::graph::GraphUi;
use super::{theme, App};
use crate::density::{bin, render, Range};
use crate::session::default_scale;
use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, TextureHandle, TextureOptions};
use sf_core::scale::ScaleSpec;
use sf_gating::{GateTree, NodeId};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Graph(u64),
}

pub struct State {
    pub pops_open: bool,
    pub options_open: bool,
    pub tab: Tab,
    pub graphs: HashMap<u64, GraphUi>,
    pub collapsed: HashSet<(usize, NodeId)>,
    pub rename: Option<(usize, NodeId, String)>,
    pub filter: String,
    pub tree_sample: usize,
    pub overview_x: String,
    pub overview_y: String,
    thumbs: HashMap<(usize, u64), (u64, TextureHandle)>,
}

impl Default for State {
    fn default() -> Self {
        State {
            pops_open: true,
            options_open: true,
            tab: Tab::Overview,
            graphs: HashMap::new(),
            collapsed: HashSet::new(),
            rename: None,
            filter: String::new(),
            tree_sample: 0,
            overview_x: String::new(),
            overview_y: String::new(),
            thumbs: HashMap::new(),
        }
    }
}

impl State {
    /// Forget everything tied to the loaded session (after opening a workspace or importing one).
    pub fn reset(&mut self) {
        self.tab = Tab::Overview;
        self.graphs.clear();
        self.collapsed.clear();
        self.rename = None;
        self.thumbs.clear();
    }
}

impl App {
    /// Open a population in its own tab (an existing tab is focused instead of duplicated).
    pub fn open_tab(&mut self, sample: usize, node: NodeId) -> Result<u64, String> {
        let id = self.session.open_population(sample, node)?;
        self.ws.tab = Tab::Graph(id);
        Ok(id)
    }

    pub(super) fn workstation(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // keep the active tab pointing at something that exists
        if let Tab::Graph(id) = self.ws.tab {
            if self.session.window(id).is_none() {
                self.ws.tab = self.session.windows.last().map_or(Tab::Overview, |w| Tab::Graph(w.id));
            }
        }
        let live: HashSet<u64> = self.session.windows.iter().map(|w| w.id).collect();
        self.ws.graphs.retain(|k, _| live.contains(k));
        if self.ws.tree_sample >= self.session.samples().len() {
            self.ws.tree_sample = 0;
        }

        if self.ws.pops_open {
            egui::SidePanel::left("pops").resizable(true).default_width(300.0).width_range(230.0..=560.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(10.0))).show_inside(ui, |ui| self.populations_panel(ui));
        } else {
            egui::SidePanel::left("pops_ribbon").exact_width(24.0).resizable(false).frame(egui::Frame::none().fill(theme::PANEL_2)).show_inside(ui, |ui| {
                ui.add_space(6.0);
                if ui.add(egui::Button::new("▶").frame(false)).on_hover_text("Show the populations panel").clicked() {
                    self.ws.pops_open = true;
                }
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::none().fill(theme::BG)).show_inside(ui, |ui| {
            self.tab_strip(ui);
            match self.ws.tab {
                Tab::Overview => self.overview(ui, ctx),
                Tab::Graph(id) => self.graph_tab(ui, ctx, id),
            }
        });
    }

    // ---- populations panel ------------------------------------------------------------------------

    fn populations_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Populations").strong().size(15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new("◀").frame(false)).on_hover_text("Hide this panel").clicked() {
                    self.ws.pops_open = false;
                }
                if ui.add(egui::Button::new("＋ FCS")).on_hover_text("Load FCS files (Ctrl+O)").clicked() {
                    self.open_fcs();
                }
            });
        });
        let n = self.session.samples().len();
        if n == 0 {
            ui.add_space(20.0);
            theme::caption(ui, "No samples yet. Use File ▸ Load FCS… or Import FlowJo workspace….");
            return;
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(self.ws.tree_sample > 0, egui::Button::new("◀")).clicked() {
                self.ws.tree_sample -= 1;
            }
            let name = self.session.samples()[self.ws.tree_sample].name.clone();
            egui::ComboBox::from_id_salt("tree_sample").width(ui.available_width() - 44.0).selected_text(name).show_ui(ui, |ui| {
                for i in 0..n {
                    let nm = self.session.samples()[i].name.clone();
                    ui.selectable_value(&mut self.ws.tree_sample, i, nm);
                }
            });
            if ui.add_enabled(self.ws.tree_sample + 1 < n, egui::Button::new("▶")).clicked() {
                self.ws.tree_sample += 1;
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.ws.filter).hint_text("Filter populations…").desired_width(f32::INFINITY));
        ui.add_space(4.0);
        theme::rule(ui);
        let si = self.ws.tree_sample;
        let nodes = self.session.tree(si).self_and_descendants(GateTree::ROOT).unwrap_or_default();
        let visible = self.visible_nodes(si, &nodes);
        egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
            // iterative pre-order walk: tree depth must never matter (no recursion anywhere)
            let mut hidden_below: Option<usize> = None;
            for node in nodes {
                if !visible.contains(&node) {
                    continue;
                }
                let depth = self.session.tree(si).depth(node).unwrap_or(0);
                if let Some(d) = hidden_below {
                    if depth > d && self.ws.filter.is_empty() {
                        continue;
                    }
                    hidden_below = None;
                }
                if self.ws.collapsed.contains(&(si, node)) && self.ws.filter.is_empty() {
                    hidden_below = Some(depth);
                }
                self.tree_row(ui, si, node, depth);
            }
        });
    }

    /// Nodes to show: everything, or (with a filter) the matches plus their ancestors.
    fn visible_nodes(&self, si: usize, nodes: &[NodeId]) -> HashSet<NodeId> {
        let f = self.ws.filter.trim().to_lowercase();
        if f.is_empty() {
            return nodes.iter().copied().collect();
        }
        let mut keep = HashSet::new();
        for &n in nodes {
            let hit = n != GateTree::ROOT && self.session.tree(si).name(n).map_or(false, |s| s.to_lowercase().contains(&f));
            if hit {
                let mut cur = Some(n);
                while let Some(c) = cur {
                    if !keep.insert(c) {
                        break;
                    }
                    cur = self.session.tree(si).parent(c).ok().flatten();
                }
            }
        }
        keep.insert(GateTree::ROOT);
        keep
    }

    fn tree_row(&mut self, ui: &mut egui::Ui, si: usize, node: NodeId, depth: usize) {
        let has_kids = self.session.tree(si).children(node).map(|c| !c.is_empty()).unwrap_or(false);
        let name = if node == GateTree::ROOT { "All Events".to_string() } else { self.session.tree(si).name(node).unwrap_or("?").to_string() };
        let count = self.session.count(si, node);
        let total = self.session.samples()[si].n_events.max(1);
        let parent_count = self.session.tree(si).parent(node).ok().flatten().map(|p| self.session.count(si, p)).unwrap_or(total).max(1);
        let selected = self.session.selected == Some((si, node));
        let opened = self.session.windows.iter().any(|w| w.sample == si && w.node == node);
        let row_h = 24.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(260.0), row_h), Sense::click());
        if selected {
            ui.painter().rect_filled(rect, 6.0, theme::ACCENT_DIM);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 6.0, theme::RAISED);
        }
        let x0 = rect.left() + 6.0 + depth.min(40) as f32 * 14.0;
        // expand / collapse caret (its own click target)
        if has_kids {
            let caret = egui::Rect::from_center_size(egui::pos2(x0 + 7.0, rect.center().y), egui::vec2(16.0, row_h));
            let open = !self.ws.collapsed.contains(&(si, node));
            ui.painter().text(caret.center(), egui::Align2::CENTER_CENTER, if open { "▾" } else { "▸" }, egui::FontId::proportional(13.0), theme::MUTED);
            if resp.clicked() && resp.interact_pointer_pos().map_or(false, |p| caret.contains(p)) {
                if open {
                    self.ws.collapsed.insert((si, node));
                } else {
                    self.ws.collapsed.remove(&(si, node));
                }
                return;
            }
        }
        let name_color = if opened { theme::ACCENT } else { theme::TEXT };
        if let Some((rs, rn, text)) = self.ws.rename.as_mut() {
            if *rs == si && *rn == node {
                let edit_rect = egui::Rect::from_min_max(egui::pos2(x0 + 16.0, rect.top() + 1.0), egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0));
                let r = ui.put(edit_rect, egui::TextEdit::singleline(text));
                r.request_focus();
                if r.lost_focus() {
                    let new = text.clone();
                    let (s, n) = (*rs, *rn);
                    let commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    self.ws.rename = None;
                    if commit {
                        if let Err(e) = self.session.rename(s, n, &new) {
                            self.status = e;
                        }
                    }
                }
                return;
            }
        }
        ui.painter().text(egui::pos2(x0 + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(13.0), name_color);
        let right = format!("{count}   {:.1}%", 100.0 * count as f64 / parent_count as f64);
        ui.painter().text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, right, egui::FontId::monospace(11.5), theme::MUTED);
        if resp.clicked() {
            self.session.selected = Some((si, node));
        }
        if resp.double_clicked() {
            if let Err(e) = self.open_tab(si, node) {
                self.status = e;
            }
        }
        resp.context_menu(|ui| {
            if ui.button("Open in a tab").clicked() {
                ui.close_menu();
                if let Err(e) = self.open_tab(si, node) {
                    self.status = e;
                }
            }
            if node != GateTree::ROOT {
                if ui.button("Rename…").clicked() {
                    ui.close_menu();
                    self.ws.rename = Some((si, node, name.clone()));
                }
                if ui.button("Apply gate to all samples").clicked() {
                    ui.close_menu();
                    self.status = match self.session.apply_to_all(si, node) {
                        Ok(n) => format!("Applied '{name}' to {n} other sample(s)."),
                        Err(e) => e,
                    };
                }
                ui.separator();
                if ui.button(RichText::new("Delete").color(theme::GATE_RED)).clicked() {
                    ui.close_menu();
                    self.status = match self.session.delete(si, node) {
                        Ok(n) => format!("Deleted {n} population(s)."),
                        Err(e) => e,
                    };
                }
            }
        });
    }

    // ---- tabs -----------------------------------------------------------------------------------

    fn tab_strip(&mut self, ui: &mut egui::Ui) {
        egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(8.0, 4.0)).show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("tabs").auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let r = ui.selectable_label(self.ws.tab == Tab::Overview, RichText::new("▦  Overview").size(13.0));
                    if r.clicked() {
                        self.ws.tab = Tab::Overview;
                    }
                    ui.add_space(6.0);
                    let wins: Vec<(u64, String)> = self
                        .session
                        .windows
                        .iter()
                        .map(|w| {
                            let pop = self.session.breadcrumb(w.sample, w.node).last().map(|b| b.0.clone()).unwrap_or_default();
                            (w.id, format!("{} · {}", self.session.samples()[w.sample].name.trim_end_matches(".fcs"), pop))
                        })
                        .collect();
                    let mut close = None;
                    for (id, title) in wins {
                        let active = self.ws.tab == Tab::Graph(id);
                        let r = ui.selectable_label(active, RichText::new(&title).size(13.0));
                        if active {
                            let y = r.rect.bottom() + 1.0;
                            ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.right(), y)], egui::Stroke::new(2.0_f32, theme::ACCENT));
                        }
                        if r.clicked() {
                            self.ws.tab = Tab::Graph(id);
                        }
                        if r.middle_clicked() {
                            close = Some(id);
                        }
                        if ui.add(egui::Button::new(RichText::new("✕").size(11.0).color(theme::MUTED)).frame(false)).on_hover_text("Close tab").clicked() {
                            close = Some(id);
                        }
                        ui.add_space(4.0);
                    }
                    if let Some(id) = close {
                        self.session.remember_view(id);
                        self.session.close_window(id);
                    }
                });
            });
        });
    }

    // ---- overview -------------------------------------------------------------------------------

    fn overview(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::Frame::none().inner_margin(egui::Margin::same(14.0)).show(ui, |ui| {
            if self.session.samples().is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Open FCS files to begin").size(20.0).strong());
                    ui.add_space(6.0);
                    theme::caption(ui, "File ▸ Load FCS…   (Ctrl+O)      or      File ▸ Import FlowJo workspace…");
                    ui.add_space(14.0);
                    if ui.add(egui::Button::new(RichText::new("Load FCS files").size(14.0)).fill(theme::ACCENT_DIM).min_size(egui::vec2(180.0, 34.0))).clicked() {
                        self.open_fcs();
                    }
                });
                return;
            }
            let channels = self.session.samples()[0].channels.clone();
            if self.ws.overview_x.is_empty() || !channels.contains(&self.ws.overview_x) {
                self.ws.overview_x = channels.iter().find(|c| c.eq_ignore_ascii_case("FSC-A")).or(channels.first()).cloned().unwrap_or_default();
            }
            if self.ws.overview_y.is_empty() || !channels.contains(&self.ws.overview_y) {
                self.ws.overview_y = channels.iter().find(|c| c.eq_ignore_ascii_case("SSC-A")).or(channels.get(1)).or(channels.first()).cloned().unwrap_or_default();
            }
            let sel = self.session.selected.filter(|(s, _)| *s < self.session.samples().len());
            let path = sel.map(|(s, n)| self.session.tree(s).path(n).unwrap_or_default()).unwrap_or_default();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Overview").strong().size(16.0));
                ui.add_space(12.0);
                ui.label("X");
                axis_pick(ui, "ov_x", &channels, &mut self.ws.overview_x);
                ui.label("Y");
                axis_pick(ui, "ov_y", &channels, &mut self.ws.overview_y);
                ui.add_space(8.0);
                theme::caption(ui, format!("Population: {}", if path.is_empty() { "All Events".to_string() } else { path.join(" ▸ ") }));
            });
            ui.add_space(8.0);
            let (xc, yc) = (self.ws.overview_x.clone(), self.ws.overview_y.clone());
            let n = self.session.samples().len();
            let mut open: Option<(usize, NodeId)> = None;
            let cols = ((ui.available_width() / 204.0).floor() as usize).max(1);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for row in (0..n).collect::<Vec<usize>>().chunks(cols) {
                    ui.horizontal_top(|ui| {
                    for &si in row {
                        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
                        let node = if refs.is_empty() { Some(GateTree::ROOT) } else { self.session.tree(si).find(&refs) };
                        let name = self.session.samples()[si].name.clone();
                        egui::Frame::none().fill(theme::PANEL_2).rounding(10.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                            ui.vertical(|ui| {
                            ui.set_width(176.0);
                            ui.set_min_height(196.0);
                            ui.label(RichText::new(name.trim_end_matches(".fcs")).size(12.0).strong());
                            let Some(node) = node else {
                                ui.add_space(70.0);
                                theme::caption(ui, "no such population");
                                ui.add_space(70.0);
                                return;
                            };
                            let count = self.session.count(si, node);
                            match self.thumbnail(ctx, si, node, &xc, &yc) {
                                Some(tex) => {
                                    let r = ui.add(egui::Image::new((tex.id(), egui::vec2(176.0, 140.0))).sense(Sense::click())).on_hover_text("Click to open");
                                    if r.clicked() {
                                        open = Some((si, node));
                                    }
                                }
                                None => {
                                    ui.add_space(60.0);
                                    theme::caption(ui, "channel not in this sample");
                                    ui.add_space(60.0);
                                }
                            }
                            theme::caption(ui, format!("{count} events"));
                            });
                        });
                    }
                    });
                    ui.add_space(8.0);
                }
            });
            if let Some((si, node)) = open {
                if let Err(e) = self.open_tab(si, node) {
                    self.status = e;
                }
            }
        });
    }

    /// A cached density thumbnail of a population on two channels.
    fn thumbnail(&mut self, ctx: &egui::Context, si: usize, node: NodeId, xc: &str, yc: &str) -> Option<TextureHandle> {
        let s = self.session.sample(si);
        let (rx, ry) = (s.range_of(xc), s.range_of(yc));
        let (xs, ys): (ScaleSpec, ScaleSpec) = (default_scale(xc, rx), default_scale(yc, ry));
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (self.session.version(si), node, xc, yc, xs.to_text(), ys.to_text()).hash(&mut h);
            h.finish()
        };
        if let Some((k, t)) = self.ws.thumbs.get(&(si, node as u64)) {
            if *k == key {
                return Some(t.clone());
            }
        }
        let pts = self.session.population_points(si, node, xc, yc, &xs, &ys).ok()?;
        let finite: Vec<&(f64, f64)> = pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        let (x0, x1) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (y0, y1) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        let pad = |lo: f64, hi: f64| {
            let span = if hi > lo { hi - lo } else { 1.0 };
            (lo - 0.02 * span, hi + 0.02 * span)
        };
        let ((x0, x1), (y0, y1)) = (pad(x0, x1), pad(y0, y1));
        let grid = bin(pts.iter().copied(), Range { x0, x1, y0, y1 }, 88, 70);
        let mut pixels: Vec<Color32> = render(&grid).into_iter().map(|p| Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3])).collect();
        for p in pixels.iter_mut() {
            if p.a() == 0 {
                *p = Color32::WHITE;
            }
        }
        let tex = ctx.load_texture(format!("thumb{si}_{node}"), egui::ColorImage { size: [88, 70], pixels }, TextureOptions::NEAREST);
        self.ws.thumbs.insert((si, node as u64), (key, tex.clone()));
        Some(tex)
    }
}

fn axis_pick(ui: &mut egui::Ui, id: &str, channels: &[String], value: &mut String) {
    egui::ComboBox::from_id_salt(id).width(110.0).selected_text(value.clone()).show_ui(ui, |ui| {
        for c in channels {
            ui.selectable_value(value, c.clone(), c);
        }
    });
}
