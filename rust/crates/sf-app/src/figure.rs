//! A small vector "figure": rectangles, lines, polygons, text and images in points (1/72 inch), origin top-left,
//! y down. One figure is a plot, a layout page or a poster. Two writers turn it into a file: SVG and PDF.
//!
//! The PDF is written directly (no PDF library): the base-14 Helvetica fonts, vector paths, clipping, constant
//! alpha and Flate-compressed RGB images. Text widths come from the Adobe Helvetica metrics, so alignment is the
//! same in every viewer. Characters outside WinAnsi are written as `?` (or a close ASCII form) and never silently
//! dropped.

use crate::figure_font::{HELVETICA, HELVETICA_BOLD, HELVETICA_BOLD_EXTRA, HELVETICA_EXTRA};

pub type Rgba = [u8; 4];

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f32,
    /// `(on, off)` dash lengths.
    pub dash: Option<(f32, f32)>,
}

impl Stroke {
    pub fn new(color: Rgba, width: f32) -> Stroke {
        Stroke { color, width, dash: None }
    }
    pub fn dashed(color: Rgba, width: f32, on: f32, off: f32) -> Stroke {
        Stroke { color, width, dash: Some((on, off)) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub color: Rgba,
    pub align: Align,
    pub valign: VAlign,
    pub bold: bool,
    pub italic: bool,
    /// Read bottom-to-top (a y-axis label); `x` is then the centre line and `align` runs along the text.
    pub vertical: bool,
}

impl Text {
    pub fn new(x: f32, y: f32, text: impl Into<String>, size: f32) -> Text {
        Text { x, y, text: text.into(), size, color: BLACK, align: Align::Left, valign: VAlign::Bottom, bold: false, italic: false, vertical: false }
    }
    pub fn align(mut self, a: Align, v: VAlign) -> Text {
        self.align = a;
        self.valign = v;
        self
    }
    pub fn color(mut self, c: Rgba) -> Text {
        self.color = c;
        self
    }
    pub fn bold(mut self) -> Text {
        self.bold = true;
        self
    }
    pub fn italic(mut self) -> Text {
        self.italic = true;
        self
    }
    pub fn vertical(mut self) -> Text {
        self.vertical = true;
        self
    }
}

pub const BLACK: Rgba = [0, 0, 0, 255];
pub const WHITE: Rgba = [255, 255, 255, 255];

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Rect { x: f32, y: f32, w: f32, h: f32, fill: Option<Rgba>, stroke: Option<Stroke> },
    Line { a: (f32, f32), b: (f32, f32), stroke: Stroke },
    Poly { pts: Vec<(f32, f32)>, closed: bool, fill: Option<Rgba>, stroke: Option<Stroke> },
    Text(Text),
    /// `rgb` is `px_w * px_h * 3` bytes, row 0 at the top, drawn without smoothing.
    Image { x: f32, y: f32, w: f32, h: f32, px_w: u32, px_h: u32, rgb: Vec<u8> },
    /// Everything inside is clipped to the rectangle.
    Clip { x: f32, y: f32, w: f32, h: f32, items: Vec<Item> },
}

impl Item {
    fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Item::Rect { x, y, .. } | Item::Image { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Item::Line { a, b, .. } => {
                a.0 += dx;
                a.1 += dy;
                b.0 += dx;
                b.1 += dy;
            }
            Item::Poly { pts, .. } => pts.iter_mut().for_each(|p| {
                p.0 += dx;
                p.1 += dy;
            }),
            Item::Text(t) => {
                t.x += dx;
                t.y += dy;
            }
            Item::Clip { x, y, items, .. } => {
                *x += dx;
                *y += dy;
                items.iter_mut().for_each(|i| i.translate(dx, dy));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
}

impl Figure {
    pub fn new(width: f32, height: f32) -> Figure {
        Figure { width, height, items: Vec::new() }
    }

    pub fn push(&mut self, item: Item) {
        self.items.push(item);
    }

    /// Put another figure on this one with its top-left corner at `(x, y)`.
    pub fn place(&mut self, mut sub: Figure, x: f32, y: f32) {
        sub.items.iter_mut().for_each(|i| i.translate(x, y));
        self.items.extend(sub.items);
    }

    /// Number of items, counting those inside clips.
    pub fn count(&self) -> usize {
        fn n(items: &[Item]) -> usize {
            items.iter().map(|i| if let Item::Clip { items, .. } = i { 1 + n(items) } else { 1 }).sum()
        }
        n(&self.items)
    }

    /// Every text string on the figure (for tests and accessibility).
    pub fn texts(&self) -> Vec<String> {
        fn collect(items: &[Item], out: &mut Vec<String>) {
            for i in items {
                match i {
                    Item::Text(t) => out.push(t.text.clone()),
                    Item::Clip { items, .. } => collect(items, out),
                    _ => {}
                }
            }
        }
        let mut v = Vec::new();
        collect(&self.items, &mut v);
        v
    }
}

// ---- text metrics --------------------------------------------------------------------------------------------

/// What a character becomes in the PDF (WinAnsi): its byte, or a short ASCII stand-in.
fn win_ansi(c: char) -> Vec<u8> {
    match c {
        ' '..='~' => vec![c as u8],
        '\u{A0}'..='\u{FF}' => vec![c as u32 as u8],
        '▸' | '›' | '►' | '▶' | '→' => vec![b'>'],
        '◂' | '‹' | '◄' | '◀' | '←' => vec![b'<'],
        '≥' => b">=".to_vec(),
        '≤' => b"<=".to_vec(),
        '≠' => b"!=".to_vec(),
        '–' => vec![0x96],
        '—' => vec![0x97],
        '…' => vec![0x85],
        '•' => vec![0x95],
        '‘' | '’' => vec![b'\''],
        '“' | '”' => vec![b'"'],
        '\t' | '\n' | '\r' => vec![b' '],
        _ => vec![b'?'],
    }
}

fn glyph_width(b: u8, bold: bool) -> f32 {
    let (table, extra): (&[u16; 95], &[(u8, u16)]) = if bold { (&HELVETICA_BOLD, &HELVETICA_BOLD_EXTRA) } else { (&HELVETICA, &HELVETICA_EXTRA) };
    match b {
        32..=126 => table[(b - 32) as usize] as f32,
        _ => extra.iter().find(|(c, _)| *c == b).map_or(556.0, |(_, w)| *w as f32),
    }
}

/// Width in points of a string set in Helvetica (or Helvetica-Bold) at `size`.
pub fn text_width(text: &str, size: f32, bold: bool) -> f32 {
    text.chars().flat_map(win_ansi).map(|b| glyph_width(b, bold)).sum::<f32>() * size / 1000.0
}

/// Baseline origin of a text item: where its first glyph starts, in figure coordinates.
fn origin(t: &Text) -> (f32, f32) {
    let w = text_width(&t.text, t.size, t.bold);
    if t.vertical {
        let start = match t.align {
            Align::Left => t.y,
            Align::Center => t.y + w / 2.0,
            Align::Right => t.y + w,
        };
        return (t.x + 0.36 * t.size, start);
    }
    let x = match t.align {
        Align::Left => t.x,
        Align::Center => t.x - w / 2.0,
        Align::Right => t.x - w,
    };
    let y = match t.valign {
        VAlign::Top => t.y + 0.72 * t.size,
        VAlign::Middle => t.y + 0.36 * t.size,
        VAlign::Bottom => t.y,
    };
    (x, y)
}

fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" || s == "-0" { "0".to_string() } else { s.to_string() }
}

// ---- SVG ------------------------------------------------------------------------------------------------------------

fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            c if (c as u32) < 32 && c != '\t' && c != '\n' => " ".to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn rgb_hex(c: Rgba) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
    for ch in bytes.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

fn png_rgb(w: u32, h: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().expect("png header");
        wr.write_image_data(rgb).expect("png data");
    }
    out
}

fn svg_paint(fill: Option<Rgba>, stroke: &Option<Stroke>) -> String {
    let mut s = match fill {
        Some(c) => format!(" fill=\"{}\"{}", rgb_hex(c), if c[3] < 255 { format!(" fill-opacity=\"{:.3}\"", c[3] as f32 / 255.0) } else { String::new() }),
        None => " fill=\"none\"".to_string(),
    };
    if let Some(st) = stroke {
        s += &format!(" stroke=\"{}\" stroke-width=\"{}\"", rgb_hex(st.color), num(st.width));
        if st.color[3] < 255 {
            s += &format!(" stroke-opacity=\"{:.3}\"", st.color[3] as f32 / 255.0);
        }
        if let Some((a, b)) = st.dash {
            s += &format!(" stroke-dasharray=\"{} {}\"", num(a), num(b));
        }
    }
    s
}

fn svg_items(items: &[Item], out: &mut String, clip_id: &mut usize) {
    for item in items {
        match item {
            Item::Rect { x, y, w, h, fill, stroke } => out.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"{}/>\n", num(*x), num(*y), num(*w), num(*h), svg_paint(*fill, stroke))),
            Item::Line { a, b, stroke } => out.push_str(&format!("<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"{}/>\n", num(a.0), num(a.1), num(b.0), num(b.1), svg_paint(None, &Some(stroke.clone())))),
            Item::Poly { pts, closed, fill, stroke } => {
                let p: Vec<String> = pts.iter().map(|(x, y)| format!("{},{}", num(*x), num(*y))).collect();
                let tag = if *closed { "polygon" } else { "polyline" };
                out.push_str(&format!("<{tag} points=\"{}\"{}/>\n", p.join(" "), svg_paint(*fill, stroke)));
            }
            Item::Text(t) => {
                let (bx, by) = origin(t);
                let anchor = match t.align {
                    Align::Left => "start",
                    Align::Center => "middle",
                    Align::Right => "end",
                };
                // text-anchor does the horizontal alignment, so use the anchor point itself rather than the start
                let w = text_width(&t.text, t.size, t.bold);
                let ax = match t.align {
                    Align::Left => bx,
                    Align::Center => bx + w / 2.0,
                    Align::Right => bx + w,
                };
                let style = format!("font-family=\"Helvetica, Arial, sans-serif\" font-size=\"{}\"{} fill=\"{}\" text-anchor=\"{anchor}\"", num(t.size), if t.bold { " font-weight=\"bold\"" } else { "" }, rgb_hex(t.color)) + if t.italic { " font-style=\"italic\"" } else { "" };
                if t.vertical {
                    out.push_str(&format!("<text transform=\"translate({},{}) rotate(-90)\" x=\"0\" y=\"0\" {style}>{}</text>\n", num(bx), num(by), xml_escape(&t.text)));
                    // the anchor along the text is relative to its start after rotation
                    let last = out.rfind("<text").unwrap();
                    let fixed = out[last..].replacen("x=\"0\"", &format!("x=\"{}\"", num(match t.align { Align::Left => 0.0, Align::Center => w / 2.0, Align::Right => w })), 1);
                    out.truncate(last);
                    out.push_str(&fixed);
                } else {
                    out.push_str(&format!("<text x=\"{}\" y=\"{}\" {style}>{}</text>\n", num(ax), num(by), xml_escape(&t.text)));
                }
            }
            Item::Image { x, y, w, h, px_w, px_h, rgb } => {
                let png = png_rgb(*px_w, *px_h, rgb);
                out.push_str(&format!("<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" image-rendering=\"pixelated\" href=\"data:image/png;base64,{}\"/>\n", num(*x), num(*y), num(*w), num(*h), base64(&png)));
            }
            Item::Clip { x, y, w, h, items } => {
                *clip_id += 1;
                let id = *clip_id;
                out.push_str(&format!("<clipPath id=\"clip{id}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath>\n<g clip-path=\"url(#clip{id})\">\n", num(*x), num(*y), num(*w), num(*h)));
                svg_items(items, out, clip_id);
                out.push_str("</g>\n");
            }
        }
    }
}

/// The figure as a standalone SVG document.
pub fn to_svg(fig: &Figure) -> String {
    let mut out = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}pt\" height=\"{h}pt\" viewBox=\"0 0 {w} {h}\">\n<rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>\n", w = num(fig.width), h = num(fig.height));
    let mut clip_id = 0;
    svg_items(&fig.items, &mut out, &mut clip_id);
    out.push_str("</svg>\n");
    out
}

// ---- PDF -------------------------------------------------------------------------------------------------------------

struct PageCtx {
    h: f32,
    content: String,
    images: Vec<(usize, usize)>,
    alphas: Vec<u8>,
}

impl PageCtx {
    fn y(&self, y: f32) -> String {
        num(self.h - y)
    }
    fn rg(c: Rgba, stroke: bool) -> String {
        format!("{:.3} {:.3} {:.3} {}\n", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, if stroke { "RG" } else { "rg" })
    }
    fn alpha(&mut self, a: u8) {
        if !self.alphas.contains(&a) {
            self.alphas.push(a);
        }
        self.content.push_str(&format!("/A{a} gs\n"));
    }
    fn stroke_style(&mut self, s: &Stroke) {
        self.content.push_str(&Self::rg(s.color, true));
        self.content.push_str(&format!("{} w\n", num(s.width)));
        match s.dash {
            Some((a, b)) => self.content.push_str(&format!("[{} {}] 0 d\n", num(a), num(b))),
            None => self.content.push_str("[] 0 d\n"),
        }
    }
    /// Fill and/or stroke the path already in the content stream.
    fn paint(&mut self, path: &str, fill: Option<Rgba>, stroke: &Option<Stroke>, close: bool) {
        if let Some(c) = fill {
            self.content.push_str("q\n");
            self.alpha(c[3]);
            self.content.push_str(&Self::rg(c, false));
            self.content.push_str(path);
            self.content.push_str("f\nQ\n");
        }
        if let Some(s) = stroke {
            self.content.push_str("q\n");
            self.alpha(s.color[3]);
            self.stroke_style(s);
            self.content.push_str(path);
            self.content.push_str(if close { "s\n" } else { "S\n" });
            self.content.push_str("Q\n");
        }
    }
}

fn pdf_escape(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 4);
    for &b in bytes {
        match b {
            b'(' | b')' | b'\\' => {
                out.push(b'\\');
                out.push(b);
            }
            b => out.push(b),
        }
    }
    out
}

fn pdf_items(items: &[Item], cx: &mut PageCtx, objs: &mut Vec<Vec<u8>>) {
    for item in items {
        match item {
            Item::Rect { x, y, w, h, fill, stroke } => {
                let path = format!("{} {} {} {} re\n", num(*x), cx.y(*y + *h), num(*w), num(*h));
                cx.paint(&path, *fill, stroke, false);
            }
            Item::Line { a, b, stroke } => {
                let path = format!("{} {} m\n{} {} l\n", num(a.0), cx.y(a.1), num(b.0), cx.y(b.1));
                cx.paint(&path, None, &Some(stroke.clone()), false);
            }
            Item::Poly { pts, closed, fill, stroke } => {
                if pts.len() < 2 {
                    continue;
                }
                let mut path = format!("{} {} m\n", num(pts[0].0), cx.y(pts[0].1));
                for p in &pts[1..] {
                    path += &format!("{} {} l\n", num(p.0), cx.y(p.1));
                }
                if *closed {
                    path += "h\n";
                }
                cx.paint(&path, if *closed { *fill } else { None }, stroke, *closed);
            }
            Item::Text(t) => {
                let bytes: Vec<u8> = t.text.chars().flat_map(win_ansi).collect();
                let (bx, by) = origin(t);
                let tm = if t.vertical { format!("0 1 -1 0 {} {} Tm", num(bx), cx.y(by)) } else { format!("1 0 0 1 {} {} Tm", num(bx), cx.y(by)) };
                cx.content.push_str("q\n");
                cx.alpha(t.color[3]);
                let font = match (t.bold, t.italic) {
                    (false, false) => "F1",
                    (true, false) => "F2",
                    (false, true) => "F3",
                    (true, true) => "F4",
                };
                cx.content.push_str(&format!("BT\n/{font} {} Tf\n{}{}\n(", num(t.size), PageCtx::rg(t.color, false), tm));
                let esc = pdf_escape(&bytes);
                // the content stream is bytes; WinAnsi bytes above 127 are kept as Latin-1 chars and re-encoded on write
                cx.content.extend(esc.iter().map(|&b| b as char));
                cx.content.push_str(") Tj\nET\nQ\n");
            }
            Item::Image { x, y, w, h, px_w, px_h, rgb } => {
                let z = miniz_oxide::deflate::compress_to_vec_zlib(rgb, 6);
                let mut body = format!("<< /Type /XObject /Subtype /Image /Width {px_w} /Height {px_h} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Interpolate false /Filter /FlateDecode /Length {} >>\nstream\n", z.len()).into_bytes();
                body.extend(&z);
                body.extend(b"\nendstream");
                objs.push(body);
                let id = objs.len();
                let n = cx.images.len() + 1;
                cx.images.push((n, id));
                cx.content.push_str(&format!("q\n{} 0 0 {} {} {} cm\n/Im{n} Do\nQ\n", num(*w), num(*h), num(*x), cx.y(*y + *h)));
            }
            Item::Clip { x, y, w, h, items } => {
                cx.content.push_str(&format!("q\n{} {} {} {} re W n\n", num(*x), cx.y(*y + *h), num(*w), num(*h)));
                pdf_items(items, cx, objs);
                cx.content.push_str("Q\n");
            }
        }
    }
}

/// A multi-page PDF, one page per figure (each page the size of its figure).
pub fn to_pdf(pages: &[Figure], title: &str) -> Vec<u8> {
    // object ids: 1 catalog, 2 pages, 3 Helvetica, 4 Helvetica-Bold, 5 info; the rest are created as needed
    let mut objs: Vec<Vec<u8>> = vec![vec![]; 7];
    objs[2] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec();
    objs[3] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec();
    objs[5] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Oblique /Encoding /WinAnsiEncoding >>".to_vec();
    objs[6] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-BoldOblique /Encoding /WinAnsiEncoding >>".to_vec();
    let t: Vec<u8> = title.chars().flat_map(win_ansi).collect();
    let mut info = b"<< /Title (".to_vec();
    info.extend(pdf_escape(&t));
    info.extend(b") /Producer (StreamFLOW) >>");
    objs[4] = info;
    let mut alpha_ids: Vec<(u8, usize)> = Vec::new();
    let mut page_ids = Vec::new();
    for fig in pages {
        let mut cx = PageCtx { h: fig.height, content: String::new(), images: vec![], alphas: vec![] };
        pdf_items(&fig.items, &mut cx, &mut objs);
        for a in &cx.alphas {
            if !alpha_ids.iter().any(|(x, _)| x == a) {
                let c = *a as f32 / 255.0;
                objs.push(format!("<< /Type /ExtGState /ca {c:.4} /CA {c:.4} >>").into_bytes());
                alpha_ids.push((*a, objs.len()));
            }
        }
        // WinAnsi bytes were carried as Latin-1 chars
        let content: Vec<u8> = cx.content.chars().map(|c| c as u32 as u8).collect();
        let mut body = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
        body.extend(&content);
        body.extend(b"\nendstream");
        objs.push(body);
        let content_id = objs.len();
        let xobj: String = cx.images.iter().map(|(n, id)| format!("/Im{n} {id} 0 R ")).collect();
        let gs: String = cx.alphas.iter().map(|a| format!("/A{a} {} 0 R ", alpha_ids.iter().find(|(x, _)| x == a).unwrap().1)).collect();
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 3 0 R /F2 4 0 R /F3 6 0 R /F4 7 0 R >> /XObject << {xobj}>> /ExtGState << {gs}>> >> /Contents {content_id} 0 R >>", num(fig.width), num(fig.height)).into_bytes());
        page_ids.push(objs.len());
    }
    let kids: String = page_ids.iter().map(|i| format!("{i} 0 R ")).collect();
    objs[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    objs[1] = format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", page_ids.len()).into_bytes();

    let mut out: Vec<u8> = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objs.len());
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(body);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in &offsets {
        out.extend(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// A strict structural check of a PDF written by [`to_pdf`] (and any simple PDF with a classic xref table).
/// Returns the number of pages, or what is wrong. Verifies the header, the cross-reference offsets, that every
/// object is where the table says, stream lengths, and that every image decompresses to exactly the bytes its
/// dictionary promises.
pub fn validate_pdf(data: &[u8]) -> Result<usize, String> {
    let find = |hay: &[u8], needle: &[u8], from: usize| -> Option<usize> { hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from) };
    if !data.starts_with(b"%PDF-1.") {
        return Err("missing %PDF header".into());
    }
    if !data.ends_with(b"%%EOF\n") {
        return Err("missing %%EOF".into());
    }
    let sx = data.windows(9).rposition(|w| w == b"startxref").ok_or("no startxref")?;
    let tail = std::str::from_utf8(&data[sx + 9..]).map_err(|_| "startxref not text")?;
    let xref_at: usize = tail.split_whitespace().next().ok_or("no xref offset")?.parse().map_err(|_| "bad xref offset")?;
    if !data[xref_at..].starts_with(b"xref\n") {
        return Err("startxref does not point at the xref table".into());
    }
    let head = std::str::from_utf8(&data[xref_at + 5..xref_at + 40.min(data.len() - xref_at - 5)]).unwrap_or("");
    let mut it = head.split_whitespace();
    let (first, count): (usize, usize) = (it.next().and_then(|v| v.parse().ok()).ok_or("bad xref header")?, it.next().and_then(|v| v.parse().ok()).ok_or("bad xref count")?);
    if first != 0 {
        return Err("xref must start at 0".into());
    }
    let table_start = find(data, b"\n", xref_at + 5).ok_or("xref header")? + 1;
    let mut pages = 0;
    for n in 0..count {
        let line = &data[table_start + n * 20..table_start + (n + 1) * 20];
        let line = std::str::from_utf8(line).map_err(|_| "xref entry not text")?;
        if n == 0 {
            if !line.starts_with("0000000000 65535 f") {
                return Err("bad free entry".into());
            }
            continue;
        }
        let off: usize = line[..10].parse().map_err(|_| format!("bad offset for object {n}"))?;
        if !line[10..].starts_with(" 00000 n") {
            return Err(format!("bad entry for object {n}"));
        }
        let expect = format!("{n} 0 obj\n");
        if !data[off..].starts_with(expect.as_bytes()) {
            return Err(format!("object {n} is not at offset {off}"));
        }
        let end = find(data, b"\nendobj\n", off).ok_or(format!("object {n} has no endobj"))?;
        let body = &data[off + expect.len()..end];
        if let Some(sp) = find(body, b"stream\n", 0) {
            let dict = std::str::from_utf8(&body[..sp]).map_err(|_| "stream dict not text")?;
            let len: usize = dict.split("/Length ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|v| v.parse().ok()).ok_or(format!("object {n}: no /Length"))?;
            let s = sp + 7;
            if body.len() < s + len + 10 || &body[s + len..] != b"\nendstream" {
                return Err(format!("object {n}: stream is not exactly {len} bytes"));
            }
            if dict.contains("/FlateDecode") {
                let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&body[s..s + len]).map_err(|e| format!("object {n}: bad deflate data ({e:?})"))?;
                let dim = |key: &str| -> Option<usize> { dict.split(key).nth(1)?.split_whitespace().next()?.parse().ok() };
                if let (Some(w), Some(h)) = (dim("/Width "), dim("/Height ")) {
                    if raw.len() != w * h * 3 {
                        return Err(format!("object {n}: image is {} bytes, expected {}", raw.len(), w * h * 3));
                    }
                }
            }
        }
        if find(body, b"/Type /Page ", 0).is_some() {
            pages += 1;
        }
    }
    let trailer = std::str::from_utf8(&data[table_start + count * 20..]).map_err(|_| "trailer not text")?;
    if !trailer.starts_with("trailer\n") || !trailer.contains(&format!("/Size {count}")) {
        return Err("trailer /Size does not match the table".into());
    }
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Figure {
        let mut f = Figure::new(300.0, 200.0);
        f.push(Item::Rect { x: 10.0, y: 10.0, w: 100.0, h: 50.0, fill: Some([255, 0, 0, 128]), stroke: Some(Stroke::new([0, 0, 0, 255], 1.0)) });
        f.push(Item::Line { a: (0.0, 0.0), b: (300.0, 200.0), stroke: Stroke::dashed([0, 0, 255, 255], 1.5, 4.0, 2.0) });
        f.push(Item::Poly { pts: vec![(20.0, 100.0), (80.0, 100.0), (50.0, 160.0)], closed: true, fill: None, stroke: Some(Stroke::new([0, 128, 0, 255], 2.0)) });
        f.push(Item::Text(Text::new(150.0, 30.0, "Hello (world) \\ ≥ ▸ é", 12.0).align(Align::Center, VAlign::Middle)));
        f.push(Item::Text(Text::new(12.0, 100.0, "CD3 vertical", 10.0).align(Align::Center, VAlign::Middle).vertical().bold()));
        f.push(Item::Text(Text::new(150.0, 60.0, "control layer", 9.0).italic()));
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| (i * 20) as u8).collect();
        f.push(Item::Clip { x: 150.0, y: 80.0, w: 100.0, h: 80.0, items: vec![Item::Image { x: 140.0, y: 70.0, w: 120.0, h: 90.0, px_w: 4, px_h: 3, rgb }] });
        f
    }

    #[test]
    fn helvetica_widths_come_from_the_adobe_metrics() {
        // H=722 e=556 l=222 l=222 o=556 -> 2278/1000 em
        assert!((text_width("Hello", 10.0, false) - 22.78).abs() < 1e-4);
        assert!((text_width("Hello", 10.0, true) - 10.0 * (722.0 + 556.0 + 278.0 + 278.0 + 611.0) / 1000.0).abs() < 1e-4, "bold H e l l o = 722 556 278 278 611");
        assert_eq!(text_width("", 12.0, false), 0.0);
        // a character with no WinAnsi glyph still occupies space and prints as ?
        assert_eq!(win_ansi('\u{4E2D}'), vec![b'?']);
        assert_eq!(win_ansi('≥'), b">=".to_vec());
        assert!((text_width("≥", 10.0, false) - text_width(">=", 10.0, false)).abs() < 1e-6);
    }

    #[test]
    fn svg_is_well_formed_xml_and_carries_every_text() {
        let svg = to_svg(&sample());
        let doc = sf_core::xml::parse(&svg).expect("the SVG parses as XML");
        assert_eq!(doc.name, "svg");
        assert!(svg.contains("Hello (world) \\ ≥ ▸ é"), "SVG keeps full Unicode");
        assert!(svg.contains("clip-path=\"url(#clip1)\"") && svg.contains("data:image/png;base64,"));
        assert!(svg.contains("stroke-dasharray=\"4 2\"") && svg.contains("fill-opacity=\"0.502\""));
        assert!(svg.contains("rotate(-90)") && svg.contains("font-style=\"italic\""));
        // markup characters in text are escaped
        let mut f = Figure::new(10.0, 10.0);
        f.push(Item::Text(Text::new(1.0, 1.0, "a<b & \"c\"", 8.0)));
        assert!(to_svg(&f).contains("a&lt;b &amp; &quot;c&quot;") && sf_core::xml::parse(&to_svg(&f)).is_ok());
    }

    #[test]
    fn the_pdf_is_structurally_valid_with_exact_offsets_streams_and_images() {
        let mut pages = vec![sample(), Figure::new(100.0, 100.0)];
        pages[1].push(Item::Text(Text::new(5.0, 50.0, "page two", 9.0)));
        let pdf = to_pdf(&pages, "Test layout");
        assert_eq!(validate_pdf(&pdf), Ok(2), "{}", String::from_utf8_lossy(&pdf[..200.min(pdf.len())]));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/BaseFont /Helvetica ") && text.contains("/Title (Test layout)"));
        assert!(text.contains("/BaseFont /Helvetica-Oblique ") && text.contains("/F3 6 0 R") && text.contains("/F3 9 Tf"), "italic text uses the oblique font");
        assert!(text.contains("/MediaBox [0 0 300 200]") && text.contains("/MediaBox [0 0 100 100]"));
        assert!(text.contains("/A128 gs") && text.contains("/ca 0.5020"), "constant alpha is expressed as an ExtGState");
        assert!(text.contains("[4 2] 0 d") && text.contains("re W n"), "dash pattern and clipping are written");
        // the nasty characters were escaped and mapped: ( ) \ -> escaped, >= and > stand in for the symbols, e-acute is byte 0xE9
        assert!(pdf.windows(b"Hello \\(world\\) \\\\ >= > \xE9".len()).any(|w| w == b"Hello \\(world\\) \\\\ >= > \xE9"));
        // y is flipped: the top-left rect at y=10,h=50 has its PDF bottom edge at 200-60=140
        assert!(text.contains("10 140 100 50 re"), "{}", text.lines().filter(|l| l.contains(" re")).collect::<Vec<_>>().join(" | "));
        // deterministic: no dates, same bytes twice
        assert_eq!(pdf, to_pdf(&pages, "Test layout"));
    }

    #[test]
    fn the_validator_rejects_damaged_files() {
        let pdf = to_pdf(&[sample()], "x");
        assert_eq!(validate_pdf(&pdf), Ok(1));
        let mut bad = pdf.clone();
        let i = bad.windows(7).position(|w| w == b"1 0 obj").unwrap();
        bad.splice(i..i, b"  ".iter().copied()); // shift every offset
        assert!(validate_pdf(&bad).is_err());
        assert!(validate_pdf(b"not a pdf").is_err());
        let truncated = &pdf[..pdf.len() - 8];
        assert!(validate_pdf(truncated).is_err());
        // corrupt a deflate stream
        let mut corrupt = pdf.clone();
        let s = corrupt.windows(13).position(|w| w == b"/FlateDecode ").unwrap();
        let st = corrupt[s..].windows(7).position(|w| w == b"stream\n").unwrap() + s + 7;
        corrupt[st + 3] ^= 0xFF;
        corrupt[st + 4] ^= 0xFF;
        assert!(validate_pdf(&corrupt).is_err(), "a damaged image stream is caught");
    }

    #[test]
    fn placing_a_figure_moves_every_item_including_clipped_ones() {
        let mut page = Figure::new(500.0, 500.0);
        page.place(sample(), 100.0, 200.0);
        match &page.items[0] {
            Item::Rect { x, y, .. } => assert_eq!((*x, *y), (110.0, 210.0)),
            o => panic!("{o:?}"),
        }
        match page.items.last().unwrap() {
            Item::Clip { x, y, items, .. } => {
                assert_eq!((*x, *y), (250.0, 280.0));
                assert!(matches!(&items[0], Item::Image { x, y, .. } if (*x, *y) == (240.0, 270.0)));
            }
            o => panic!("{o:?}"),
        }
        assert_eq!(page.count(), sample().count());
    }
}
