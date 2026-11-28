//! Turn a [`Figure`] into pixels at any resolution (dots per inch), for PNG files, the clipboard and previews.
//!
//! A small scan-line rasteriser: filled polygons, strokes (with dashes), images, clipping and text. Output is
//! anti-aliased by drawing at twice the size and averaging, unless the picture is large enough that one sample per
//! pixel already looks smooth. Text uses the application's bundled sans-serif outlines placed at the Helvetica
//! advance widths the PDF and SVG use, so a PNG lines up with the vector exports.

use crate::figure::{origin, text_width, Figure, Item, Rgba, Stroke, Text};
use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};

/// Largest picture produced, in pixels. A 12 x 10 inch figure at 600 dpi is 43 million.
pub const MAX_PIXELS: u64 = 120_000_000;

pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) RGBA, row 0 at the top.
    pub rgba: Vec<u8>,
}

/// Pixel size of a figure at `dpi`.
pub fn pixel_size(fig: &Figure, dpi: f32) -> (u32, u32) {
    (((fig.width * dpi / 72.0).round() as u32).max(1), ((fig.height * dpi / 72.0).round() as u32).max(1))
}

pub fn rasterize(fig: &Figure, dpi: f32, transparent: bool) -> Result<Raster, String> {
    if !(dpi.is_finite() && dpi >= 10.0) {
        return Err("The resolution must be at least 10 dpi".into());
    }
    let (w, h) = pixel_size(fig, dpi);
    if w as u64 * h as u64 > MAX_PIXELS {
        return Err(format!("{w} x {h} pixels is too large (the limit is {} million); lower the size or the resolution", MAX_PIXELS / 1_000_000));
    }
    let ss: u32 = if (w as u64) * (h as u64) > 6_000_000 { 1 } else { 2 };
    let (sw, sh) = (w * ss, h * ss);
    let scale = dpi / 72.0 * ss as f32;
    let mut cv = Canvas { w: sw as i32, px: vec![0; (sw * sh * 4) as usize], scale, fonts: Fonts::load() };
    if !transparent {
        cv.px.chunks_exact_mut(4).for_each(|p| p.copy_from_slice(&[255, 255, 255, 255]));
    }
    let clip = (0, 0, sw as i32, sh as i32);
    cv.items(&fig.items, clip);
    // average each ss x ss block (premultiplied), then un-premultiply
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let n = (ss * ss) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..ss {
                for dx in 0..ss {
                    let i = (((y * ss + dy) * sw + x * ss + dx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += cv.px[i + c] as u32;
                    }
                }
            }
            let a = (acc[3] + n / 2) / n;
            let o = ((y * w + x) * 4) as usize;
            if a > 0 {
                for c in 0..3 {
                    rgba[o + c] = ((acc[c] as f32 / n as f32) * 255.0 / a as f32).round().min(255.0) as u8;
                }
            }
            rgba[o + 3] = a as u8;
        }
    }
    Ok(Raster { width: w, height: h, rgba })
}

/// Encode as PNG, recording the resolution so other programs print it at the intended size.
pub fn to_png(r: &Raster, dpi: f32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, r.width, r.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let ppm = (dpi / 0.0254).round() as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions { xppu: ppm, yppu: ppm, unit: png::Unit::Meter }));
        let mut w = enc.write_header().map_err(|e| e.to_string())?;
        w.write_image_data(&r.rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

struct Fonts {
    regular: Option<std::rc::Rc<Vec<u8>>>,
}

impl Fonts {
    fn load() -> Fonts {
        let defs = eframe::egui::FontDefinitions::default();
        Fonts { regular: defs.font_data.get("Ubuntu-Light").map(|d| std::rc::Rc::new(d.font.to_vec())) }
    }
}

struct Canvas {
    w: i32,
    /// Premultiplied RGBA.
    px: Vec<u8>,
    scale: f32,
    fonts: Fonts,
}

type Clip = (i32, i32, i32, i32);

impl Canvas {
    fn blend(&mut self, x: i32, y: i32, c: Rgba, cov: f32, clip: Clip) {
        if x < clip.0 || y < clip.1 || x >= clip.2 || y >= clip.3 {
            return;
        }
        let a = (c[3] as f32 / 255.0) * cov.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let i = ((y * self.w + x) * 4) as usize;
        let inv = 1.0 - a;
        for k in 0..3 {
            let src = c[k] as f32 * a;
            self.px[i + k] = (src + self.px[i + k] as f32 * inv).round().min(255.0) as u8;
        }
        self.px[i + 3] = (a * 255.0 + self.px[i + 3] as f32 * inv).round().min(255.0) as u8;
    }

    fn items(&mut self, items: &[Item], clip: Clip) {
        let s = self.scale;
        for it in items {
            match it {
                Item::Rect { x, y, w, h, fill, stroke } => {
                    let pts = [(*x, *y), (x + w, *y), (x + w, y + h), (*x, y + h)].map(|p| (p.0 * s, p.1 * s));
                    if let Some(f) = fill {
                        self.fill(&pts, *f, clip);
                    }
                    if let Some(st) = stroke {
                        let mut p = pts.to_vec();
                        p.push(pts[0]);
                        self.stroke_path(&p, st, clip);
                    }
                }
                Item::Line { a, b, stroke } => self.stroke_path(&[(a.0 * s, a.1 * s), (b.0 * s, b.1 * s)], stroke, clip),
                Item::Poly { pts, closed, fill, stroke } => {
                    let p: Vec<(f32, f32)> = pts.iter().map(|q| (q.0 * s, q.1 * s)).collect();
                    if let (Some(f), true) = (fill, *closed) {
                        self.fill(&p, *f, clip);
                    }
                    if let Some(st) = stroke {
                        let mut q = p.clone();
                        if *closed && !p.is_empty() {
                            q.push(p[0]);
                        }
                        self.stroke_path(&q, st, clip);
                    }
                }
                Item::Text(t) => self.text(t, clip),
                Item::Image { x, y, w, h, px_w, px_h, rgb } => {
                    let (x0, y0, x1, y1) = (x * s, y * s, (x + w) * s, (y + h) * s);
                    let (ix0, iy0) = ((x0.round() as i32).max(clip.0), (y0.round() as i32).max(clip.1));
                    let (ix1, iy1) = ((x1.round() as i32).min(clip.2), (y1.round() as i32).min(clip.3));
                    for py in iy0..iy1 {
                        let v = ((py as f32 + 0.5 - y0) / (y1 - y0) * *px_h as f32).floor().clamp(0.0, *px_h as f32 - 1.0) as usize;
                        for px in ix0..ix1 {
                            let u = ((px as f32 + 0.5 - x0) / (x1 - x0) * *px_w as f32).floor().clamp(0.0, *px_w as f32 - 1.0) as usize;
                            let k = (v * *px_w as usize + u) * 3;
                            if k + 2 < rgb.len() {
                                self.blend(px, py, [rgb[k], rgb[k + 1], rgb[k + 2], 255], 1.0, clip);
                            }
                        }
                    }
                }
                Item::Clip { x, y, w, h, items } => {
                    let c = (((x * s).round() as i32).max(clip.0), ((y * s).round() as i32).max(clip.1), (((x + w) * s).round() as i32).min(clip.2), (((y + h) * s).round() as i32).min(clip.3));
                    self.items(items, c);
                }
            }
        }
    }

    /// Even-odd scan-line fill, sampling at pixel centres.
    fn fill(&mut self, pts: &[(f32, f32)], c: Rgba, clip: Clip) {
        if pts.len() < 3 {
            return;
        }
        let (mut ymin, mut ymax) = (f32::MAX, f32::MIN);
        for p in pts {
            ymin = ymin.min(p.1);
            ymax = ymax.max(p.1);
        }
        let (y0, y1) = ((ymin.floor() as i32).max(clip.1), (ymax.ceil() as i32).min(clip.3));
        let mut xs: Vec<f32> = Vec::new();
        for y in y0..y1 {
            let yc = y as f32 + 0.5;
            xs.clear();
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
                if (a.1 <= yc) != (b.1 <= yc) {
                    xs.push(a.0 + (yc - a.1) / (b.1 - a.1) * (b.0 - a.0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            for pair in xs.chunks_exact(2) {
                let (xa, xb) = (((pair[0] - 0.5).ceil() as i32).max(clip.0), ((pair[1] - 0.5).ceil() as i32).min(clip.2));
                for x in xa..xb {
                    self.blend(x, y, c, 1.0, clip);
                }
            }
        }
    }

    fn stroke_path(&mut self, pts: &[(f32, f32)], st: &Stroke, clip: Clip) {
        let wpx = (st.width * self.scale).max(self.scale * 0.35);
        let half = wpx / 2.0;
        let mut dash_pos = 0.0f32;
        for seg in pts.windows(2) {
            let (a, b) = (seg[0], seg[1]);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            if len < 1e-6 {
                continue;
            }
            let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
            let spans: Vec<(f32, f32)> = match st.dash {
                None => vec![(0.0, len)],
                Some((on, off)) => {
                    let (on, off) = ((on * self.scale).max(1.0), (off * self.scale).max(1.0));
                    let period = on + off;
                    let mut v = Vec::new();
                    let mut t = 0.0;
                    while t < len {
                        let phase = (dash_pos + t) % period;
                        if phase < on {
                            let run = (on - phase).min(len - t);
                            v.push((t, t + run));
                            t += run;
                        } else {
                            t += period - phase;
                        }
                    }
                    dash_pos += len;
                    v
                }
            };
            for (t0, t1) in spans {
                let (p, q) = ((a.0 + ux * t0, a.1 + uy * t0), (a.0 + ux * t1, a.1 + uy * t1));
                let (nx, ny) = (-uy * half, ux * half);
                self.fill(&[(p.0 + nx, p.1 + ny), (q.0 + nx, q.1 + ny), (q.0 - nx, q.1 - ny), (p.0 - nx, p.1 - ny)], st.color, clip);
            }
        }
        // round joins on thick paths keep corners and curves unbroken
        if st.dash.is_none() && wpx > 2.0 && pts.len() > 2 {
            for p in &pts[1..pts.len() - 1] {
                let circle: Vec<(f32, f32)> = (0..8).map(|k| (p.0 + half * (k as f32 * std::f32::consts::TAU / 8.0).cos(), p.1 + half * (k as f32 * std::f32::consts::TAU / 8.0).sin())).collect();
                self.fill(&circle, st.color, clip);
            }
        }
    }

    fn text(&mut self, t: &Text, clip: Clip) {
        let Some(bytes) = self.fonts.regular.clone() else { return };
        let Ok(font) = FontRef::try_from_slice(&bytes) else { return };
        let em = t.size * self.scale;
        let upm = font.units_per_em().unwrap_or(1000.0);
        let scaled = font.as_scaled(PxScale::from(em * font.height_unscaled() / upm));
        let (bx, by) = origin(t);
        let (bx, by) = (bx * self.scale, by * self.scale);
        let mut pen = 0.0f32;
        let passes: &[f32] = if t.bold { &[0.0, 0.035] } else { &[0.0] };
        for ch in t.text.chars() {
            let adv = text_width(&ch.to_string(), t.size, t.bold) * self.scale;
            if ch != ' ' {
                // a character the font has no outline for would print as an empty box: draw what the PDF writer
                // substitutes for it (an arrow becomes '>') so every export format reads the same
                let ch = if scaled.glyph_id(ch).0 == 0 { crate::figure::ascii_fallback(ch).unwrap_or(ch) } else { ch };
                let glyph = scaled.glyph_id(ch).with_scale_and_position(scaled.scale(), point(0.0, 0.0));
                if let Some(og) = scaled.outline_glyph(glyph) {
                    // centre the outline in its Helvetica advance so spacing matches the vector exports
                    let b = og.px_bounds();
                    let shift = (adv - (b.max.x - b.min.x)) / 2.0 - b.min.x;
                    for extra in passes {
                        og.draw(|gx, gy, cov| {
                            let lx = pen + shift + b.min.x + gx as f32 + extra * em;
                            let ly = b.min.y + gy as f32;
                            let lx = if t.italic { lx - ly * 0.2 } else { lx };
                            let (fx, fy) = if t.vertical { (bx + ly, by - lx) } else { (bx + lx, by + ly) };
                            self.blend(fx.floor() as i32, fy.floor() as i32, t.color, cov, clip);
                        });
                    }
                }
            }
            pen += adv;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::figure::{Align, Figure, Item, Stroke, Text, VAlign};

    fn px(r: &Raster, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * r.width + x) * 4) as usize;
        [r.rgba[i], r.rgba[i + 1], r.rgba[i + 2], r.rgba[i + 3]]
    }

    #[test]
    fn size_follows_the_resolution_and_the_background_is_white_or_clear() {
        let f = Figure::new(144.0, 72.0); // 2 x 1 inch
        let r = rasterize(&f, 300.0, false).unwrap();
        assert_eq!((r.width, r.height), (600, 300));
        assert_eq!(px(&r, 10, 10), [255, 255, 255, 255]);
        assert_eq!(px(&rasterize(&f, 100.0, true).unwrap(), 5, 5), [0, 0, 0, 0]);
        assert!(rasterize(&f, 5.0, false).is_err());
        assert!(rasterize(&Figure::new(10_000.0, 10_000.0), 600.0, false).err().unwrap().contains("too large"));
    }

    #[test]
    fn fills_strokes_clips_images_and_alpha_land_where_the_figure_says() {
        let mut f = Figure::new(100.0, 100.0);
        f.push(Item::Rect { x: 10.0, y: 10.0, w: 30.0, h: 20.0, fill: Some([255, 0, 0, 255]), stroke: None });
        f.push(Item::Line { a: (0.0, 60.0), b: (100.0, 60.0), stroke: Stroke::new([0, 0, 255, 255], 4.0) });
        f.push(Item::Clip { x: 50.0, y: 10.0, w: 20.0, h: 20.0, items: vec![Item::Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0, fill: Some([0, 200, 0, 255]), stroke: None }] });
        f.push(Item::Rect { x: 10.0, y: 70.0, w: 20.0, h: 20.0, fill: Some([0, 0, 0, 128]), stroke: None });
        f.push(Item::Image { x: 60.0, y: 70.0, w: 20.0, h: 20.0, px_w: 2, px_h: 1, rgb: vec![255, 0, 255, 0, 255, 255] });
        let r = rasterize(&f, 72.0, false).unwrap();
        assert_eq!(px(&r, 20, 20), [255, 0, 0, 255]);
        assert_eq!(px(&r, 5, 20), [255, 255, 255, 255]);
        assert_eq!(px(&r, 50, 60), [0, 0, 255, 255], "the stroke");
        assert_eq!(px(&r, 50, 64), [255, 255, 255, 255], "and nothing beyond its width");
        assert_eq!(px(&r, 60, 20), [0, 200, 0, 255], "inside the clip");
        assert_eq!(px(&r, 80, 20), [255, 255, 255, 255], "outside the clip");
        assert!((px(&r, 20, 80)[0] as i32 - 127).abs() <= 2, "half-transparent black over white is mid grey");
        assert_eq!(px(&r, 62, 80), [255, 0, 255, 255], "image, left pixel");
        assert_eq!(px(&r, 78, 80), [0, 255, 255, 255], "image, right pixel");
    }

    #[test]
    fn edges_are_anti_aliased_and_dashes_have_gaps() {
        let mut f = Figure::new(40.0, 40.0);
        f.push(Item::Poly { pts: vec![(5.0, 5.0), (35.0, 5.0), (5.0, 35.0)], closed: true, fill: Some([0, 0, 0, 255]), stroke: None });
        f.push(Item::Line { a: (0.0, 38.0), b: (40.0, 38.0), stroke: Stroke::dashed([0, 0, 0, 255], 1.0, 4.0, 4.0) });
        let r = rasterize(&f, 144.0, false).unwrap();
        let edge: Vec<u8> = (0..r.width).map(|x| px(&r, x, 20)[0]).collect();
        assert!(edge.iter().any(|&v| v > 5 && v < 250), "a diagonal edge has partial pixels");
        let row: Vec<u8> = (0..r.width).map(|x| px(&r, x, 76)[0]).collect();
        assert!(row.iter().any(|&v| v < 60) && row.iter().any(|&v| v > 200), "dashes alternate");
    }

    fn ink(r: &Raster, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
        (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| px(r, x, y)[0] < 128).count()
    }

    #[test]
    fn text_is_drawn_aligned_bold_heavier_and_vertical_text_runs_up() {
        let mut f = Figure::new(200.0, 100.0);
        f.push(Item::Text(Text::new(100.0, 20.0, "CD3-A", 12.0).align(Align::Center, VAlign::Middle)));
        f.push(Item::Text(Text::new(10.0, 60.0, "Count", 12.0).align(Align::Left, VAlign::Bottom)));
        f.push(Item::Text(Text::new(10.0, 90.0, "Count", 12.0).align(Align::Left, VAlign::Bottom).bold()));
        f.push(Item::Text(Text::new(180.0, 50.0, "Count", 12.0).align(Align::Center, VAlign::Middle).vertical()));
        let r = rasterize(&f, 144.0, false).unwrap();
        let w = crate::figure::text_width("CD3-A", 12.0, false) * 2.0;
        let centre = 200.0;
        assert!(ink(&r, (centre - w / 2.0) as u32, 20, (centre + w / 2.0) as u32, 60) > 40, "centred text is where it should be");
        assert_eq!(ink(&r, 0, 0, (centre - w / 2.0 - 8.0) as u32, 70), 0, "and nothing to its left");
        let (plain, bold) = (ink(&r, 20, 90, 200, 125), ink(&r, 20, 150, 200, 190));
        assert!(plain > 30 && bold > plain, "bold is heavier: {plain} vs {bold}");
        // vertical text: tall, narrow ink column around x = 360
        let tall: Vec<u32> = (0..r.height).filter(|&y| (340..380).any(|x| px(&r, x, y)[0] < 128)).collect();
        let wide: Vec<u32> = (340..380).filter(|&x| (0..r.height).any(|y| px(&r, x, y)[0] < 128)).collect();
        assert!(!tall.is_empty() && tall.last().unwrap() - tall[0] > 40 && wide.last().unwrap() - wide[0] < 30, "vertical text is tall and narrow");
    }

    #[test]
    fn a_real_plot_rasterises_and_encodes_as_png_with_its_resolution() {
        let pts: Vec<(f64, f64)> = (0..3000).map(|i| (500.0 + ((i * 37) % 200) as f64, 300.0 + ((i * 11) % 100) as f64)).collect();
        let spec = crate::plot_figure::tests_support::demo_spec(&pts);
        let fig = crate::plot_figure::plot_figure(&spec);
        let r = rasterize(&fig, 200.0, false).unwrap();
        assert_eq!((r.width, r.height), pixel_size(&fig, 200.0));
        assert!(ink(&r, 0, 0, r.width, r.height) > 2000, "something was drawn");
        let png = to_png(&r, 200.0).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(&png));
        let mut rd = dec.read_info().unwrap();
        let info = rd.info();
        assert_eq!((info.width, info.height), (r.width, r.height));
        let dims = info.pixel_dims.unwrap();
        assert_eq!(dims.unit, png::Unit::Meter);
        assert!((dims.xppu as f32 / 39.3701 - 200.0).abs() < 1.0, "200 dpi recorded: {}", dims.xppu);
        let mut buf = vec![0; rd.output_buffer_size().unwrap()];
        rd.next_frame(&mut buf).unwrap();
        assert_eq!(buf, r.rgba);
        if let Ok(p) = std::env::var("SF_RASTER_DEBUG_PNG") {
            std::fs::write(p, &png).unwrap();
        }
    }
}
