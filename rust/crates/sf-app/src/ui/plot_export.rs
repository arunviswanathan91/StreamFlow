//! The plot export editor: size (in, cm, mm, pt or pixels), resolution, format, what appears on the plot, and a live
//! preview, before saving as PNG / PDF / SVG or copying the image to the clipboard.

use super::{theme, App};
use crate::density::PlotStyle;
use crate::figure::Figure;
use eframe::egui::{self, RichText, TextureHandle, TextureOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Png,
    Pdf,
    Svg,
}

impl Format {
    pub const ALL: [Format; 3] = [Format::Png, Format::Pdf, Format::Svg];
    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG image",
            Format::Pdf => "PDF (vector)",
            Format::Svg => "SVG (vector)",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Pdf => "pdf",
            Format::Svg => "svg",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Inch,
    Cm,
    Mm,
    Point,
    Pixel,
}

impl Unit {
    pub const ALL: [Unit; 5] = [Unit::Inch, Unit::Cm, Unit::Mm, Unit::Point, Unit::Pixel];
    pub fn label(self) -> &'static str {
        match self {
            Unit::Inch => "in",
            Unit::Cm => "cm",
            Unit::Mm => "mm",
            Unit::Point => "pt",
            Unit::Pixel => "px",
        }
    }
    /// Points per one of this unit (pixels depend on the resolution).
    fn points(self, dpi: f32) -> f32 {
        match self {
            Unit::Inch => 72.0,
            Unit::Cm => 72.0 / 2.54,
            Unit::Mm => 72.0 / 25.4,
            Unit::Point => 1.0,
            Unit::Pixel => 72.0 / dpi,
        }
    }
    pub fn from_points(self, pt: f32, dpi: f32) -> f32 {
        pt / self.points(dpi)
    }
    pub fn to_points(self, v: f32, dpi: f32) -> f32 {
        v * self.points(dpi)
    }
}

/// Everything the export editor lets you change.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub format: Format,
    pub unit: Unit,
    /// Size in points (1/72 inch), whatever unit is shown.
    pub width_pt: f32,
    pub height_pt: f32,
    pub lock_aspect: bool,
    pub dpi: f32,
    pub transparent: bool,
    pub gates: bool,
    pub fmo: bool,
    pub titles: bool,
    pub footer: bool,
    pub style: PlotStyle,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { format: Format::Png, unit: Unit::Inch, width_pt: 360.0, height_pt: 312.0, lock_aspect: true, dpi: 300.0, transparent: false, gates: true, fmo: true, titles: true, footer: true, style: PlotStyle::default() }
    }
}

pub const MIN_PT: f32 = 72.0;
pub const MAX_PT: f32 = 72.0 * 40.0;
pub const DPI_CHOICES: [f32; 5] = [72.0, 150.0, 300.0, 600.0, 1200.0];

/// Size presets: (name, width pt, height pt).
pub const PRESETS: [(&str, f32, f32); 5] = [("Journal, 1 column (88 mm)", 249.4, 216.0), ("Journal, 2 columns (180 mm)", 510.2, 360.0), ("Slide, quarter", 360.0, 270.0), ("Square, 3 in", 216.0, 216.0), ("Letter plot, 5 x 4.3 in", 360.0, 312.0)];

impl Settings {
    /// Output size in pixels (PNG) at the chosen resolution.
    pub fn pixels(&self) -> (u32, u32) {
        (((self.width_pt * self.dpi / 72.0).round() as u32).max(1), ((self.height_pt * self.dpi / 72.0).round() as u32).max(1))
    }

    /// Set the width, keeping the shape when the aspect is locked.
    pub fn set_width(&mut self, w: f32) {
        let w = w.clamp(MIN_PT, MAX_PT);
        if self.lock_aspect && self.width_pt > 0.0 {
            let k = w / self.width_pt;
            self.height_pt = (self.height_pt * k).clamp(MIN_PT, MAX_PT);
        }
        self.width_pt = w;
    }

    pub fn set_height(&mut self, h: f32) {
        let h = h.clamp(MIN_PT, MAX_PT);
        if self.lock_aspect && self.height_pt > 0.0 {
            let k = h / self.height_pt;
            self.width_pt = (self.width_pt * k).clamp(MIN_PT, MAX_PT);
        }
        self.height_pt = h;
    }

    /// The figure changes with these only (not with the file format or resolution).
    fn figure_key(&self) -> String {
        format!("{:?}|{:?}|{:?}|{}{}{}{}|{:?}", self.width_pt.to_bits(), self.height_pt.to_bits(), self.style, self.gates, self.fmo, self.titles, self.footer, ())
    }
}

#[derive(Default)]
pub struct State {
    /// The plot window being exported, while the dialog is open.
    pub open: Option<u64>,
    pub set: Settings,
    fig: Option<(String, Result<Figure, String>)>,
    preview: Option<(String, TextureHandle)>,
}

impl App {
    /// Open the export editor for a graph window, starting from the plot's own style.
    pub fn open_plot_export(&mut self, id: u64) {
        let style = self.ws.graphs.get(&id).map(|g| g.plot.clone()).unwrap_or_default();
        self.pexp.set.style = style;
        self.pexp.fig = None;
        self.pexp.open = Some(id);
    }

    /// The figure for the current settings (cached until something that changes it is edited).
    pub fn export_figure(&mut self, id: u64, set: &Settings) -> Result<Figure, String> {
        self.window_figure_with(id, set.width_pt, set.height_pt, &crate::ui::layout_view::FigOpts { gates: set.gates, fmo: set.fmo, titles: set.titles, footer: set.footer }, set.style.clone())
    }

    pub(crate) fn plot_export_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.pexp.open else { return };
        if self.session.window(id).is_none() {
            self.pexp.open = None;
            return;
        }
        let mut open = true;
        let mut set = self.pexp.set.clone();
        let (kind, mut act): (_, Option<&'static str>) = (self.session.window(id).unwrap().kind, None);
        egui::Window::new("Export plot").open(&mut open).collapsible(false).resizable(true).default_width(760.0).default_pos([180.0, 90.0]).show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(300.0);
                    ui.label(RichText::new("Format").strong().size(12.0).color(theme::MUTED));
                    ui.horizontal(|ui| {
                        for f in Format::ALL {
                            ui.selectable_value(&mut set.format, f, f.label());
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new("Size").strong().size(12.0).color(theme::MUTED));
                    ui.horizontal(|ui| {
                        let mut u = set.unit;
                        egui::ComboBox::from_id_salt("pe_unit").width(56.0).selected_text(u.label()).show_ui(ui, |ui| {
                            for x in Unit::ALL {
                                ui.selectable_value(&mut u, x, x.label());
                            }
                        });
                        set.unit = u;
                        let dpi = set.dpi;
                        let speed = if u == Unit::Pixel { 4.0 } else { 0.05 };
                        let mut w = u.from_points(set.width_pt, dpi);
                        let mut h = u.from_points(set.height_pt, dpi);
                        let (cw, ch) = (ui.add(egui::DragValue::new(&mut w).speed(speed).max_decimals(if u == Unit::Pixel { 0 } else { 2 }).prefix("W ")).changed(), ui.add(egui::DragValue::new(&mut h).speed(speed).max_decimals(if u == Unit::Pixel { 0 } else { 2 }).prefix("H ")).changed());
                        if cw {
                            set.set_width(u.to_points(w, dpi));
                        } else if ch {
                            set.set_height(u.to_points(h, dpi));
                        }
                        ui.checkbox(&mut set.lock_aspect, "Lock shape").on_hover_text("Keep the shape when one side changes");
                    });
                    egui::ComboBox::from_id_salt("pe_preset").width(240.0).selected_text("Size presets…").show_ui(ui, |ui| {
                        for (name, w, h) in PRESETS {
                            if ui.selectable_label(false, name).clicked() {
                                set.width_pt = w;
                                set.height_pt = h;
                            }
                        }
                    });
                    ui.add_space(6.0);
                    ui.label(RichText::new("Resolution").strong().size(12.0).color(theme::MUTED));
                    ui.add_enabled_ui(set.format == Format::Png, |ui| {
                        ui.horizontal(|ui| {
                            for d in DPI_CHOICES {
                                if ui.selectable_label((set.dpi - d).abs() < 0.5, format!("{d:.0}")).clicked() {
                                    set.dpi = d;
                                }
                            }
                            ui.add(egui::DragValue::new(&mut set.dpi).range(10.0..=1200.0).speed(1.0).suffix(" dpi"));
                        });
                    });
                    if set.format == Format::Png {
                        let (pw, ph) = set.pixels();
                        let big = pw as u64 * ph as u64 > crate::raster::MAX_PIXELS;
                        let text = format!("{pw} × {ph} px  ({:.1} × {:.1} in)", set.width_pt / 72.0, set.height_pt / 72.0);
                        ui.label(RichText::new(text).color(if big { theme::GATE_RED } else { theme::MUTED }).size(12.0));
                        ui.checkbox(&mut set.transparent, "Transparent background");
                    } else {
                        ui.label(RichText::new("Vector formats have no resolution; they scale without loss.").color(theme::MUTED).size(11.5));
                    }
                    ui.add_space(6.0);
                    ui.label(RichText::new("On the plot").strong().size(12.0).color(theme::MUTED));
                    ui.checkbox(&mut set.gates, "Gates");
                    ui.checkbox(&mut set.fmo, "FMO lines");
                    ui.checkbox(&mut set.titles, "Sample and population titles");
                    ui.checkbox(&mut set.footer, "Event count under the axis");
                    ui.checkbox(&mut set.style.grid, "Grid lines");
                    egui::CollapsingHeader::new("Plot style").default_open(false).show(ui, |ui| {
                        super::plot_style_ui::plot_style_controls(ui, kind, &mut set.style, "pe_style");
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.label(RichText::new("Preview").strong().size(12.0).color(theme::MUTED));
                    let key = format!("{}|{}", self.pexp.fig.as_ref().map_or("", |f| f.0.as_str()), set.transparent);
                    if let Some((_, tex)) = &self.pexp.preview {
                        let sz = tex.size_vec2();
                        let k = (430.0 / sz.x).min(380.0 / sz.y).min(1.0);
                        let (rect, _) = ui.allocate_exact_size(sz * k, egui::Sense::hover());
                        ui.painter().rect_filled(rect.expand(3.0), 2.0, if set.transparent { egui::Color32::from_gray(150) } else { egui::Color32::from_gray(90) });
                        ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                    }
                    let _ = key;
                    if let Some((_, Err(e))) = &self.pexp.fig {
                        ui.colored_label(theme::GATE_RED, e);
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Save…").strong()).clicked() {
                            act = Some("save");
                        }
                        if ui.add_enabled(set.format == Format::Png, egui::Button::new("Copy image")).on_hover_text("Copy the picture, at this size and resolution, to the clipboard to paste into a slide or document").clicked() {
                            act = Some("copy");
                        }
                        if ui.button("Close").clicked() {
                            act = Some("close");
                        }
                    });
                });
            });
        });
        // rebuild the figure and preview when something that changes them was edited
        let key = set.figure_key();
        if self.pexp.fig.as_ref().map_or(true, |f| f.0 != key) {
            let r = self.export_figure(id, &set);
            self.pexp.fig = Some((key.clone(), r));
            self.pexp.preview = None;
        }
        let pkey = format!("{key}|{}", set.transparent);
        if self.pexp.preview.as_ref().map_or(true, |p| p.0 != pkey) {
            if let Some((_, Ok(fig))) = &self.pexp.fig {
                // a preview at about screen resolution
                let dpi = (430.0 * 72.0 / fig.width).clamp(72.0, 200.0).max(96.0);
                if let Ok(r) = crate::raster::rasterize(fig, dpi, set.transparent) {
                    let img = egui::ColorImage::from_rgba_unmultiplied([r.width as usize, r.height as usize], &r.rgba);
                    self.pexp.preview = Some((pkey, ctx.load_texture("plot_export_preview", img, TextureOptions::LINEAR)));
                }
            }
        }
        self.pexp.set = set.clone();
        if !open {
            act = Some("close");
        }
        match act {
            Some("close") => self.pexp.open = None,
            Some("save") => self.export_save(id, &set),
            Some("copy") => self.export_copy(id, &set),
            _ => {}
        }
    }

    /// Render the plot with these settings and return the file bytes.
    pub fn export_bytes(&mut self, id: u64, set: &Settings, title: &str) -> Result<Vec<u8>, String> {
        let fig = self.export_figure(id, set)?;
        match set.format {
            Format::Pdf => Ok(crate::figure::to_pdf(&[fig], title)),
            Format::Svg => Ok(crate::figure::to_svg(&fig).into_bytes()),
            Format::Png => {
                let r = crate::raster::rasterize(&fig, set.dpi, set.transparent)?;
                crate::raster::to_png(&r, set.dpi)
            }
        }
    }

    fn export_save(&mut self, id: u64, set: &Settings) {
        let Some(w) = self.session.window(id).cloned() else { return };
        let stem = format!("{}_{}_{}", w.x_chan, w.y_chan, self.session.sample(w.sample).name.trim_end_matches(".fcs")).replace(['/', '\\', ':', ' '], "_");
        let Some(p) = rfd::FileDialog::new().add_filter(set.format.label(), &[set.format.ext()]).set_file_name(format!("{stem}.{}", set.format.ext())).save_file() else { return };
        self.status = match self.export_bytes(id, set, &stem).and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string())) {
            Ok(()) => match set.format {
                Format::Png => format!("Saved {} ({} x {} px at {:.0} dpi).", p.display(), set.pixels().0, set.pixels().1, set.dpi),
                _ => format!("Saved {} ({:.1} x {:.1} in).", p.display(), set.width_pt / 72.0, set.height_pt / 72.0),
            },
            Err(e) => format!("Could not export the plot: {e}"),
        };
    }

    fn export_copy(&mut self, id: u64, set: &Settings) {
        self.status = match self.export_figure(id, set).and_then(|f| crate::raster::rasterize(&f, set.dpi, set.transparent)).and_then(|r| super::copy_rgba_to_clipboard(r.width as usize, r.height as usize, &r.rgba)) {
            Ok(()) => format!("Copied the plot to the clipboard ({} x {} px).", set.pixels().0, set.pixels().1),
            Err(e) => format!("Could not copy the image: {e}"),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_convert_both_ways_and_pixels_follow_the_resolution() {
        for u in Unit::ALL {
            let v = u.from_points(u.to_points(3.0, 300.0), 300.0);
            assert!((v - 3.0).abs() < 1e-4, "{u:?}");
        }
        assert!((Unit::Inch.from_points(72.0, 300.0) - 1.0).abs() < 1e-6);
        assert!((Unit::Mm.to_points(25.4, 300.0) - 72.0).abs() < 1e-3);
        assert!((Unit::Pixel.to_points(300.0, 300.0) - 72.0).abs() < 1e-4, "300 px at 300 dpi is one inch");
        assert!((Unit::Pixel.to_points(600.0, 600.0) - 72.0).abs() < 1e-4);
    }

    #[test]
    fn a_locked_aspect_keeps_the_shape_and_sizes_are_limited() {
        let mut s = Settings { width_pt: 300.0, height_pt: 200.0, lock_aspect: true, ..Settings::default() };
        s.set_width(450.0);
        assert!((s.height_pt - 300.0).abs() < 1e-3);
        s.set_height(150.0);
        assert!((s.width_pt - 225.0).abs() < 1e-3);
        s.lock_aspect = false;
        s.set_width(600.0);
        assert_eq!(s.height_pt, 150.0);
        s.set_width(1.0);
        assert_eq!(s.width_pt, MIN_PT);
        s.set_width(1e9);
        assert_eq!(s.width_pt, MAX_PT);
        let s = Settings { width_pt: 216.0, height_pt: 144.0, dpi: 300.0, ..Settings::default() };
        assert_eq!(s.pixels(), (900, 600));
    }
}

impl State {
    /// Has the dialog rendered its preview? (for tests)
    #[allow(dead_code)]
    pub fn preview_ready(&self) -> bool {
        self.preview.is_some()
    }
}

impl App {
    /// PNG / clipboard buttons for built layout pages, at a chosen resolution (shared with the plot export editor).
    pub(super) fn page_image_row(&mut self, ui: &mut egui::Ui, pages: &[Figure], current: usize, stem: &str) {
        let have = !pages.is_empty() && current < pages.len();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Image").size(12.0).color(theme::MUTED));
            ui.add(egui::DragValue::new(&mut self.pexp.set.dpi).range(72.0..=600.0).speed(1.0).suffix(" dpi"));
            if ui.add_enabled(have, egui::Button::new("This page as PNG…")).clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{stem}.png")).save_file() {
                    self.status = match self.save_page_png(&pages[current], &p) {
                        Ok(m) => m,
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
            }
            if ui.add_enabled(pages.len() > 1, egui::Button::new("All pages as PNG…")).clicked() {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    let mut done = 0;
                    let mut err = None;
                    for (i, f) in pages.iter().enumerate() {
                        match self.save_page_png(f, &dir.join(format!("{stem}_{:03}.png", i + 1))) {
                            Ok(_) => done += 1,
                            Err(e) => {
                                err = Some(e);
                                break;
                            }
                        }
                    }
                    self.status = match err {
                        None => format!("Saved {done} PNG page(s) to {}.", dir.display()),
                        Some(e) => format!("Saved {done} page(s), then: {e}"),
                    };
                }
            }
            if ui.add_enabled(have, egui::Button::new("All pages as PowerPoint…")).on_hover_text("One slide per page, as pictures at the chosen resolution").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("PowerPoint", &["pptx"]).set_file_name(format!("{stem}.pptx")).save_file() {
                    self.status = match self.pages_pptx(pages, stem).and_then(|b| std::fs::write(&p, b).map_err(|e| e.to_string())) {
                        Ok(()) => format!("Saved {} slide(s) to {}.", pages.len(), p.display()),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
            }
            if ui.add_enabled(have, egui::Button::new("Copy page")).on_hover_text("Copy this page as a picture to paste into a slide or document").clicked() {
                self.status = match crate::raster::rasterize(&pages[current], self.pexp.set.dpi, false).and_then(|r| super::copy_rgba_to_clipboard(r.width as usize, r.height as usize, &r.rgba).map(|_| (r.width, r.height))) {
                    Ok((w, h)) => format!("Copied the page to the clipboard ({w} x {h} px)."),
                    Err(e) => format!("Could not copy the page: {e}"),
                };
            }
        });
    }

    /// Rasterise a figure at the chosen resolution and write it as a PNG that records that resolution.
    pub fn save_page_png(&self, fig: &Figure, path: &std::path::Path) -> Result<String, String> {
        let dpi = self.pexp.set.dpi;
        let r = crate::raster::rasterize(fig, dpi, false)?;
        let png = crate::raster::to_png(&r, dpi)?;
        std::fs::write(path, png).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!("Saved {} ({} x {} px at {dpi:.0} dpi).", path.display(), r.width, r.height))
    }
}
