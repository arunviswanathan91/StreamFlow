//! The Layout Editor: a page canvas where samples are dragged from a list to make plot tiles, dropped onto an
//! existing tile to overlay them, moved and resized, and run as a batch (one page per sample or per keyword group).

use super::layout_view::paint_figure;
use super::{theme, App};
use crate::density::PlotKind;
use crate::figure::{to_pdf, Figure, Item};
use crate::layout_doc::{Iterate, LayerRole, LayoutDoc, Tile, ViewMode, MIN_TILE, PALETTE};
use crate::layout_render::{render_batch, render_page};
use crate::stats_table::pop_label;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, TextureOptions};
use sf_gating::GateTree;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sel {
    Tile(u32),
    Text(u32),
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    sel: Sel,
    resize: bool,
}

pub struct State {
    pub doc: LayoutDoc,
    pub selected: Option<Sel>,
    fig: Option<(u64, Figure)>,
    textures: Vec<TextureHandle>,
    /// The page as drawn on screen: its rectangle and the points-to-pixels scale (for hit-testing and tests).
    pub canvas: Option<(Rect, f32)>,
    /// Where each sample row of the list was drawn (drag sources).
    pub sample_rows: Vec<(String, Rect)>,
    drag: Option<Drag>,
    ctx_tile: Option<u32>,
    /// What could not be drawn in the editor view or the last batch.
    pub notes: Vec<String>,
    /// The pages of the last batch run, and the one being previewed (`None` = the editor view).
    pub batch: Vec<Figure>,
    pub preview: Option<usize>,
    batch_textures: Vec<Vec<TextureHandle>>,
}

impl Default for State {
    fn default() -> Self {
        State { doc: LayoutDoc::new("Layout"), selected: None, fig: None, textures: vec![], canvas: None, sample_rows: vec![], drag: None, ctx_tile: None, notes: vec![], batch: vec![], preview: None, batch_textures: vec![] }
    }
}

fn hash_of<T: std::hash::Hash>(t: T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn collect_images(items: &[Item], out: &mut Vec<(u32, u32, Vec<u8>)>) {
    for i in items {
        match i {
            Item::Image { px_w, px_h, rgb, .. } => out.push((*px_w, *px_h, rgb.clone())),
            Item::Clip { items, .. } => collect_images(items, out),
            _ => {}
        }
    }
}

fn textures_for(ctx: &egui::Context, fig: &Figure, tag: &str) -> Vec<TextureHandle> {
    let mut imgs = Vec::new();
    collect_images(&fig.items, &mut imgs);
    imgs.into_iter().enumerate().map(|(k, (w, h, rgb))| ctx.load_texture(format!("{tag}{k}"), egui::ColorImage::from_rgb([w as usize, h as usize], &rgb), TextureOptions::NEAREST)).collect()
}

impl App {
    // ---- editing operations (also used by the tests) ------------------------------------------------------------

    /// A sample was dropped at a page point: onto a tile it becomes an overlay layer, elsewhere a new tile.
    pub fn editor_drop(&mut self, sample: &str, at: (f32, f32)) {
        let template: Option<Tile> = match self.lay.ed.selected {
            Some(Sel::Tile(id)) => self.lay.ed.doc.tile(id).cloned(),
            _ => self.lay.ed.doc.tiles.last().cloned(),
        };
        let doc = &mut self.lay.ed.doc;
        if let Some(id) = doc.tile_at(at) {
            let added = doc.tile_mut(id).map_or(false, |t| t.add_layer(sample, LayerRole::Iterator));
            self.lay.ed.selected = Some(Sel::Tile(id));
            self.status = if added { format!("Added {} to the tile as an overlay.", sample.trim_end_matches(".fcs")) } else { format!("{} is already on that tile.", sample.trim_end_matches(".fcs")) };
            return;
        }
        let id = doc.add_tile(at, 240.0, 210.0);
        if let Some(t) = doc.tile_mut(id) {
            if let Some(src) = template {
                // a new tile starts like the last one: same population, axes and style
                t.population = src.population;
                t.x_chan = src.x_chan;
                t.y_chan = src.y_chan;
                t.kind = src.kind;
                t.x_scale = src.x_scale;
                t.y_scale = src.y_scale;
                t.title = src.title;
            }
            t.add_layer(sample, LayerRole::Iterator);
        }
        self.lay.ed.selected = Some(Sel::Tile(id));
        self.status = format!("New tile for {}. Drag another sample onto it to overlay.", sample.trim_end_matches(".fcs"));
    }

    pub fn editor_delete_selected(&mut self) {
        match self.lay.ed.selected.take() {
            Some(Sel::Tile(id)) => {
                self.lay.ed.doc.remove_tile(id);
            }
            Some(Sel::Text(id)) => {
                self.lay.ed.doc.remove_text(id);
            }
            None => {}
        }
    }

    /// Render every page of the batch and keep them for preview and saving.
    pub fn editor_run_batch(&mut self, ctx: Option<&egui::Context>) -> Result<usize, String> {
        let (pages, notes) = render_batch(&mut self.session, &self.lay.ed.doc)?;
        let n = pages.len();
        self.lay.ed.batch_textures = match ctx {
            Some(c) => pages.iter().enumerate().map(|(i, p)| textures_for(c, p, &format!("batch{i}_"))).collect(),
            None => vec![],
        };
        self.lay.ed.batch = pages;
        self.lay.ed.notes = notes;
        self.status = format!("Batch: {n} page(s){}.", if self.lay.ed.notes.is_empty() { String::new() } else { format!(", {} problem(s) listed", self.lay.ed.notes.len()) });
        Ok(n)
    }

    pub fn write_batch_pdf(&self, path: &std::path::Path) -> Result<(), String> {
        if self.lay.ed.batch.is_empty() {
            return Err("run the batch first".into());
        }
        std::fs::write(path, to_pdf(&self.lay.ed.batch, &self.lay.ed.doc.name)).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// A ready-made layout for demonstrations and the command line: a histogram of the first fluorescence channel
    /// with the first sample iterating over an "unstained" control, a two-channel plot, and a title with tokens.
    pub fn demo_layout(&mut self, keyword: Option<String>) {
        self.lay.tab = super::layout_view::LayTab::Editor;
        self.set_module(super::Module::Layout);
        let names: Vec<String> = self.session.samples().iter().map(|s| s.name.clone()).collect();
        let Some(first) = names.first().cloned() else { return };
        let control = names.iter().find(|n| n.to_lowercase().contains("unstained")).cloned();
        let chans = self.session.sample(0).channels.clone();
        let fluor: Vec<String> = chans.iter().filter(|c| !c.starts_with("FSC") && !c.starts_with("SSC") && c.as_str() != "Time" && !c.ends_with("-W") && !c.ends_with("-H") && !c.starts_with("Comp-") && !c.starts_with("Unmix-")).cloned().collect();
        let doc = &mut self.lay.ed.doc;
        *doc = LayoutDoc::new("Demo layout");
        let a = doc.add_tile((230.0, 270.0), 330.0, 300.0);
        if let Some(t) = doc.tile_mut(a) {
            t.x_chan = fluor.first().cloned().unwrap_or_default();
            t.kind = PlotKind::Histogram;
            t.add_layer(&first, LayerRole::Iterator);
            if let Some(c) = &control {
                t.add_layer(c, LayerRole::Control);
            }
        }
        let b = doc.add_tile((590.0, 270.0), 330.0, 300.0);
        if let Some(t) = doc.tile_mut(b) {
            t.x_chan = fluor.first().cloned().unwrap_or_default();
            t.y_chan = fluor.get(1).cloned().unwrap_or_default();
            t.add_layer(&first, LayerRole::Iterator);
        }
        doc.add_text((36.0, 26.0), "[File_Name] \u{2014} [Keyword]  (page [Page] of [Pages])");
        if let Some(t) = doc.texts.last_mut() {
            t.size = 15.0;
            t.bold = true;
        }
        if let Some(k) = keyword {
            doc.iterate = Iterate::Keyword(k);
        }
        self.lay.ed.selected = Some(Sel::Tile(a));
    }

    // ---- screen --------------------------------------------------------------------------------------------------

    pub(super) fn editor_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        egui::SidePanel::left("led_cfg").resizable(false).exact_width(330.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.editor_panel(ui, ctx));
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(10.0))).show_inside(ui, |ui| self.editor_canvas(ui, ctx));
    }

    fn editor_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(RichText::new("Layout editor").strong().size(16.0));
        theme::caption(ui, "Drag a sample onto the page to make a plot. Drop another sample on a plot to overlay it. Right-click a plot to set a layer as a control.");
        ui.horizontal(|ui| {
            if ui.button("+ Text").clicked() {
                let id = self.lay.ed.doc.add_text((30.0, 24.0), "[File_Name]");
                self.lay.ed.selected = Some(Sel::Text(id));
            }
            if ui.add_enabled(self.lay.ed.selected.is_some(), egui::Button::new("Delete")).on_hover_text("Delete key").clicked() {
                self.editor_delete_selected();
            }
        });
        theme::section(ui, "Samples (drag onto the page)");
        let mut rows = Vec::new();
        let names: Vec<String> = self.session.samples().iter().map(|s| s.name.clone()).collect();
        let labels = super::cross_view::short_all(&names);
        egui::ScrollArea::vertical().id_salt("led_samples").max_height(180.0).show(ui, |ui| {
            for (n, l) in names.iter().zip(&labels) {
                let r = ui.add(egui::Label::new(RichText::new(format!("⠿  {l}")).size(12.5)).sense(Sense::drag()));
                r.dnd_set_drag_payload(n.clone());
                rows.push((n.clone(), r.rect));
            }
        });
        self.lay.ed.sample_rows = rows;
        // a label that follows the pointer while dragging
        if egui::DragAndDrop::has_payload_of_type::<String>(ctx) {
            if let (Some(p), Some(n)) = (ctx.pointer_latest_pos(), egui::DragAndDrop::payload::<String>(ctx)) {
                ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("led_ghost"))).text(p + egui::vec2(12.0, 10.0), egui::Align2::LEFT_TOP, n.trim_end_matches(".fcs"), egui::FontId::proportional(12.0), Color32::WHITE);
            }
        }
        match self.lay.ed.selected {
            Some(Sel::Tile(id)) => self.tile_properties(ui, id),
            Some(Sel::Text(id)) => self.text_properties(ui, id),
            None => theme::caption(ui, "Select a plot or text on the page to edit it."),
        }
        theme::section(ui, "Page and batch");
        let doc = &mut self.lay.ed.doc;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut doc.a4, false, "Letter");
            ui.selectable_value(&mut doc.a4, true, "A4");
            ui.checkbox(&mut doc.landscape, "Landscape");
        });
        let by_kw = matches!(doc.iterate, Iterate::Keyword(_));
        ui.horizontal(|ui| {
            if ui.selectable_label(!by_kw, "One page per sample").clicked() {
                doc.iterate = Iterate::Sample;
            }
            if ui.selectable_label(by_kw, "One page per keyword value").clicked() && !by_kw {
                doc.iterate = Iterate::Keyword("$FIL".into());
            }
        });
        if let Iterate::Keyword(k) = &mut doc.iterate {
            let keys: Vec<String> = {
                let mut v: Vec<String> = self.session.samples().first().map(|s| s.keywords.keys().cloned().collect()).unwrap_or_default();
                v.sort();
                v
            };
            ui.horizontal(|ui| {
                ui.label("Keyword");
                ui.add(egui::TextEdit::singleline(k).desired_width(110.0));
                egui::ComboBox::from_id_salt("led_kw").width(24.0).selected_text("▾").show_ui(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for key in &keys {
                            if ui.selectable_label(key == k, key).clicked() {
                                *k = key.clone();
                            }
                        }
                    });
                });
            });
            theme::caption(ui, "Each page overlays all samples that share the keyword's value.");
        }
        let n_all = self.session.samples().len();
        ui.horizontal(|ui| {
            let every = self.lay.ed.doc.samples.is_empty();
            ui.label(if every { format!("Batch over all {n_all} samples") } else { format!("Batch over {} chosen samples", self.lay.ed.doc.samples.len()) });
            if ui.small_button("All").clicked() {
                self.lay.ed.doc.samples.clear();
            }
        });
        egui::ScrollArea::vertical().id_salt("led_batch_samples").max_height(100.0).show(ui, |ui| {
            for n in &names {
                let mut on = self.lay.ed.doc.samples.is_empty() || self.lay.ed.doc.samples.contains(n);
                let was = on;
                ui.checkbox(&mut on, n.trim_end_matches(".fcs"));
                if on != was {
                    if self.lay.ed.doc.samples.is_empty() {
                        self.lay.ed.doc.samples = names.clone();
                    }
                    if on {
                        self.lay.ed.doc.samples.push(n.clone());
                        self.lay.ed.doc.samples.dedup();
                    } else {
                        self.lay.ed.doc.samples.retain(|x| x != n);
                    }
                }
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("Run batch").strong()).fill(theme::ACCENT_DIM)).clicked() {
                if let Err(e) = self.editor_run_batch(Some(ctx)) {
                    self.status = e;
                } else {
                    self.lay.ed.preview = Some(0);
                }
            }
            let have = !self.lay.ed.batch.is_empty();
            if ui.add_enabled(have, egui::Button::new("Save PDF…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name(format!("{}.pdf", self.lay.ed.doc.name)).save_file() {
                    self.status = match self.write_batch_pdf(&p) {
                        Ok(()) => format!("Saved {} page(s) to {}.", self.lay.ed.batch.len(), p.display()),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
            }
        });
        let (pages, cur) = (std::mem::take(&mut self.lay.ed.batch), self.lay.ed.preview.unwrap_or(0));
        let stem = self.lay.ed.doc.name.clone();
        self.page_image_row(ui, &pages, cur, &stem);
        self.lay.ed.batch = pages;
        if !self.lay.ed.notes.is_empty() {
            ui.label(RichText::new(format!("{} problem(s):", self.lay.ed.notes.len())).color(theme::WARN).size(12.0));
            egui::ScrollArea::vertical().id_salt("led_notes").max_height(110.0).show(ui, |ui| {
                for n in &self.lay.ed.notes {
                    theme::caption(ui, n.clone());
                }
            });
        }
    }

    fn text_properties(&mut self, ui: &mut egui::Ui, id: u32) {
        theme::section(ui, "Text");
        if let Some(t) = self.lay.ed.doc.texts.iter_mut().find(|t| t.id == id) {
            ui.add(egui::TextEdit::multiline(&mut t.text).desired_rows(2).desired_width(290.0));
            ui.horizontal(|ui| {
                ui.label("Size");
                ui.add(egui::DragValue::new(&mut t.size).range(6.0..=48.0));
                ui.checkbox(&mut t.bold, "Bold");
                ui.checkbox(&mut t.italic, "Italic");
            });
            theme::caption(ui, "Tokens: [File_Name] [Keyword] [Group_Size] [Population] [Page] [Pages]");
        }
    }

    fn tile_properties(&mut self, ui: &mut egui::Ui, id: u32) {
        let Some(tile) = self.lay.ed.doc.tile(id).cloned() else { return };
        let first = tile.layers.iter().find_map(|l| self.session.samples().iter().position(|s| s.name == l.sample)).unwrap_or(0);
        if self.session.samples().is_empty() {
            return;
        }
        let chans = self.session.sample(first).channels.clone();
        let paths: Vec<Vec<String>> = self.session.tree(first).self_and_descendants(GateTree::ROOT).unwrap_or_default().into_iter().skip(1).filter_map(|n| self.session.tree(first).path(n).ok()).collect();
        theme::section(ui, "Plot");
        let mut t = tile.clone();
        egui::ComboBox::from_id_salt("led_pop").width(280.0).selected_text(format!("Population: {}", pop_label(&t.population))).show_ui(ui, |ui| {
            if ui.selectable_label(t.population.is_empty(), "All Events").clicked() {
                t.population.clear();
            }
            for p in paths {
                if ui.selectable_label(t.population == p, p.join(" ▸ ")).clicked() {
                    t.population = p;
                }
            }
        });
        let shown = |c: &str, fallback: &str| if c.is_empty() { fallback.to_string() } else { c.to_string() };
        for (id_s, label, which) in [("led_x", "X", 0), ("led_y", "Y", 1)] {
            ui.horizontal(|ui| {
                ui.label(label);
                let cur = if which == 0 { shown(&t.x_chan, "FSC-A") } else { shown(&t.y_chan, "SSC-A") };
                egui::ComboBox::from_id_salt(id_s).width(150.0).selected_text(cur.clone()).show_ui(ui, |ui| {
                    for c in &chans {
                        if ui.selectable_label(*c == cur, c.as_str()).clicked() {
                            if which == 0 {
                                t.x_chan = c.clone();
                                t.x_scale = None;
                            } else {
                                t.y_chan = c.clone();
                                t.y_scale = None;
                            }
                        }
                    }
                });
            });
        }
        egui::ComboBox::from_id_salt("led_kind").width(190.0).selected_text(format!("Plot type: {}", t.kind.label())).show_ui(ui, |ui| {
            for k in PlotKind::ALL {
                ui.selectable_value(&mut t.kind, k, k.label());
            }
        });
        egui::CollapsingHeader::new("Plot style").default_open(false).show(ui, |ui| {
            super::plot_style_ui::plot_style_controls(ui, t.kind, &mut t.style, "led_style");
        });
        let mut lock = t.view == ViewMode::Lock;
        if ui.checkbox(&mut lock, "Same view on every page").on_hover_text("Locks the axes to the union of everything the batch shows, so pages are comparable").changed() {
            t.view = if lock { ViewMode::Lock } else { ViewMode::Fixed(crate::density::Range { x0: 0.0, x1: 262_144.0, y0: 0.0, y1: 262_144.0 }) };
        }
        ui.horizontal(|ui| {
            ui.checkbox(&mut t.show_gates, "Gates");
            ui.checkbox(&mut t.show_fmo, "FMO");
            ui.checkbox(&mut t.legend, "Legend");
        });
        ui.label(RichText::new("Title").size(11.5).color(theme::MUTED));
        ui.add(egui::TextEdit::singleline(&mut t.title).desired_width(290.0));
        theme::section(ui, "Layers");
        let mut remove: Option<usize> = None;
        for (i, l) in t.layers.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let c = PALETTE[l.color % PALETTE.len()];
                ui.label(RichText::new("■").color(Color32::from_rgb(c[0], c[1], c[2])));
                let name = l.sample.trim_end_matches(".fcs").to_string();
                ui.label(if l.role == LayerRole::Control { RichText::new(name).italics() } else { RichText::new(name) });
                let control = l.role == LayerRole::Control;
                if ui.small_button(if control { "control" } else { "iterates" }).on_hover_text("A control stays the same on every page; an iterating layer is replaced by each page's sample").clicked() {
                    l.role = if control { LayerRole::Iterator } else { LayerRole::Control };
                }
                if ui.small_button("✕").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            t.layers.remove(i);
        }
        if t != tile {
            if let Some(slot) = self.lay.ed.doc.tile_mut(id) {
                *slot = t;
            }
        }
    }

    fn editor_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // a batch page being previewed replaces the editor view
        if let Some(i) = self.lay.ed.preview.filter(|i| *i < self.lay.ed.batch.len()) {
            ui.horizontal(|ui| {
                if ui.add_enabled(i > 0, egui::Button::new("◀")).clicked() {
                    self.lay.ed.preview = Some(i - 1);
                }
                ui.label(format!("Batch page {} of {}", i + 1, self.lay.ed.batch.len()));
                if ui.add_enabled(i + 1 < self.lay.ed.batch.len(), egui::Button::new("▶")).clicked() {
                    self.lay.ed.preview = Some(i + 1);
                }
                if ui.button("Back to editing").clicked() {
                    self.lay.ed.preview = None;
                }
            });
            let fig = self.lay.ed.batch[i].clone();
            let avail = ui.available_size();
            let scale = (avail.x / fig.width).min((avail.y - 6.0).max(100.0) / fig.height);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(fig.width * scale, fig.height * scale), Sense::hover());
            let tex = self.lay.ed.batch_textures.get(i).cloned().unwrap_or_default();
            paint_figure(&ui.painter_at(rect), rect, &fig, &tex);
            return;
        }
        // the editor view: the document as the first page would be drawn, refreshed when anything changes
        let key = hash_of((self.lay.ed.doc.to_json().write(), (0..self.session.samples().len()).map(|i| self.session.version(i)).sum::<u64>(), format!("{:?}", self.session.fmo_all())));
        if self.lay.ed.fig.as_ref().map_or(true, |(k, _)| *k != key) {
            let (fig, notes) = render_page(&mut self.session, &self.lay.ed.doc, None, &HashMap::new(), 1, 1);
            self.lay.ed.textures = textures_for(ctx, &fig, "editor");
            self.lay.ed.fig = Some((key, fig));
            if self.lay.ed.batch.is_empty() {
                self.lay.ed.notes = notes;
            }
        }
        let fig = self.lay.ed.fig.as_ref().map(|(_, f)| f.clone()).unwrap();
        let avail = ui.available_size();
        let scale = (avail.x / fig.width).min((avail.y - 6.0).max(100.0) / fig.height);
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(fig.width * scale, fig.height * scale), Sense::click_and_drag());
        self.lay.ed.canvas = Some((rect, scale));
        let tex = self.lay.ed.textures.clone();
        paint_figure(&ui.painter_at(rect), rect, &fig, &tex);
        let painter = ui.painter_at(rect);
        let to_page = |p: Pos2| ((p.x - rect.left()) / scale, (p.y - rect.top()) / scale);
        let to_screen = |x: f32, y: f32| Pos2::new(rect.left() + x * scale, rect.top() + y * scale);
        let doc_tiles: Vec<Tile> = self.lay.ed.doc.tiles.clone();
        // selection frame and resize handle
        if let Some(Sel::Tile(id)) = self.lay.ed.selected {
            if let Some(t) = doc_tiles.iter().find(|t| t.id == id) {
                let r = Rect::from_min_size(to_screen(t.x, t.y), egui::vec2(t.w * scale, t.h * scale));
                painter.rect_stroke(r, 0.0, Stroke::new(1.5_f32, theme::ACCENT));
                painter.rect_filled(Rect::from_center_size(r.right_bottom(), egui::vec2(9.0, 9.0)), 1.0, theme::ACCENT);
            }
        }
        if let Some(Sel::Text(id)) = self.lay.ed.selected {
            if let Some(t) = self.lay.ed.doc.texts.iter().find(|t| t.id == id) {
                let w = crate::figure::text_width(&t.text, t.size, t.bold).max(20.0);
                painter.rect_stroke(Rect::from_min_size(to_screen(t.x, t.y), egui::vec2(w * scale, t.size * 1.2 * scale)).expand(2.0), 0.0, Stroke::new(1.2_f32, theme::ACCENT));
            }
        }
        // dropping a sample: highlight the tile it would join, or the page it would create a tile on
        if resp.dnd_hover_payload::<String>().is_some() {
            if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                let at = to_page(p);
                match self.lay.ed.doc.tile_at(at).and_then(|id| doc_tiles.iter().find(|t| t.id == id)) {
                    Some(t) => painter.rect_stroke(Rect::from_min_size(to_screen(t.x, t.y), egui::vec2(t.w * scale, t.h * scale)), 0.0, Stroke::new(3.0_f32, Color32::from_rgb(40, 120, 255))),
                    None => painter.rect_stroke(rect.shrink(1.0), 0.0, Stroke::new(2.0_f32, Color32::from_rgb(40, 120, 255))),
                };
            }
        }
        if let Some(name) = resp.dnd_release_payload::<String>() {
            if let Some(p) = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos())) {
                self.editor_drop(&name, to_page(p));
            }
        }
        // select, move, resize
        if resp.drag_started() {
            if let Some(o) = ui.input(|i| i.pointer.press_origin()) {
                let at = to_page(o);
                let handle = match self.lay.ed.selected {
                    Some(Sel::Tile(id)) => doc_tiles.iter().find(|t| t.id == id).filter(|t| (to_screen(t.x + t.w, t.y + t.h) - o).length() <= 9.0).map(|_| Drag { sel: Sel::Tile(id), resize: true }),
                    _ => None,
                };
                self.lay.ed.drag = handle.or_else(|| self.hit(at).map(|s| Drag { sel: s, resize: false }));
                if let Some(d) = self.lay.ed.drag {
                    self.lay.ed.selected = Some(d.sel);
                    if let Sel::Tile(id) = d.sel {
                        self.lay.ed.doc.raise(id);
                    }
                }
            }
        }
        if let (true, Some(d)) = (resp.dragged(), self.lay.ed.drag) {
            let dl = resp.drag_delta() / scale;
            match (d.sel, d.resize) {
                (Sel::Tile(id), false) => self.lay.ed.doc.move_tile(id, dl.x, dl.y),
                (Sel::Tile(id), true) => self.lay.ed.doc.resize_tile(id, dl.x, dl.y),
                (Sel::Text(id), _) => {
                    if let Some(t) = self.lay.ed.doc.texts.iter_mut().find(|t| t.id == id) {
                        t.x += dl.x;
                        t.y += dl.y;
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.lay.ed.drag = None;
        }
        if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.lay.ed.selected = self.hit(to_page(p));
            }
        }
        if resp.secondary_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                self.lay.ed.ctx_tile = self.lay.ed.doc.tile_at(to_page(p));
                if let Some(id) = self.lay.ed.ctx_tile {
                    self.lay.ed.selected = Some(Sel::Tile(id));
                }
            }
        }
        let ctx_tile = self.lay.ed.ctx_tile;
        resp.context_menu(|ui| match ctx_tile.and_then(|id| self.lay.ed.doc.tile(id).cloned()) {
            Some(t) => {
                ui.label(RichText::new("Layers").strong());
                for (i, l) in t.layers.iter().enumerate() {
                    let control = l.role == LayerRole::Control;
                    let name = l.sample.trim_end_matches(".fcs");
                    if ui.button(if control { format!("Let {name} iterate") } else { format!("Set {name} as control") }).clicked() {
                        if let Some(tm) = self.lay.ed.doc.tile_mut(t.id) {
                            tm.set_control(i, !control);
                        }
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Bring to front").clicked() {
                    self.lay.ed.doc.raise(t.id);
                    ui.close_menu();
                }
                if ui.button("Delete plot").clicked() {
                    self.lay.ed.doc.remove_tile(t.id);
                    self.lay.ed.selected = None;
                    ui.close_menu();
                }
            }
            None => {
                ui.label(RichText::new("Drop a sample here to make a plot").color(theme::MUTED));
            }
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Delete)) && !ctx.wants_keyboard_input() {
            self.editor_delete_selected();
        }
        if self.lay.ed.doc.tiles.is_empty() && self.lay.ed.doc.texts.is_empty() {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Drag a sample from the list onto the page", egui::FontId::proportional(15.0), Color32::from_gray(150));
        }
        let _ = MIN_TILE;
    }

    /// What is under a page point: a tile, or a text box.
    fn hit(&self, at: (f32, f32)) -> Option<Sel> {
        if let Some(t) = self.lay.ed.doc.texts.iter().rev().find(|t| {
            let w = crate::figure::text_width(&t.text, t.size, t.bold).max(20.0);
            at.0 >= t.x - 2.0 && at.0 <= t.x + w + 2.0 && at.1 >= t.y - 2.0 && at.1 <= t.y + t.size * 1.2 + 2.0
        }) {
            return Some(Sel::Text(t.id));
        }
        self.lay.ed.doc.tile_at(at).map(Sel::Tile)
    }
}
