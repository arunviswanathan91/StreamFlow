//! FlowJo workspace (`.wsp`) import: per-sample gating trees, transforms and compensation.
//!
//! FlowJo stores gate coordinates in raw data units and applies the sample's per-parameter transform
//! (linear / log / biex / logicle) to both the events and the gate vertices before testing membership,
//! so the imported tree lives in *transformed* space: each gate's channels are the workspace's display
//! names (`Comp-PE-Cy7-A` for a compensated axis) and [`WspSample::build_view`] produces matching
//! transformed columns from raw events. FlowJo also saves a population `count` for every node, which is
//! kept in [`WspSample::flowjo_counts`] so an import can be checked against FlowJo's own numbers.
//!
//! Unsupported constructs (ellipses, quadrants, Boolean nodes, `fasinh` axes, ...) are never guessed:
//! the affected population and its subtree are skipped and named in the warnings.

use crate::{EdgeRule, Gate, GateTree, NodeId};
use sf_core::compensation::{Compensation, Mode};
use sf_core::transforms::Transform;
use sf_core::xml::{self, Element};
use std::collections::{BTreeSet, HashMap};
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct WspError(pub String);

impl fmt::Display for WspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "workspace import: {}", self.0)
    }
}

impl std::error::Error for WspError {}

pub use sf_core::scale::ScaleSpec as TransformSpec;

#[derive(Debug, Clone, PartialEq)]
pub struct CompSpec {
    /// Prefix FlowJo puts on compensated axis names (`Comp-`).
    pub prefix: String,
    pub detectors: Vec<String>,
    /// Row `i` is the spillover of detector `i` into every detector (row-major `n x n`).
    pub spill: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct WspSample {
    pub name: String,
    /// File name (not path) of the FCS file, URL-decoded from the workspace's `DataSet uri`.
    pub file_name: Option<String>,
    pub comp: Option<CompSpec>,
    pub transforms: HashMap<String, TransformSpec>,
    pub tree: GateTree,
    /// FlowJo's saved event count per population (the root holds the sample's total).
    pub flowjo_counts: HashMap<NodeId, u64>,
    /// Populations whose gate FlowJo generated itself (`userDefined="0"`, e.g. its automatic "Size" gate).
    pub auto_gates: BTreeSet<NodeId>,
    pub warnings: Vec<String>,
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_transforms(el: &Element, warnings: &mut Vec<String>) -> (HashMap<String, TransformSpec>, BTreeSet<String>) {
    let mut out = HashMap::new();
    let mut unusable = BTreeSet::new();
    for x in &el.children {
        let Some(param) = x.child("parameter").and_then(|p| p.attr("name")) else { continue };
        let f = |k: &str| x.attr(k).and_then(num);
        let spec = match x.name.as_str() {
            "linear" => f("minRange").zip(f("maxRange")).map(|(min, max)| TransformSpec::Linear { min, max }),
            "log" => f("offset").zip(f("decades")).map(|(offset, decades)| TransformSpec::Log { offset, decades }),
            "biex" => match (f("neg"), f("width"), f("pos"), f("maxRange")) {
                (Some(neg), Some(width), Some(pos), Some(max)) => Some(TransformSpec::Biex { neg, width, pos, max }),
                _ => None,
            },
            "logicle" => match (f("T"), f("W"), f("M"), f("A")) {
                (Some(t), Some(w), Some(m), Some(a)) => Some(TransformSpec::Logicle { t, w, m, a }),
                _ => None,
            },
            other => {
                warnings.push(format!("axis '{param}': transform type '{other}' is not supported (gates on it are skipped)"));
                unusable.insert(param.to_string());
                continue;
            }
        };
        match spec {
            Some(s) => {
                out.insert(param.to_string(), s);
            }
            None => {
                unusable.insert(param.to_string());
                warnings.push(format!("axis '{param}': incomplete {} transform parameters", x.name));
            }
        }
    }
    (out, unusable)
}

fn parse_comp(m: &Element) -> Result<CompSpec, String> {
    let prefix = m.attr("prefix").unwrap_or("Comp-").to_string();
    let detectors: Vec<String> = match m.child("parameters") {
        Some(p) => p.children_named("parameter").filter_map(|e| e.attr("name")).map(str::to_string).collect(),
        None => m.children_named("spillover").filter_map(|e| e.attr("parameter")).map(str::to_string).collect(),
    };
    let n = detectors.len();
    if n == 0 {
        return Err("compensation matrix has no parameters".into());
    }
    let mut spill = vec![f64::NAN; n * n];
    for row in m.children_named("spillover") {
        let rp = row.attr("parameter").ok_or("spillover row without a parameter")?;
        let i = detectors.iter().position(|d| d == rp).ok_or_else(|| format!("spillover row '{rp}' is not a matrix parameter"))?;
        for c in row.children_named("coefficient") {
            let cp = c.attr("parameter").ok_or("coefficient without a parameter")?;
            let j = detectors.iter().position(|d| d == cp).ok_or_else(|| format!("coefficient column '{cp}' is not a matrix parameter"))?;
            spill[i * n + j] = c.attr("value").and_then(num).ok_or_else(|| format!("bad coefficient {rp}/{cp}"))?;
        }
    }
    if spill.iter().any(|v| v.is_nan()) {
        return Err("compensation matrix is incomplete".into());
    }
    Ok(CompSpec { prefix, detectors, spill })
}

struct Ctx<'a> {
    transforms: &'a HashMap<String, TransformSpec>,
    /// Axes whose FlowJo transform we cannot reproduce; a gate on one is refused, never guessed.
    unusable: &'a BTreeSet<String>,
    built: HashMap<String, Box<dyn Transform>>,
}

impl Ctx<'_> {
    fn xf(&mut self, axis: &str) -> Result<Option<&dyn Transform>, String> {
        if self.unusable.contains(axis) {
            return Err(format!("axis '{axis}' uses a transform that is not supported"));
        }
        if !self.built.contains_key(axis) {
            match self.transforms.get(axis) {
                Some(spec) => {
                    let t = spec.build().map_err(|e| format!("axis '{axis}': {e}"))?;
                    self.built.insert(axis.to_string(), t);
                }
                None => return Ok(None),
            }
        }
        Ok(self.built.get(axis).map(|b| b.as_ref()))
    }
}

fn dim_name(d: &Element) -> Option<String> {
    d.child("fcs-dimension").and_then(|f| f.attr("name")).map(str::to_string)
}

/// Convert one FlowJo `<Gate>` element into a gate in transformed space.
fn convert_gate(g: &Element, ctx: &mut Ctx) -> Result<Gate, String> {
    let shape = g.children.iter().find(|c| c.name.ends_with("Gate")).ok_or("no gate shape")?;
    let dims: Vec<Element> = shape.children_named("dimension").cloned().collect();
    match shape.name.as_str() {
        "PolygonGate" => {
            if dims.len() != 2 {
                return Err(format!("polygon needs 2 dimensions (got {})", dims.len()));
            }
            let names = [dim_name(&dims[0]).ok_or("polygon dimension without a name")?, dim_name(&dims[1]).ok_or("polygon dimension without a name")?];
            let mut verts = Vec::new();
            for v in shape.children_named("vertex") {
                let c: Vec<f64> = v.children_named("coordinate").map(|c| c.attr("value").and_then(num).ok_or("bad vertex coordinate")).collect::<Result<_, _>>()?;
                if c.len() != 2 {
                    return Err("vertex needs 2 coordinates".into());
                }
                verts.push((c[0], c[1]));
            }
            let mut out = Vec::with_capacity(verts.len());
            for (x, y) in verts {
                let tx = match ctx.xf(&names[0])? {
                    Some(t) => t.forward(x),
                    None => x,
                };
                let ty = match ctx.xf(&names[1])? {
                    Some(t) => t.forward(y),
                    None => y,
                };
                out.push((tx, ty));
            }
            let mut gate = Gate::polygon(&names[0], &names[1], &out).map_err(|e| e.to_string())?;
            if shape.attr("eventsInside") == Some("0") {
                gate = gate.inverted();
            }
            Ok(gate)
        }
        "RectangleGate" => {
            let open = 1e300;
            let mut axes = Vec::new();
            for d in &dims {
                let name = dim_name(d).ok_or("rectangle dimension without a name")?;
                let t = ctx.xf(&name)?;
                let tf = |v: Option<f64>, default: f64| match v {
                    Some(v) => t.map_or(v, |t| t.forward(v)),
                    None => default,
                };
                let (lo, hi) = (tf(d.attr("min").and_then(num), -open), tf(d.attr("max").and_then(num), open));
                axes.push((name, lo, hi));
            }
            let mut gate = match axes.len() {
                1 => Gate::interval(&axes[0].0, axes[0].1, axes[0].2),
                2 => Gate::rectangle(&axes[0].0, &axes[1].0, axes[0].1, axes[0].2, axes[1].1, axes[1].2),
                n => return Err(format!("rectangle needs 1 or 2 dimensions (got {n})")),
            }
            .map_err(|e| e.to_string())?
            .with_edges(EdgeRule::HalfOpen);
            if shape.attr("eventsInside") == Some("0") {
                gate = gate.inverted();
            }
            Ok(gate)
        }
        other => Err(format!("{other} is not supported")),
    }
}

/// Walk a `SampleNode`/`Population` subtree iteratively (depth must never matter).
fn import_tree(sample_node: &Element, ctx: &mut Ctx, warnings: &mut Vec<String>) -> (GateTree, HashMap<NodeId, u64>, BTreeSet<NodeId>) {
    let mut tree = GateTree::new("All Events");
    let mut counts = HashMap::new();
    let mut auto = BTreeSet::new();
    if let Some(c) = sample_node.attr("count").and_then(|c| c.parse::<u64>().ok()) {
        counts.insert(GateTree::ROOT, c);
    }
    let mut stack: Vec<(&Element, NodeId)> = vec![(sample_node, GateTree::ROOT)];
    while let Some((node, id)) = stack.pop() {
        let Some(subs) = node.child("Subpopulations") else { continue };
        for child in &subs.children {
            if child.name != "Population" {
                warnings.push(format!("'{}': {} nodes are not supported (skipped with their subtree)", child.attr("name").unwrap_or("?"), child.name));
                continue;
            }
            let name = child.attr("name").unwrap_or("").to_string();
            let Some(gate_el) = child.child("Gate") else {
                warnings.push(format!("population '{name}' has no gate (skipped with its subtree)"));
                continue;
            };
            let gate = match convert_gate(gate_el, ctx) {
                Ok(g) => g,
                Err(e) => {
                    warnings.push(format!("population '{name}': {e} (skipped with its subtree)"));
                    continue;
                }
            };
            match tree.add_gate(id, &name, gate) {
                Ok(nid) => {
                    if gate_el.children.iter().any(|s| s.attr("userDefined") == Some("0")) {
                        auto.insert(nid);
                    }
                    if let Some(c) = child.attr("count").and_then(|c| c.parse::<u64>().ok()) {
                        counts.insert(nid, c);
                    }
                    stack.push((child, nid));
                }
                Err(e) => warnings.push(format!("population '{name}': {e} (skipped with its subtree)")),
            }
        }
    }
    (tree, counts, auto)
}

/// Import every sample in a workspace. Returns the samples plus workspace-level warnings.
pub fn parse_wsp(text: &str) -> Result<Vec<WspSample>, WspError> {
    let root = xml::parse(text).map_err(|e| WspError(e.to_string()))?;
    if root.name != "Workspace" {
        return Err(WspError(format!("not a FlowJo workspace (root element is <{}>)", root.name)));
    }
    let list = root.child("SampleList").ok_or_else(|| WspError("workspace has no SampleList".into()))?;
    let mut out = Vec::new();
    for sample in list.children_named("Sample") {
        let mut warnings = Vec::new();
        let Some(node) = sample.child("SampleNode") else {
            warnings.push("sample has no SampleNode".to_string());
            continue;
        };
        let name = node.attr("name").unwrap_or("").to_string();
        let file_name = sample
            .child("DataSet")
            .and_then(|d| d.attr("uri"))
            .map(|u| url_decode(u.rsplit('/').next().unwrap_or(u)));
        let (transforms, unusable) = sample.child("Transformations").map(|t| parse_transforms(t, &mut warnings)).unwrap_or_default();
        let comp = match sample.child("spilloverMatrix") {
            Some(m) => match parse_comp(m) {
                Ok(c) => Some(c),
                Err(e) => {
                    warnings.push(format!("compensation: {e}"));
                    None
                }
            },
            None => None,
        };
        let mut ctx = Ctx { transforms: &transforms, unusable: &unusable, built: HashMap::new() };
        let (tree, flowjo_counts, auto_gates) = import_tree(node, &mut ctx, &mut warnings);
        out.push(WspSample { name, file_name, comp, transforms: transforms.clone(), tree, flowjo_counts, auto_gates, warnings });
    }
    Ok(out)
}

impl WspSample {
    /// Channel names used by the imported gates.
    pub fn gate_channels(&self) -> BTreeSet<String> {
        let mut set = BTreeSet::new();
        for id in self.tree.self_and_descendants(GateTree::ROOT).unwrap() {
            if let Ok(crate::NodeKind::Gate(g)) = self.tree.kind(id) {
                set.insert(g.x_chan.clone());
                if let Some(y) = &g.y_chan {
                    set.insert(y.clone());
                }
            }
        }
        set
    }

    /// Transformed (and, for `Comp-` axes, compensated) columns for every channel the gates use, ready
    /// for [`crate::EventView`]. `events` is row-major `f32` with `channels` as column names.
    pub fn build_view(&self, events: &[f32], channels: &[String]) -> Result<(Vec<f32>, Vec<String>), String> {
        let n_cols = channels.len();
        if n_cols == 0 || events.len() % n_cols != 0 {
            return Err("event matrix does not match its channel list".into());
        }
        let n = events.len() / n_cols;
        let comped = match &self.comp {
            Some(c) => {
                let cols: Vec<usize> = c
                    .detectors
                    .iter()
                    .map(|d| channels.iter().position(|ch| ch == d).ok_or_else(|| format!("compensation detector '{d}' is not in the data file")))
                    .collect::<Result<_, _>>()?;
                let comp = Compensation::new(c.detectors.clone(), c.spill.clone()).map_err(|e| e.to_string())?;
                let mut copy = events.to_vec();
                comp.apply(&mut copy, n_cols, &cols, Mode::Standard).map_err(|e| e.to_string())?;
                Some(copy)
            }
            None => None,
        };
        let names: Vec<String> = self.gate_channels().into_iter().collect();
        let mut view = vec![0f32; n * names.len()];
        for (k, axis) in names.iter().enumerate() {
            let (source, use_comp) = match &self.comp {
                Some(c) if axis.starts_with(&c.prefix) => (&axis[c.prefix.len()..], true),
                _ => (axis.as_str(), false),
            };
            let col = channels.iter().position(|ch| ch == source).ok_or_else(|| format!("axis '{axis}': channel '{source}' is not in the data file"))?;
            let data = if use_comp { comped.as_ref().expect("prefix implies a matrix") } else { events };
            let xf = match self.transforms.get(axis) {
                Some(spec) => Some(spec.build().map_err(|e| format!("axis '{axis}': {e}"))?),
                None => None,
            };
            for r in 0..n {
                let v = data[r * n_cols + col] as f64;
                view[r * names.len() + k] = match &xf {
                    Some(t) => t.forward(v) as f32,
                    None => v as f32,
                };
            }
        }
        Ok((view, names))
    }
}
