//! Layout screen: one plot per sample on shared axes, paginated, previewed, and saved as a multi-page PDF (or a
//! page as SVG). Also the single-plot figure behind the graph window's Export menu.

use super::{theme, App};
use crate::density::{PlotKind, PlotStyle};
use crate::figure::{to_pdf, to_svg, Figure, Item, Rgba, Text};
use crate::layout::{build_pages, collect, BatchRequest, LayoutSpec, PageSetup, View};
use crate::plot_figure::{plot_figure, PlotSpec};
use crate::session::default_scale;
use crate::stats_table::pop_label;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, TextureHandle, TextureOptions};
use sf_gating::GateTree;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LayTab {
    Grid,
    Editor,
}

/// What the single-plot export shows.
#[derive(Debug, Clone, Copy)]
pub struct FigOpts {
    pub gates: bool,
    pub fmo: bool,
    pub titles: bool,
    pub footer: bool,
}

pub struct State {
    pub tab: LayTab,
    pub ed: super::layout_editor::State,
    pub population: Vec<String>,
    pub x_chan: String,
    pub y_chan: String,
    pub kind: PlotKind,
    pub style: PlotStyle,
    pub sel: Vec<bool>,
    pub spec: LayoutSpec,
    pub a4: bool,
    pub landscape: bool,
    pub show_gates: bool,
    pub show_fmo: bool,
    pub pages: Vec<Figure>,
    pub page: usize,
    /// What the pages were built for, to say when they are out of date.
    pub built_for: Option<String>,
    /// Plots that could not be drawn (and why), from the last build.
    pub notes: Vec<String>,
    textures: Vec<Vec<TextureHandle>>,
}

impl Default for State {
    fn default() -> Self {
        State { tab: LayTab::Grid, ed: Default::default(), population: vec![], x_chan: String::new(), y_chan: String::new(), kind: PlotKind::Pseudocolor, style: PlotStyle::default(), sel: vec![], spec: LayoutSpec::default(), a4: false, landscape: true, show_gates: true, show_fmo: true, pages: vec![], page: 0, built_for: None, notes: vec![], textures: vec![] }
    }
}

fn color(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

/// Draw a figure into `rect` (scaled to fit), taking each image's texture in order.
pub(super) fn paint_figure(painter: &egui::Painter, rect: Rect, fig: &Figure, textures: &[TextureHandle]) {
    let scale = rect.width() / fig.width;
    painter.rect_filled(rect, 2.0, Color32::WHITE);
    let mut next = 0;
    paint_items(&painter.with_clip_rect(rect), rect.min, scale, &fig.items, textures, &mut next);
}

fn paint_items(painter: &egui::Painter, o: Pos2, s: f32, items: &[Item], textures: &[TextureHandle], next: &mut usize) {
    let pt = |x: f32, y: f32| Pos2::new(o.x + x * s, o.y + y * s);
    let stroke = |st: &crate::figure::Stroke| egui::Stroke::new((st.width * s).max(0.4), color(st.color));
    for item in items {
        match item {
            Item::Rect { x, y, w, h, fill, stroke: st } => {
                let r = Rect::from_min_size(pt(*x, *y), egui::vec2(w * s, h * s));
                if let Some(f) = fill {
                    painter.rect_filled(r, 0.0, color(*f));
                }
                if let Some(st) = st {
                    painter.rect_stroke(r, 0.0, stroke(st));
                }
            }
            Item::Line { a, b, stroke: st } => {
                let (pa, pb) = (pt(a.0, a.1), pt(b.0, b.1));
                match st.dash {
                    Some((on, off)) => painter.extend(egui::Shape::dashed_line(&[pa, pb], stroke(st), on * s, off * s)),
                    None => {
                        painter.line_segment([pa, pb], stroke(st));
                    }
                }
            }
            Item::Poly { pts, closed, fill, stroke: st } => {
                let p: Vec<Pos2> = pts.iter().map(|(x, y)| pt(*x, *y)).collect();
                if let (Some(f), true) = (fill, *closed) {
                    fill_under_curve(painter, &p, color(*f));
                }
                if let Some(st) = st {
                    painter.add(if *closed { egui::Shape::closed_line(p, stroke(st)) } else { egui::Shape::line(p, stroke(st)) });
                }
            }
            Item::Text(t) => paint_text(painter, o, s, t),
            Item::Image { x, y, w, h, .. } => {
                if let Some(tex) = textures.get(*next) {
                    painter.image(tex.id(), Rect::from_min_size(pt(*x, *y), egui::vec2(w * s, h * s)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
                }
                *next += 1;
            }
            Item::Clip { x, y, w, h, items } => {
                let clip = Rect::from_min_size(pt(*x, *y), egui::vec2(w * s, h * s)).intersect(painter.clip_rect());
                paint_items(&painter.with_clip_rect(clip), o, s, items, textures, next);
            }
        }
    }
}

/// Fill a polygon of the form `[baseline start, curve..., baseline end]` (a histogram) as trapezoid strips. egui
/// cannot fill a concave outline in one piece, and a histogram is concave.
fn fill_under_curve(painter: &egui::Painter, p: &[Pos2], fill: Color32) {
    if p.len() < 4 {
        return;
    }
    let base = p[0].y;
    let curve = &p[1..p.len() - 1];
    let mut mesh = egui::epaint::Mesh::default();
    for w in curve.windows(2) {
        let i = mesh.vertices.len() as u32;
        for q in [Pos2::new(w[0].x, base), w[0], w[1], Pos2::new(w[1].x, base)] {
            mesh.colored_vertex(q, fill);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
}

fn paint_text(painter: &egui::Painter, o: Pos2, s: f32, t: &Text) {
    let at = Pos2::new(o.x + t.x * s, o.y + t.y * s);
    let font = FontId::proportional((t.size * s).max(3.0));
    if t.vertical {
        let galley = painter.layout_no_wrap(t.text.clone(), font, color(t.color));
        let (w, h) = (galley.size().x, galley.size().y);
        let along = match t.align {
            crate::figure::Align::Left => 0.0,
            crate::figure::Align::Center => w / 2.0,
            crate::figure::Align::Right => w,
        };
        painter.add(egui::epaint::TextShape::new(Pos2::new(at.x - h / 2.0, at.y + along), galley, color(t.color)).with_angle(-std::f32::consts::FRAC_PI_2));
        return;
    }
    let h = match t.align {
        crate::figure::Align::Left => egui::Align::Min,
        crate::figure::Align::Center => egui::Align::Center,
        crate::figure::Align::Right => egui::Align::Max,
    };
    let v = match t.valign {
        crate::figure::VAlign::Top => egui::Align::Min,
        crate::figure::VAlign::Middle => egui::Align::Center,
        crate::figure::VAlign::Bottom => egui::Align::Max,
    };
    painter.text(at, Align2([h, v]), &t.text, font, color(t.color));
}

fn image_count(items: &[Item]) -> usize {
    items.iter().map(|i| match i {
        Item::Image { .. } => 1,
        Item::Clip { items, .. } => image_count(items),
        _ => 0,
    }).sum()
}

fn collect_images<'a>(items: &'a [Item], out: &mut Vec<&'a Item>) {
    for i in items {
        match i {
            Item::Image { .. } => out.push(i),
            Item::Clip { items, .. } => collect_images(items, out),
            _ => {}
        }
    }
}

impl App {
    fn lay_sync(&mut self) {
        let n = self.session.samples().len();
        self.lay.sel.resize(n, true);
        if n == 0 {
            return;
        }
        let chans = self.session.sample(0).channels.clone();
        if !chans.contains(&self.lay.x_chan) {
            self.lay.x_chan = chans.iter().find(|c| c.eq_ignore_ascii_case("FSC-A")).or(chans.first()).cloned().unwrap_or_default();
        }
        if !chans.contains(&self.lay.y_chan) {
            self.lay.y_chan = chans.iter().find(|c| c.eq_ignore_ascii_case("SSC-A")).or(chans.get(1)).or(chans.first()).cloned().unwrap_or_default();
        }
    }

    fn lay_request(&self) -> BatchRequest {
        let s0 = self.session.sample(0);
        BatchRequest {
            population: self.lay.population.clone(),
            x_chan: self.lay.x_chan.clone(),
            y_chan: self.lay.y_chan.clone(),
            x_scale: default_scale(&self.lay.x_chan, s0.range_of(&self.lay.x_chan)),
            y_scale: default_scale(&self.lay.y_chan, s0.range_of(&self.lay.y_chan)),
            kind: self.lay.kind,
            style: self.lay.style.clone(),
            samples: (0..self.session.samples().len()).filter(|&i| self.lay.sel.get(i).copied().unwrap_or(false)).collect(),
            view: View::FitAll,
            show_gates: self.lay.show_gates,
            show_fmo: self.lay.show_fmo,
        }
    }

    /// Build the pages for the current settings.
    pub fn build_layout(&mut self) -> Result<usize, String> {
        self.lay_sync();
        if self.session.samples().is_empty() {
            return Err("load samples first".into());
        }
        self.lay.spec.page = if self.lay.a4 { PageSetup::a4(self.lay.landscape) } else { PageSetup::letter(self.lay.landscape) };
        let req = self.lay_request();
        let (common, cells) = collect(&mut self.session, &req)?;
        self.lay.notes = cells.iter().filter(|c| c.points.is_none()).map(|c| format!("{}: {}", c.sample, c.note.clone().unwrap_or_else(|| "no data".into()))).collect();
        let pages = build_pages(&self.lay.spec, &common, &cells)?;
        self.lay.built_for = Some(format!("{} · {} × {} · {} samples", pop_label(&self.lay.population), req.x_chan, req.y_chan, cells.len()));
        self.lay.textures.clear();
        self.lay.page = self.lay.page.min(pages.len() - 1);
        let n = pages.len();
        self.lay.pages = pages;
        self.status = format!("Layout: {} plot(s) on {n} page(s){}.", cells.len(), if self.lay.notes.is_empty() { String::new() } else { format!(", {} could not be drawn", self.lay.notes.len()) });
        Ok(n)
    }

    /// Write the built pages as one PDF.
    pub fn write_layout_pdf(&self, path: &std::path::Path) -> Result<(), String> {
        if self.lay.pages.is_empty() {
            return Err("build the layout first".into());
        }
        std::fs::write(path, to_pdf(&self.lay.pages, &self.lay.spec.title)).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Write the page being previewed as an SVG.
    pub fn write_layout_svg(&self, path: &std::path::Path) -> Result<(), String> {
        let page = self.lay.pages.get(self.lay.page).ok_or("build the layout first")?;
        std::fs::write(path, to_svg(page)).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The window's plot as a figure: its population, axes, scales, current view, gates and FMO lines.
    pub fn window_figure(&mut self, id: u64, width: f32, height: f32) -> Result<Figure, String> {
        let style = self.ws.graphs.get(&id).map(|g| g.plot.clone()).unwrap_or_default();
        let fmo = self.ws.graphs.get(&id).map_or(true, |g| g.style.fmo);
        self.window_figure_with(id, width, height, &FigOpts { gates: true, fmo, titles: true, footer: true }, style)
    }

    /// The same, with the export editor's choices: what to show and which plot style to use.
    pub fn window_figure_with(&mut self, id: u64, width: f32, height: f32, opts: &FigOpts, style: PlotStyle) -> Result<Figure, String> {
        let w = self.session.window(id).ok_or("no such window")?.clone();
        let range = self.ws.graphs.get(&id).and_then(|g| g.range).filter(|r| r.is_valid()).ok_or("this plot has no view to export yet")?;
        let (show_gates, show_fmo) = (opts.gates, opts.fmo);
        let req = BatchRequest { population: self.session.tree(w.sample).path(w.node).unwrap_or_default(), x_chan: w.x_chan.clone(), y_chan: w.y_chan.clone(), x_scale: w.x_scale.clone(), y_scale: w.y_scale.clone(), kind: w.kind, style: style.clone(), samples: vec![w.sample], view: View::Fixed(range), show_gates, show_fmo };
        let (common, cells) = collect(&mut self.session, &req)?;
        let cell = &cells[0];
        let pts = cell.points.as_ref().ok_or_else(|| cell.note.clone().unwrap_or_else(|| "no data".into()))?;
        Ok(plot_figure(&PlotSpec {
            width,
            height,
            captions: if opts.titles { vec![cell.sample.trim_end_matches(".fcs").to_string(), pop_label(&req.population)] } else { vec![] },
            points: pts,
            layers: vec![],
            legend: false,
            style: common.style.clone(),
            range: common.range,
            kind: common.kind,
            x_chan: common.x_chan.clone(),
            y_chan: common.y_chan.clone(),
            x_scale: common.x_scale.clone(),
            y_scale: common.y_scale.clone(),
            gates: cell.gates.clone(),
            fmo: common.fmo.clone(),
            footer: if opts.footer { Some(format!("{} events · {:.1}% of parent", cell.events, cell.parent_pct)) } else { None },
        }))
    }

    fn lay_textures(&mut self, ctx: &egui::Context) {
        if self.lay.textures.len() == self.lay.pages.len() {
            return;
        }
        self.lay.textures = self
            .lay
            .pages
            .iter()
            .enumerate()
            .map(|(pi, p)| {
                let mut imgs = Vec::new();
                collect_images(&p.items, &mut imgs);
                imgs.iter()
                    .enumerate()
                    .filter_map(|(k, it)| match it {
                        Item::Image { px_w, px_h, rgb, .. } => Some(ctx.load_texture(format!("layout{pi}_{k}"), egui::ColorImage::from_rgb([*px_w as usize, *px_h as usize], rgb), TextureOptions::NEAREST)),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        debug_assert!(self.lay.pages.iter().zip(&self.lay.textures).all(|(p, t)| image_count(&p.items) == t.len()));
    }

    pub(super) fn layout_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.session.samples().is_empty() {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Load samples first.").color(theme::MUTED)));
            return;
        }
        self.lay_sync();
        egui::TopBottomPanel::top("lay_tabs").frame(egui::Frame::none().fill(theme::PANEL).inner_margin(egui::Margin::symmetric(12.0, 6.0))).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                for (t, l) in [(LayTab::Grid, "Grid of plots"), (LayTab::Editor, "Layout editor")] {
                    if theme::toggle(ui, self.lay.tab == t, l).clicked() {
                        self.lay.tab = t;
                    }
                }
            });
        });
        if self.lay.tab == LayTab::Editor {
            self.editor_screen(ui, ctx);
            return;
        }
        egui::SidePanel::left("lay_cfg").resizable(false).exact_width(330.0).frame(egui::Frame::none().fill(theme::PANEL_2).inner_margin(egui::Margin::same(12.0))).show_inside(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.layout_panel(ui));
        });
        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::same(14.0))).show_inside(ui, |ui| {
            if self.lay.pages.is_empty() {
                ui.label(RichText::new("Choose the population, axes and samples, then press Build layout.").color(theme::MUTED));
                return;
            }
            self.lay_textures(ctx);
            ui.horizontal(|ui| {
                if ui.add_enabled(self.lay.page > 0, egui::Button::new("◀")).clicked() {
                    self.lay.page -= 1;
                }
                ui.label(format!("Page {} of {}", self.lay.page + 1, self.lay.pages.len()));
                if ui.add_enabled(self.lay.page + 1 < self.lay.pages.len(), egui::Button::new("▶")).clicked() {
                    self.lay.page += 1;
                }
                if let Some(b) = &self.lay.built_for {
                    theme::caption(ui, b.clone());
                }
            });
            ui.add_space(6.0);
            let page = &self.lay.pages[self.lay.page];
            let avail = ui.available_size();
            let scale = (avail.x / page.width).min((avail.y - 8.0).max(100.0) / page.height);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(page.width * scale, page.height * scale), Sense::hover());
            let tex = self.lay.textures.get(self.lay.page).cloned().unwrap_or_default();
            paint_figure(&ui.painter_at(rect), rect, &self.lay.pages[self.lay.page], &tex);
        });
    }

    fn layout_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Layout").strong().size(16.0));
        theme::caption(ui, "One plot per sample, the same population on the same axes with one shared view, laid out on pages and saved as a PDF.");
        ui.add_space(4.0);
        if ui.add(egui::Button::new(RichText::new("Build layout").strong()).fill(theme::ACCENT_DIM).min_size(egui::vec2(290.0, 32.0))).clicked() {
            if let Err(e) = self.build_layout() {
                self.status = e;
            }
        }
        let have = !self.lay.pages.is_empty();
        ui.horizontal(|ui| {
            if ui.add_enabled(have, egui::Button::new("Save PDF…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("PDF", &["pdf"]).set_file_name("layout.pdf").save_file() {
                    self.status = match self.write_layout_pdf(&p) {
                        Ok(()) => format!("Saved {} page(s) to {}.", self.lay.pages.len(), p.display()),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
            }
            if ui.add_enabled(have, egui::Button::new("This page as SVG…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("SVG", &["svg"]).set_file_name("page.svg").save_file() {
                    self.status = match self.write_layout_svg(&p) {
                        Ok(()) => format!("Saved page {} to {}.", self.lay.page + 1, p.display()),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
            }
        });
        let (pages, cur) = (std::mem::take(&mut self.lay.pages), self.lay.page);
        self.page_image_row(ui, &pages, cur, "layout");
        self.lay.pages = pages;
        if !self.lay.notes.is_empty() {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{} plot(s) could not be drawn:", self.lay.notes.len())).color(theme::WARN).size(12.0));
            for n in &self.lay.notes {
                theme::caption(ui, n.clone());
            }
        }
        ui.add_space(4.0);
        theme::section(ui, "Plot");
        let paths: Vec<Vec<String>> = self.session.tree(0).self_and_descendants(GateTree::ROOT).unwrap_or_default().into_iter().skip(1).filter_map(|n| self.session.tree(0).path(n).ok()).collect();
        let cur = pop_label(&self.lay.population);
        egui::ComboBox::from_id_salt("lay_pop").width(280.0).selected_text(format!("Population: {cur}")).show_ui(ui, |ui| {
            if ui.selectable_label(self.lay.population.is_empty(), "All Events").clicked() {
                self.lay.population.clear();
            }
            for p in paths {
                if ui.selectable_label(self.lay.population == p, p.join(" ▸ ")).clicked() {
                    self.lay.population = p;
                }
            }
        });
        let chans = self.session.sample(0).channels.clone();
        for (id, label, which) in [("lay_x", "X", 0), ("lay_y", "Y", 1)] {
            ui.horizontal(|ui| {
                ui.label(label);
                let cur = if which == 0 { self.lay.x_chan.clone() } else { self.lay.y_chan.clone() };
                egui::ComboBox::from_id_salt(id).width(130.0).selected_text(cur.clone()).show_ui(ui, |ui| {
                    for c in &chans {
                        if ui.selectable_label(*c == cur, c.as_str()).clicked() {
                            if which == 0 {
                                self.lay.x_chan = c.clone();
                            } else {
                                self.lay.y_chan = c.clone();
                            }
                        }
                    }
                });
            });
        }
        egui::ComboBox::from_id_salt("lay_kind").width(190.0).selected_text(format!("Plot type: {}", self.lay.kind.label())).show_ui(ui, |ui| {
            for k in PlotKind::ALL {
                ui.selectable_value(&mut self.lay.kind, k, k.label());
            }
        });
        egui::CollapsingHeader::new("Plot style").default_open(false).show(ui, |ui| {
            super::plot_style_ui::plot_style_controls(ui, self.lay.kind, &mut self.lay.style, "lay_style");
        });
        ui.checkbox(&mut self.lay.show_gates, "Draw the population's gates, with their %");
        ui.checkbox(&mut self.lay.show_fmo, "Draw FMO reference lines");
        theme::section(ui, "Page");
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.lay.a4, false, "Letter");
            ui.selectable_value(&mut self.lay.a4, true, "A4");
            ui.checkbox(&mut self.lay.landscape, "Landscape");
        });
        ui.horizontal(|ui| {
            ui.label("Columns");
            ui.add(egui::DragValue::new(&mut self.lay.spec.cols).range(1..=8));
            ui.label("Rows");
            ui.add(egui::DragValue::new(&mut self.lay.spec.rows).range(1..=8));
        });
        ui.checkbox(&mut self.lay.spec.order_down, "Fill down the columns first");
        theme::section(ui, "Text");
        for (label, field) in [("Title", 0), ("Under each plot", 1), ("Footer", 2)] {
            ui.label(RichText::new(label).size(11.5).color(theme::MUTED));
            let text = match field {
                0 => &mut self.lay.spec.title,
                1 => &mut self.lay.spec.caption,
                _ => &mut self.lay.spec.footer,
            };
            if field == 1 {
                ui.add(egui::TextEdit::multiline(text).desired_rows(2).desired_width(290.0));
            } else {
                ui.add(egui::TextEdit::singleline(text).desired_width(290.0));
            }
        }
        theme::caption(ui, "Tokens: [File_Name] [Population] [Events] [Percent_Parent] [Page] [Pages] [Count]");
        theme::section(ui, "Samples");
        ui.horizontal(|ui| {
            if ui.small_button("All").clicked() {
                self.lay.sel.iter_mut().for_each(|s| *s = true);
            }
            if ui.small_button("None").clicked() {
                self.lay.sel.iter_mut().for_each(|s| *s = false);
            }
        });
        let labels: Vec<String> = super::cross_view::short_all(&self.session.samples().iter().map(|s| s.name.clone()).collect::<Vec<_>>());
        egui::ScrollArea::vertical().id_salt("lay_samples").max_height(170.0).show(ui, |ui| {
            for (i, l) in labels.iter().enumerate() {
                ui.checkbox(&mut self.lay.sel[i], l.as_str());
            }
        });
    }
}
