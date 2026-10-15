//! The Layout Editor's document: a canvas (one page) holding tiles and text boxes, plus how a batch iterates it.
//!
//! * A **tile** is a plot blueprint, not a picture: population, axes, view, and a list of **layers** (samples drawn
//!   on it). Dropping a second sample onto a tile adds an overlay layer.
//! * A layer is an **iterator** (its sample is swapped for each page of a batch) or a **control** (the same on
//!   every page). The sample named in the layer is what the editor shows when no batch is running.
//! * **Text boxes** may hold tokens (`[File_Name]`, `[Keyword]`, `[Page]`, ...) replaced for every page.
//! * The whole document is saved in the workspace file.

use crate::density::{PlotKind, Range};
use crate::layout::PageSetup;
use sf_core::json::Json;
use sf_core::scale::ScaleSpec;

/// Overlay colours, by palette index (wraps around).
pub const PALETTE: [[u8; 3]; 8] = [[0x1F, 0x77, 0xB4], [0xD6, 0x27, 0x28], [0x2C, 0xA0, 0x2C], [0xFF, 0x7F, 0x0E], [0x94, 0x67, 0xBD], [0x8C, 0x56, 0x4B], [0xE3, 0x77, 0xC2], [0x17, 0xBE, 0xCF]];

pub const MIN_TILE: f32 = 90.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerRole {
    /// Swapped for each sample (or group of samples) of the batch.
    Iterator,
    /// The same on every page.
    Control,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// What the editor shows; for a control also what every page shows.
    pub sample: String,
    pub role: LayerRole,
    /// Index into [`PALETTE`].
    pub color: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewMode {
    /// One view for the whole batch: the union of every layer on every page, so pages are comparable.
    Lock,
    Fixed(Range),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub population: Vec<String>,
    pub x_chan: String,
    pub y_chan: String,
    pub kind: PlotKind,
    /// `None`: the default scale for the channel.
    pub x_scale: Option<ScaleSpec>,
    pub y_scale: Option<ScaleSpec>,
    pub view: ViewMode,
    pub layers: Vec<Layer>,
    pub show_gates: bool,
    pub show_fmo: bool,
    pub legend: bool,
    /// Text above the plot; tokens allowed.
    pub title: String,
}

impl Tile {
    pub fn new(id: u32, x: f32, y: f32, w: f32, h: f32) -> Tile {
        Tile { id, x, y, w, h, population: vec![], x_chan: String::new(), y_chan: String::new(), kind: PlotKind::Pseudocolor, x_scale: None, y_scale: None, view: ViewMode::Lock, layers: vec![], show_gates: true, show_fmo: true, legend: true, title: "[File_Name]".into() }
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 <= self.x + self.w && p.1 >= self.y && p.1 <= self.y + self.h
    }

    /// Add a sample as an overlay layer. `false` (and nothing added) if it is already on the tile.
    pub fn add_layer(&mut self, sample: &str, role: LayerRole) -> bool {
        if self.layers.iter().any(|l| l.sample == sample) {
            return false;
        }
        let color = self.layers.iter().map(|l| l.color + 1).max().unwrap_or(0);
        self.layers.push(Layer { sample: sample.to_string(), role, color });
        true
    }

    /// Make a layer a control (`true`) or an iterator (`false`).
    pub fn set_control(&mut self, layer: usize, control: bool) -> bool {
        match self.layers.get_mut(layer) {
            Some(l) => {
                l.role = if control { LayerRole::Control } else { LayerRole::Iterator };
                true
            }
            None => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextBox {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub text: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Iterate {
    /// One page per sample.
    Sample,
    /// One page per distinct value of an FCS keyword; the page's iterator layers show all samples with that value.
    Keyword(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayoutDoc {
    pub name: String,
    pub a4: bool,
    pub landscape: bool,
    pub tiles: Vec<Tile>,
    pub texts: Vec<TextBox>,
    pub iterate: Iterate,
    /// Samples the batch runs over, by name; empty = every loaded sample.
    pub samples: Vec<String>,
    next_id: u32,
}

impl LayoutDoc {
    pub fn new(name: &str) -> LayoutDoc {
        LayoutDoc { name: name.to_string(), a4: false, landscape: true, tiles: vec![], texts: vec![], iterate: Iterate::Sample, samples: vec![], next_id: 1 }
    }

    pub fn page(&self) -> PageSetup {
        let mut p = if self.a4 { PageSetup::a4(self.landscape) } else { PageSetup::letter(self.landscape) };
        p.margin = 0.0;
        p
    }

    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id - 1
    }

    /// A new tile centred on `at`, kept on the page.
    pub fn add_tile(&mut self, at: (f32, f32), w: f32, h: f32) -> u32 {
        let id = self.id();
        let p = self.page();
        let (x, y) = ((at.0 - w / 2.0).clamp(0.0, (p.width - w).max(0.0)), (at.1 - h / 2.0).clamp(0.0, (p.height - h).max(0.0)));
        self.tiles.push(Tile::new(id, x, y, w, h));
        id
    }

    pub fn add_text(&mut self, at: (f32, f32), text: &str) -> u32 {
        let id = self.id();
        self.texts.push(TextBox { id, x: at.0, y: at.1, text: text.to_string(), size: 12.0, bold: false, italic: false });
        id
    }

    pub fn tile(&self, id: u32) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }

    pub fn tile_mut(&mut self, id: u32) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|t| t.id == id)
    }

    pub fn remove_tile(&mut self, id: u32) -> bool {
        let n = self.tiles.len();
        self.tiles.retain(|t| t.id != id);
        self.tiles.len() != n
    }

    pub fn remove_text(&mut self, id: u32) -> bool {
        let n = self.texts.len();
        self.texts.retain(|t| t.id != id);
        self.texts.len() != n
    }

    /// The topmost tile under a page point (later tiles are drawn on top).
    pub fn tile_at(&self, p: (f32, f32)) -> Option<u32> {
        self.tiles.iter().rev().find(|t| t.contains(p)).map(|t| t.id)
    }

    /// Move a tile by `(dx, dy)`, keeping it on the page.
    pub fn move_tile(&mut self, id: u32, dx: f32, dy: f32) {
        let p = self.page();
        if let Some(t) = self.tile_mut(id) {
            t.x = (t.x + dx).clamp(0.0, (p.width - t.w).max(0.0));
            t.y = (t.y + dy).clamp(0.0, (p.height - t.h).max(0.0));
        }
    }

    /// Resize from the bottom-right corner, keeping at least [`MIN_TILE`] and staying on the page.
    pub fn resize_tile(&mut self, id: u32, dw: f32, dh: f32) {
        let p = self.page();
        if let Some(t) = self.tile_mut(id) {
            t.w = (t.w + dw).clamp(MIN_TILE, (p.width - t.x).max(MIN_TILE));
            t.h = (t.h + dh).clamp(MIN_TILE, (p.height - t.y).max(MIN_TILE));
        }
    }

    /// Bring a tile to the front.
    pub fn raise(&mut self, id: u32) {
        if let Some(i) = self.tiles.iter().position(|t| t.id == id) {
            let t = self.tiles.remove(i);
            self.tiles.push(t);
        }
    }

    // ---- JSON ------------------------------------------------------------------------------------------------

    pub fn to_json(&self) -> Json {
        let tiles: Vec<Json> = self.tiles.iter().map(tile_json).collect();
        let texts: Vec<Json> = self
            .texts
            .iter()
            .map(|t| Json::obj().with("id", Json::num(t.id as f64)).with("x", Json::num(t.x as f64)).with("y", Json::num(t.y as f64)).with("text", Json::Str(t.text.clone())).with("size", Json::num(t.size as f64)).with("bold", Json::Bool(t.bold)).with("italic", Json::Bool(t.italic)))
            .collect();
        let iterate = match &self.iterate {
            Iterate::Sample => Json::Str("sample".into()),
            Iterate::Keyword(k) => Json::obj().with("keyword", Json::Str(k.clone())),
        };
        Json::obj()
            .with("name", Json::Str(self.name.clone()))
            .with("a4", Json::Bool(self.a4))
            .with("landscape", Json::Bool(self.landscape))
            .with("tiles", Json::Arr(tiles))
            .with("texts", Json::Arr(texts))
            .with("iterate", iterate)
            .with("samples", Json::Arr(self.samples.iter().map(|s| Json::Str(s.clone())).collect()))
            .with("next_id", Json::num(self.next_id as f64))
    }

    pub fn from_json(j: &Json) -> Result<LayoutDoc, String> {
        if !matches!(j, Json::Obj(_)) {
            return Err("layout: the saved layout is not an object".into());
        }
        let mut doc = LayoutDoc::new(j.get("name").and_then(Json::as_str).unwrap_or("Layout"));
        doc.a4 = j.get("a4").and_then(Json::as_bool).unwrap_or(false);
        doc.landscape = j.get("landscape").and_then(Json::as_bool).unwrap_or(true);
        doc.iterate = match j.get("iterate") {
            Some(Json::Str(s)) if s == "sample" => Iterate::Sample,
            Some(o @ Json::Obj(_)) => Iterate::Keyword(o.get("keyword").and_then(Json::as_str).ok_or("layout: iterate keyword is missing")?.to_string()),
            None => Iterate::Sample,
            Some(_) => return Err("layout: unknown 'iterate' setting".into()),
        };
        if let Some(a) = j.get("samples").and_then(Json::as_arr) {
            doc.samples = a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect();
        }
        for t in j.get("tiles").and_then(Json::as_arr).unwrap_or(&[]) {
            doc.tiles.push(tile_from_json(t)?);
        }
        for t in j.get("texts").and_then(Json::as_arr).unwrap_or(&[]) {
            let f = |k: &str| t.get(k).and_then(Json::as_f64).ok_or_else(|| format!("layout: text box is missing '{k}'"));
            doc.texts.push(TextBox { id: f("id")? as u32, x: f("x")? as f32, y: f("y")? as f32, text: t.get("text").and_then(Json::as_str).unwrap_or("").to_string(), size: f("size")? as f32, bold: t.get("bold").and_then(Json::as_bool).unwrap_or(false), italic: t.get("italic").and_then(Json::as_bool).unwrap_or(false) });
        }
        let max_id = doc.tiles.iter().map(|t| t.id).chain(doc.texts.iter().map(|t| t.id)).max().unwrap_or(0);
        doc.next_id = (j.get("next_id").and_then(Json::as_f64).unwrap_or(0.0) as u32).max(max_id + 1);
        Ok(doc)
    }
}

fn kind_name(k: PlotKind) -> &'static str {
    match k {
        PlotKind::Pseudocolor => "pseudocolor",
        PlotKind::Dot => "dot",
        PlotKind::Histogram => "histogram",
    }
}

fn tile_json(t: &Tile) -> Json {
    let layers: Vec<Json> = t.layers.iter().map(|l| Json::obj().with("sample", Json::Str(l.sample.clone())).with("control", Json::Bool(l.role == LayerRole::Control)).with("color", Json::num(l.color as f64))).collect();
    let view = match &t.view {
        ViewMode::Lock => Json::Str("lock".into()),
        ViewMode::Fixed(r) => Json::Arr(vec![Json::num(r.x0), Json::num(r.x1), Json::num(r.y0), Json::num(r.y1)]),
    };
    let mut o = Json::obj()
        .with("id", Json::num(t.id as f64))
        .with("x", Json::num(t.x as f64))
        .with("y", Json::num(t.y as f64))
        .with("w", Json::num(t.w as f64))
        .with("h", Json::num(t.h as f64))
        .with("population", Json::Arr(t.population.iter().map(|p| Json::Str(p.clone())).collect()))
        .with("x_chan", Json::Str(t.x_chan.clone()))
        .with("y_chan", Json::Str(t.y_chan.clone()))
        .with("kind", Json::Str(kind_name(t.kind).into()))
        .with("view", view)
        .with("layers", Json::Arr(layers))
        .with("show_gates", Json::Bool(t.show_gates))
        .with("show_fmo", Json::Bool(t.show_fmo))
        .with("legend", Json::Bool(t.legend))
        .with("title", Json::Str(t.title.clone()));
    if let Some(s) = &t.x_scale {
        o = o.with("x_scale", Json::Str(s.to_text()));
    }
    if let Some(s) = &t.y_scale {
        o = o.with("y_scale", Json::Str(s.to_text()));
    }
    o
}

fn tile_from_json(j: &Json) -> Result<Tile, String> {
    let f = |k: &str| j.get(k).and_then(Json::as_f64).ok_or_else(|| format!("layout: tile is missing '{k}'"));
    let mut t = Tile::new(f("id")? as u32, f("x")? as f32, f("y")? as f32, f("w")? as f32, f("h")? as f32);
    t.population = j.get("population").and_then(Json::as_arr).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default();
    t.x_chan = j.get("x_chan").and_then(Json::as_str).unwrap_or("").to_string();
    t.y_chan = j.get("y_chan").and_then(Json::as_str).unwrap_or("").to_string();
    t.kind = match j.get("kind").and_then(Json::as_str).unwrap_or("pseudocolor") {
        "pseudocolor" => PlotKind::Pseudocolor,
        "dot" => PlotKind::Dot,
        "histogram" => PlotKind::Histogram,
        other => return Err(format!("layout: unknown plot type '{other}'")),
    };
    let scale = |k: &str| -> Result<Option<ScaleSpec>, String> {
        match j.get(k).and_then(Json::as_str) {
            Some(s) => ScaleSpec::parse(s).map(|(sp, _)| Some(sp)),
            None => Ok(None),
        }
    };
    t.x_scale = scale("x_scale")?;
    t.y_scale = scale("y_scale")?;
    t.view = match j.get("view") {
        Some(Json::Str(s)) if s == "lock" => ViewMode::Lock,
        Some(Json::Arr(a)) if a.len() == 4 => {
            let v: Vec<f64> = a.iter().filter_map(Json::as_f64).collect();
            if v.len() != 4 {
                return Err("layout: a fixed view needs four numbers".into());
            }
            ViewMode::Fixed(Range { x0: v[0], x1: v[1], y0: v[2], y1: v[3] })
        }
        None => ViewMode::Lock,
        Some(_) => return Err("layout: unknown view setting".into()),
    };
    for l in j.get("layers").and_then(Json::as_arr).unwrap_or(&[]) {
        t.layers.push(Layer { sample: l.get("sample").and_then(Json::as_str).ok_or("layout: a layer has no sample")?.to_string(), role: if l.get("control").and_then(Json::as_bool).unwrap_or(false) { LayerRole::Control } else { LayerRole::Iterator }, color: l.get("color").and_then(Json::as_f64).unwrap_or(0.0) as usize });
    }
    t.show_gates = j.get("show_gates").and_then(Json::as_bool).unwrap_or(true);
    t.show_fmo = j.get("show_fmo").and_then(Json::as_bool).unwrap_or(true);
    t.legend = j.get("legend").and_then(Json::as_bool).unwrap_or(true);
    t.title = j.get("title").and_then(Json::as_str).unwrap_or("").to_string();
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> LayoutDoc {
        let mut d = LayoutDoc::new("Panel 1");
        let a = d.add_tile((200.0, 150.0), 220.0, 200.0);
        let t = d.tile_mut(a).unwrap();
        t.population = vec!["Lymphocytes".into(), "CD3+".into()];
        t.x_chan = "CD4-A".into();
        t.y_chan = "CD8-A".into();
        t.x_scale = Some(ScaleSpec::Arcsinh { cofactor: 150.0 });
        t.view = ViewMode::Fixed(Range { x0: -1.0, x1: 8.0, y0: -1.0, y1: 8.0 });
        t.add_layer("a.fcs", LayerRole::Iterator);
        t.add_layer("ctrl.fcs", LayerRole::Control);
        d.add_text((20.0, 20.0), "[File_Name] — [Page]");
        d.iterate = Iterate::Keyword("$FIL".into());
        d.samples = vec!["a.fcs".into(), "b.fcs".into()];
        d.a4 = true;
        d
    }

    #[test]
    fn a_document_survives_json_exactly() {
        let d = doc();
        let text = d.to_json().write();
        let back = LayoutDoc::from_json(&Json::parse(&text).unwrap()).unwrap();
        assert_eq!(back, d);
        // ids keep counting from where they were
        let mut back = back;
        let id = back.add_tile((300.0, 300.0), 100.0, 100.0);
        assert!(back.tiles.iter().filter(|t| t.id == id).count() == 1 && id > 2);
    }

    #[test]
    fn damaged_documents_are_refused_with_a_reason() {
        let mut j = doc().to_json();
        if let Json::Obj(o) = &mut j {
            o.insert("iterate".into(), Json::Num(3.0));
        }
        assert!(LayoutDoc::from_json(&j).unwrap_err().contains("iterate"));
        let bad = Json::parse(r#"{"name":"x","tiles":[{"id":1,"x":0,"y":0,"w":100,"h":100,"kind":"scatter3d"}]}"#).unwrap();
        assert!(LayoutDoc::from_json(&bad).unwrap_err().contains("unknown plot type"));
        let missing = Json::parse(r#"{"tiles":[{"id":1}]}"#).unwrap();
        assert!(LayoutDoc::from_json(&missing).unwrap_err().contains("missing 'x'"));
        assert!(LayoutDoc::from_json(&Json::Num(1.0)).unwrap_err().contains("not an object"));
        // an empty object is an empty layout, not an error
        assert!(LayoutDoc::from_json(&Json::obj()).unwrap().tiles.is_empty());
    }

    #[test]
    fn overlays_take_new_colours_and_duplicates_are_refused() {
        let mut t = Tile::new(1, 0.0, 0.0, 200.0, 200.0);
        assert!(t.add_layer("a", LayerRole::Iterator));
        assert!(t.add_layer("b", LayerRole::Iterator));
        assert!(!t.add_layer("a", LayerRole::Control), "the same sample twice is not an overlay");
        assert_eq!(t.layers.iter().map(|l| l.color).collect::<Vec<_>>(), vec![0, 1]);
        assert!(t.set_control(1, true) && t.layers[1].role == LayerRole::Control);
        assert!(t.set_control(1, false) && t.layers[1].role == LayerRole::Iterator);
        assert!(!t.set_control(5, true));
    }

    #[test]
    fn tiles_move_and_resize_but_stay_on_the_page() {
        let mut d = LayoutDoc::new("p");
        d.landscape = true; // 792 x 612
        let id = d.add_tile((100.0, 100.0), 200.0, 150.0);
        d.move_tile(id, -500.0, -500.0);
        assert_eq!((d.tile(id).unwrap().x, d.tile(id).unwrap().y), (0.0, 0.0));
        d.move_tile(id, 5000.0, 5000.0);
        assert_eq!((d.tile(id).unwrap().x, d.tile(id).unwrap().y), (792.0 - 200.0, 612.0 - 150.0));
        d.resize_tile(id, -1000.0, -1000.0);
        assert_eq!((d.tile(id).unwrap().w, d.tile(id).unwrap().h), (MIN_TILE, MIN_TILE), "never smaller than the minimum");
        d.resize_tile(id, 5000.0, 5000.0);
        let t = d.tile(id).unwrap();
        assert!(t.x + t.w <= 792.0 + 1e-3 && t.y + t.h <= 612.0 + 1e-3, "{t:?}");
        // a tile created near the edge is pulled onto the page
        let edge = d.add_tile((5.0, 5.0), 200.0, 200.0);
        assert_eq!((d.tile(edge).unwrap().x, d.tile(edge).unwrap().y), (0.0, 0.0));
    }

    #[test]
    fn the_topmost_tile_wins_a_hit_and_raise_changes_it() {
        let mut d = LayoutDoc::new("p");
        let a = d.add_tile((150.0, 150.0), 200.0, 200.0);
        let b = d.add_tile((200.0, 200.0), 200.0, 200.0);
        assert_eq!(d.tile_at((180.0, 180.0)), Some(b), "the later tile is on top where they overlap");
        d.raise(a);
        assert_eq!(d.tile_at((180.0, 180.0)), Some(a));
        assert_eq!(d.tile_at((700.0, 500.0)), None);
        assert!(d.remove_tile(a) && !d.remove_tile(a));
        assert_eq!(d.tile_at((180.0, 180.0)), Some(b));
    }
}
