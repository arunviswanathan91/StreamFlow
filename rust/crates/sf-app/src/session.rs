//! Application state without any GUI types: loaded samples, one gating tree per sample, cached
//! evaluation, open graph windows, and the operations the UI invokes. Everything here is unit-tested.
//!
//! Invariants kept from the Java app's bug log:
//! * **Golden invariant** — [`Session::open_population`] works for a population at ANY nesting depth; it
//!   only looks the node up in the tree, never walks a bounded path (BUG list: depth guards).
//! * Applying a gate to all samples replaces a same-named sibling instead of stacking a duplicate (BUG-15).
//! * A missing channel in a gate never breaks the rest of the evaluation (BUG-01): it is a warning.
//! * Deleting a population closes the graph windows that were showing it or anything beneath it.

use crate::density::Range;
use crate::drawing::{to_gate, Drawing, Shape, Tool};
use crate::density::PlotKind;
use crate::sample::CompInfo;
use sf_core::compensation::Mode;
use sf_core::json::Json;
use sf_core::scale::ScaleSpec;
use sf_gating::workspace::{workspace_from_json, workspace_to_json, SampleGates};
use sf_gating::{Evaluation, EventView, GateKind, GateTree, NodeId, NodeKind, NodeMeta};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub use crate::sample::LoadedSample;

mod groups;
pub use groups::{Applied, DrawTarget, GateOwner, Group, GroupDelete, ALL_SAMPLES};

/// A short display name for a channel: `Comp-BV421-A` -> `BV421`, `FITC-A` -> `FITC`.
pub fn short_name(channel: &str) -> String {
    let c = channel.strip_prefix(crate::sample::COMP_PREFIX).unwrap_or(channel);
    c.strip_suffix("-A").or_else(|| c.strip_suffix("-H")).or_else(|| c.strip_suffix("-W")).unwrap_or(c).to_string()
}

/// The scale a channel is shown on by default: scatter/time linear, fluorescence logicle.
pub fn default_scale(channel: &str, range: f64) -> ScaleSpec {
    let up = channel.to_ascii_uppercase();
    if up.starts_with("MAP-") || up.contains("FSC") || up.contains("SSC") || up.contains("TIME") || up.contains("WIDTH") || up.ends_with("-W") || up.ends_with("-H") && up.starts_with("FSC") {
        ScaleSpec::Identity
    } else {
        ScaleSpec::Logicle { t: range.max(262_144.0), w: 0.5, m: 4.5, a: 0.0 }
    }
}

/// One undoable state of a sample's gating (tree plus per-population view data).
#[derive(Clone)]
struct Snap {
    tree: GateTree,
    meta: HashMap<NodeId, NodeMeta>,
}

const HISTORY: usize = 60;

struct SampleState {
    tree: GateTree,
    meta: HashMap<NodeId, NodeMeta>,
    version: u64,
    cache: Option<(u64, Evaluation)>,
    undo: Vec<Snap>,
    redo: Vec<Snap>,
}

#[derive(Debug, Clone)]
pub struct GraphWindow {
    pub id: u64,
    pub sample: usize,
    pub node: NodeId,
    pub x_chan: String,
    pub y_chan: String,
    pub x_scale: ScaleSpec,
    pub y_scale: ScaleSpec,
    pub tool: Tool,
    pub drawing: Drawing,
    pub status: String,
    pub kind: PlotKind,
}

/// A fluorescence-minus-one reference level: where the control's signal ends in one channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Fmo {
    /// In raw channel units (the line is drawn at the level's position on whatever scale a plot uses).
    pub level: f64,
    /// Where it came from, e.g. `FMO CD25.fcs · Lymphocytes ▸ CD3+`.
    pub source: String,
    /// Events the percentile was taken over.
    pub n: usize,
    pub percentile: f64,
}

pub struct Session {
    samples: Vec<LoadedSample>,
    states: Vec<SampleState>,
    pub windows: Vec<GraphWindow>,
    next_window: u64,
    pub selected: Option<(usize, NodeId)>,
    /// Messages for the status bar / log (load problems, evaluation warnings...).
    pub log: Vec<String>,
    fmo: BTreeMap<String, Fmo>,
    /// Marker names typed by the user, by channel (`BV711-A` -> `CD45`). An empty name hides the file's own label.
    markers: BTreeMap<String, String>,
    /// How the pooled map attached to the samples (`Map-1` / `Map-2`) was made, in words, for the methods text.
    pub map_info: Option<String>,
    /// The markers the map was made from and their arcsinh cofactor (0 = none): what a map population is learned on.
    pub map_features: Vec<String>,
    pub map_cofactor: f64,
    /// Sample groups (besides the built-in All Samples) and the gates they own.
    pub groups: Vec<Group>,
    pub applied: Vec<Applied>,
    /// Where a newly drawn gate goes.
    pub draw_target: DrawTarget,
    /// Whether gates owned by All Samples move together in every sample.
    pub all_synced: bool,
}

impl Default for Session {
    fn default() -> Self {
        Session::new()
    }
}

impl Session {
    pub fn new() -> Session {
        Session { samples: vec![], states: vec![], windows: vec![], next_window: 1, selected: None, log: vec![], fmo: BTreeMap::new(), markers: BTreeMap::new(), map_info: None, map_features: vec![], map_cofactor: 150.0, groups: vec![], applied: vec![], draw_target: DrawTarget::default(), all_synced: false }
    }

    pub fn samples(&self) -> &[LoadedSample] {
        &self.samples
    }

    pub fn add_sample(&mut self, s: LoadedSample) -> usize {
        self.samples.push(s);
        self.states.push(SampleState { tree: GateTree::new("All Events"), meta: HashMap::new(), version: 0, cache: None, undo: vec![], redo: vec![] });
        self.samples.len() - 1
    }

    pub fn load_fcs(&mut self, path: &Path) -> Result<usize, String> {
        let s = LoadedSample::from_fcs(path)?;
        if let Some(i) = self.samples.iter().position(|x| x.name == s.name) {
            return Err(format!("a sample named '{}' is already loaded (#{i})", s.name));
        }
        Ok(self.add_sample(s))
    }

    /// Bumped by every change to a sample's gating tree; UI caches key on it.
    pub fn version(&self, sample: usize) -> u64 {
        self.states[sample].version
    }

    pub fn tree(&self, sample: usize) -> &GateTree {
        &self.states[sample].tree
    }

    fn touch(&mut self, sample: usize) {
        self.states[sample].version += 1;
        self.states[sample].cache = None;
    }

    /// Evaluate (or reuse) the sample's gating tree. Warnings are appended to the log once per evaluation.
    pub fn evaluation(&mut self, sample: usize) -> Result<&Evaluation, String> {
        let st = &self.states[sample];
        if st.cache.as_ref().map_or(true, |(v, _)| *v != st.version) {
            let s = &self.samples[sample];
            let view = EventView::new(&s.events, s.channels.len(), &s.channels).map_err(|e| e.to_string())?;
            let ev = self.states[sample].tree.evaluate(&view).map_err(|e| e.to_string())?;
            for w in &ev.warnings {
                self.log.push(format!("{}: {} — {}", self.samples[sample].name, w.name, w.message));
            }
            let v = self.states[sample].version;
            self.states[sample].cache = Some((v, ev));
        }
        Ok(&self.states[sample].cache.as_ref().unwrap().1)
    }

    pub fn count(&mut self, sample: usize, node: NodeId) -> usize {
        self.evaluation(sample).ok().and_then(|e| e.counts.get(node).copied()).unwrap_or(0)
    }

    fn default_axes(&self, sample: usize) -> (String, String) {
        let s = &self.samples[sample];
        let pick = |want: &str, fallback: usize| {
            s.channels.iter().find(|c| c.eq_ignore_ascii_case(want)).cloned().or_else(|| s.channels.get(fallback).cloned()).unwrap_or_default()
        };
        (pick("FSC-A", 0), pick("SSC-A", 1.min(s.channels.len().saturating_sub(1))))
    }

    /// Open (or return the existing) graph window for a population. Works at any nesting depth.
    pub fn open_population(&mut self, sample: usize, node: NodeId) -> Result<u64, String> {
        if sample >= self.samples.len() {
            return Err("no such sample".into());
        }
        if !self.states[sample].tree.is_alive(node) {
            return Err("that population no longer exists".into());
        }
        if let Some(w) = self.windows.iter().find(|w| w.sample == sample && w.node == node) {
            return Ok(w.id);
        }
        // remembered view (BUG-09): the axes this population was last shown on
        let remembered = self.states[sample].meta.get(&node).cloned();
        let (dx, dy) = self.default_axes(sample);
        let (x_chan, y_chan) = match &remembered {
            Some(m) => (m.view_x.clone().filter(|c| self.samples[sample].channel_index(c).is_some()).unwrap_or(dx), m.view_y.clone().filter(|c| self.samples[sample].channel_index(c).is_some()).unwrap_or(dy)),
            None => (dx, dy),
        };
        let s = &self.samples[sample];
        let rng = |c: &str| s.channel_index(c).map_or(262_144.0, |i| s.ranges[i]);
        let (x_scale, y_scale) = (default_scale(&x_chan, rng(&x_chan)), default_scale(&y_chan, rng(&y_chan)));
        let id = self.next_window;
        self.next_window += 1;
        self.windows.push(GraphWindow { id, sample, node, x_chan, y_chan, x_scale, y_scale, tool: Tool::Pointer, drawing: Drawing::default(), status: String::new(), kind: PlotKind::Pseudocolor });
        self.selected = Some((sample, node));
        Ok(id)
    }

    pub fn close_window(&mut self, id: u64) {
        self.windows.retain(|w| w.id != id);
    }

    pub fn window(&self, id: u64) -> Option<&GraphWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    pub fn window_mut(&mut self, id: u64) -> Option<&mut GraphWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Remember the axes a window is showing on its population (so reopening restores them).
    pub fn remember_view(&mut self, id: u64) {
        if let Some(w) = self.window(id).cloned() {
            let m = self.states[w.sample].meta.entry(w.node).or_default();
            m.view_x = Some(w.x_chan);
            m.view_y = Some(w.y_chan);
            m.view_x_scale = Some(w.x_scale.to_text());
            m.view_y_scale = Some(w.y_scale.to_text());
        }
    }

    /// Display-space points of the window's population on its current axes.
    pub fn window_points(&mut self, id: u64) -> Result<Vec<(f64, f64)>, String> {
        let w = self.window(id).ok_or("no such window")?.clone();
        self.population_points(w.sample, w.node, &w.x_chan, &w.y_chan, &w.x_scale, &w.y_scale)
    }

    /// Display-space points of any population on any axes (used by graph windows and overview thumbnails).
    pub fn population_points(&mut self, sample: usize, node: NodeId, x_chan: &str, y_chan: &str, xs: &ScaleSpec, ys: &ScaleSpec) -> Result<Vec<(f64, f64)>, String> {
        let ev = self.evaluation(sample)?;
        let rows = ev.masks.get(node).and_then(|m| m.as_ref()).ok_or("population has no events")?.indices();
        let s = &self.samples[sample];
        let xi = s.channel_index(x_chan).ok_or_else(|| format!("channel '{x_chan}' is not in this sample"))?;
        let yi = s.channel_index(y_chan).ok_or_else(|| format!("channel '{y_chan}' is not in this sample"))?;
        let (tx, ty) = (xs.build()?, ys.build()?);
        let d = s.channels.len();
        Ok(rows.into_iter().map(|r| (tx.forward(s.events[r * d + xi] as f64), ty.forward(s.events[r * d + yi] as f64))).collect())
    }

    /// Raw values of one channel for the events of a population.
    pub fn population_values(&mut self, sample: usize, node: NodeId, channel: &str) -> Result<Vec<f64>, String> {
        let ev = self.evaluation(sample)?;
        let rows = ev.masks.get(node).and_then(|m| m.as_ref()).ok_or("population has no events")?.indices();
        let s = &self.samples[sample];
        let c = s.channel_index(channel).ok_or_else(|| format!("channel '{channel}' is not in this sample"))?;
        let d = s.channels.len();
        Ok(rows.into_iter().map(|r| s.events[r * d + c] as f64).collect())
    }

    /// `All Events > Lymphocytes > CD3+` as (name, node) pairs, root first. Iterative.
    pub fn breadcrumb(&self, sample: usize, node: NodeId) -> Vec<(String, NodeId)> {
        let t = &self.states[sample].tree;
        let mut out = Vec::new();
        let mut cur = Some(node);
        while let Some(n) = cur {
            out.push((if n == GateTree::ROOT { "All Events".to_string() } else { t.name(n).unwrap_or("?").to_string() }, n));
            cur = t.parent(n).ok().flatten();
        }
        out.reverse();
        out
    }

    /// Show the same population (matched by path) in another sample, keeping the window and its axes.
    pub fn retarget_window(&mut self, id: u64, sample: usize) -> Result<(), String> {
        if sample >= self.samples.len() {
            return Err("no such sample".into());
        }
        let w = self.window(id).ok_or("no such window")?.clone();
        let path = self.states[w.sample].tree.path(w.node).map_err(|e| e.to_string())?;
        let refs: Vec<&str> = path.iter().map(String::as_str).collect();
        let node = if refs.is_empty() { Some(GateTree::ROOT) } else { self.states[sample].tree.find(&refs) };
        let node = node.ok_or_else(|| format!("{} has no population '{}'", self.samples[sample].name, path.join(" > ")))?;
        if let Some(other) = self.windows.iter().find(|o| o.id != id && o.sample == sample && o.node == node) {
            return Err(format!("that population is already open in another tab (#{})", other.id));
        }
        let win = self.window_mut(id).unwrap();
        win.sample = sample;
        win.node = node;
        win.drawing.cancel();
        self.selected = Some((sample, node));
        Ok(())
    }

    /// A display-space range that shows the whole population on the window's axes (1% padding).
    pub fn auto_range(&mut self, id: u64) -> Option<Range> {
        let pts = self.window_points(id).ok()?;
        let finite: Vec<&(f64, f64)> = pts.iter().filter(|p| p.0.is_finite() && p.1.is_finite()).collect();
        if finite.is_empty() {
            return None;
        }
        let (x0, x1) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (y0, y1) = finite.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        let pad = |lo: f64, hi: f64| {
            let span = if hi > lo { hi - lo } else { 1.0 };
            (lo - 0.01 * span, hi + 0.01 * span)
        };
        let ((x0, x1), (y0, y1)) = (pad(x0, x1), pad(y0, y1));
        Some(Range { x0, x1, y0, y1 })
    }

    /// Add a gate drawn in a window as a child of that window's population.
    pub fn commit_shape(&mut self, id: u64, shape: &Shape, name: &str) -> Result<NodeId, String> {
        let w = self.window(id).ok_or("no such window")?.clone();
        let gate = to_gate(shape, &w.x_chan, &w.y_chan, &w.x_scale, &w.y_scale)?;
        self.snapshot(w.sample);
        let nid = match self.states[w.sample].tree.add_gate(w.node, name, gate) {
            Ok(n) => n,
            Err(e) => {
                self.discard_snapshot(w.sample); // nothing changed, so there is nothing to undo
                return Err(e.to_string());
            }
        };
        self.touch(w.sample);
        self.apply_new_gate(w.sample, nid);
        Ok(nid)
    }


    /// Move or resize a gate: replace its geometry as ONE undoable step. The gate keeps its name and everything
    /// gated below it. Returns the population's new event count.
    pub fn edit_gate(&mut self, sample: usize, node: NodeId, gate: sf_gating::Gate) -> Result<usize, String> {
        self.snapshot(sample);
        if let Err(e) = self.states[sample].tree.set_gate(node, gate) {
            self.discard_snapshot(sample);
            return Err(e.to_string());
        }
        self.touch(sample);
        self.propagate_if_synced(sample, node);
        Ok(self.count(sample, node))
    }
    /// A unique default name under `parent`: "Population 1", "Population 2", ...
    pub fn next_name(&self, sample: usize, parent: NodeId) -> String {
        let t = &self.states[sample].tree;
        let taken: Vec<String> = t.children(parent).map(|c| c.iter().filter_map(|&k| t.name(k).ok().map(str::to_string)).collect()).unwrap_or_default();
        (1..).map(|i| format!("Population {i}")).find(|n| !taken.contains(n)).unwrap()
    }

    pub fn rename(&mut self, sample: usize, node: NodeId, new_name: &str) -> Result<(), String> {
        if self.gate_owner(sample, node).is_some() {
            return self.rename_group_owned(sample, node, new_name);
        }
        let t = &mut self.states[sample].tree;
        let parent = t.parent(node).map_err(|e| e.to_string())?.ok_or("the root cannot be renamed")?;
        if new_name.trim().is_empty() {
            return Err("a population needs a name".into());
        }
        if t.children(parent).map_err(|e| e.to_string())?.iter().any(|&c| c != node && t.name(c).map_or(false, |n| n == new_name)) {
            return Err(format!("'{new_name}' already exists at this level"));
        }
        let _ = t;
        self.snapshot(sample);
        self.states[sample].tree.rename(node, new_name).map_err(|e| e.to_string())?;
        self.touch(sample);
        Ok(())
    }

    /// Delete a population and everything beneath it; windows showing any of them are closed.
    pub fn delete(&mut self, sample: usize, node: NodeId) -> Result<usize, String> {
        if let Some(o) = self.gate_owner(sample, node) {
            return Err(format!("This gate is owned by the group '{}'. Delete it from the group (right-click ▸ Delete from group…) or release it to the samples first.", o.group));
        }
        self.delete_unchecked(sample, node)
    }

    /// Delete without the group-ownership check (used when deleting from the group itself).
    pub(crate) fn delete_unchecked(&mut self, sample: usize, node: NodeId) -> Result<usize, String> {
        let doomed = self.states[sample].tree.self_and_descendants(node).map_err(|e| e.to_string())?;
        self.snapshot(sample);
        let n = match self.states[sample].tree.remove(node) {
            Ok(n) => n,
            Err(e) => {
                self.discard_snapshot(sample);
                return Err(e.to_string());
            }
        };
        self.windows.retain(|w| !(w.sample == sample && doomed.contains(&w.node)));
        if self.selected.map_or(false, |(s, nd)| s == sample && doomed.contains(&nd)) {
            self.selected = None;
        }
        self.states[sample].meta.retain(|k, _| !doomed.contains(k));
        self.touch(sample);
        self.prune_applied();
        Ok(n)
    }

    /// Copy a population's gate to every other sample at the same path. A same-named sibling is REPLACED, so
    /// running it twice leaves one copy (BUG-15). Returns how many samples received it; samples that lack
    /// the parent path are skipped with a log message.
    pub fn apply_to_all(&mut self, sample: usize, node: NodeId) -> Result<usize, String> {
        self.apply_to_group(sample, node, ALL_SAMPLES)
    }

    // ---- undo / redo ----------------------------------------------------------------------------

    /// Record the sample's current gating so the next change can be undone. Any redo history is dropped.
    fn snapshot(&mut self, sample: usize) {
        let st = &mut self.states[sample];
        st.undo.push(Snap { tree: st.tree.clone(), meta: st.meta.clone() });
        if st.undo.len() > HISTORY {
            st.undo.remove(0);
        }
        st.redo.clear();
    }

    /// Drop the snapshot taken for an operation that then failed without changing anything.
    fn discard_snapshot(&mut self, sample: usize) {
        self.states[sample].undo.pop();
    }

    pub fn can_undo(&self, sample: usize) -> bool {
        self.states.get(sample).map_or(false, |s| !s.undo.is_empty())
    }

    pub fn can_redo(&self, sample: usize) -> bool {
        self.states.get(sample).map_or(false, |s| !s.redo.is_empty())
    }

    fn restore(&mut self, sample: usize, snap: Snap) {
        self.states[sample].tree = snap.tree;
        self.states[sample].meta = snap.meta;
        // windows whose population no longer exists in the restored state are closed
        let alive: Vec<NodeId> = (0..self.states[sample].tree.id_bound()).filter(|&n| self.states[sample].tree.is_alive(n)).collect();
        self.windows.retain(|w| w.sample != sample || alive.contains(&w.node));
        if self.selected.map_or(false, |(s, n)| s == sample && !alive.contains(&n)) {
            self.selected = None;
        }
        self.touch(sample);
    }

    // ---- gate operations from the right-click menus ------------------------------------------------

    /// The gate of a population, if it is a real gate (not the root, a Boolean or a quadrant sector).
    pub fn gate_of(&self, sample: usize, node: NodeId) -> Option<sf_gating::Gate> {
        match self.states.get(sample)?.tree.kind(node) {
            Ok(NodeKind::Gate(g)) => Some(g.clone()),
            _ => None,
        }
    }

    /// Turn a rectangle or an ellipse into the polygon with the same outline (an ellipse becomes a 48-sided one),
    /// so its corners can then be moved one by one. One undo step; the population and what is gated below it stay.
    pub fn convert_to_polygon(&mut self, sample: usize, node: NodeId) -> Result<usize, String> {
        let g = self.gate_of(sample, node).ok_or("That population is not a gate")?;
        let (xs, ys) = match g.kind {
            GateKind::Rectangle => {
                let (x0, x1, y0, y1) = (g.xs.iter().cloned().fold(f64::MAX, f64::min), g.xs.iter().cloned().fold(f64::MIN, f64::max), g.ys.iter().cloned().fold(f64::MAX, f64::min), g.ys.iter().cloned().fold(f64::MIN, f64::max));
                (vec![x0, x1, x1, x0], vec![y0, y0, y1, y1])
            }
            GateKind::Ellipse => {
                let (x0, x1, y0, y1) = (g.xs.iter().cloned().fold(f64::MAX, f64::min), g.xs.iter().cloned().fold(f64::MIN, f64::max), g.ys.iter().cloned().fold(f64::MAX, f64::min), g.ys.iter().cloned().fold(f64::MIN, f64::max));
                let (cx, cy, rx, ry) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0, (x1 - x0) / 2.0, (y1 - y0) / 2.0);
                let (c, s) = (g.angle.cos(), g.angle.sin());
                (0..48)
                    .map(|k| {
                        let t = k as f64 * std::f64::consts::TAU / 48.0;
                        let (dx, dy) = (rx * t.cos(), ry * t.sin());
                        (cx + dx * c - dy * s, cy + dx * s + dy * c)
                    })
                    .unzip()
            }
            GateKind::Polygon => return Err("It is already a polygon".into()),
            _ => return Err("Only rectangle and ellipse gates can be converted to a polygon".into()),
        };
        let mut poly = sf_gating::Gate::new(GateKind::Polygon, &g.x_chan, g.y_chan.as_deref(), xs, ys).map_err(|e| e.to_string())?;
        poly.invert = g.invert;
        poly.x_scale = g.x_scale.clone();
        poly.y_scale = g.y_scale.clone();
        self.snapshot(sample);
        if let Err(e) = self.states[sample].tree.convert_to_polygon(node, poly) {
            self.discard_snapshot(sample);
            return Err(e.to_string());
        }
        self.touch(sample);
        self.propagate_if_synced(sample, node);
        Ok(self.count(sample, node))
    }

    /// Add a corner to a polygon gate, on the edge nearest to `at` (display-space coordinates). One undo step.
    pub fn add_polygon_node(&mut self, sample: usize, node: NodeId, at: (f64, f64)) -> Result<usize, String> {
        let mut g = self.gate_of(sample, node).ok_or("That population is not a gate")?;
        if !matches!(g.kind, GateKind::Polygon) {
            return Err("Only polygon gates have corners to add".into());
        }
        let n = g.xs.len();
        let mut best = (f64::MAX, 0usize, (0.0, 0.0));
        for i in 0..n {
            let (a, b) = ((g.xs[i], g.ys[i]), (g.xs[(i + 1) % n], g.ys[(i + 1) % n]));
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let l2 = dx * dx + dy * dy;
            let t = if l2 > 0.0 { (((at.0 - a.0) * dx + (at.1 - a.1) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
            let p = (a.0 + t * dx, a.1 + t * dy);
            let d = (p.0 - at.0).powi(2) + (p.1 - at.1).powi(2);
            if d < best.0 {
                best = (d, i, p);
            }
        }
        g.xs.insert(best.1 + 1, best.2 .0);
        g.ys.insert(best.1 + 1, best.2 .1);
        self.edit_gate(sample, node, g)
    }

    /// Make the population an exclusion gate (everything the gate does NOT contain) or undo that. One undo step.
    pub fn toggle_exclusion(&mut self, sample: usize, node: NodeId) -> Result<bool, String> {
        let g = self.gate_of(sample, node).ok_or("That population is not a gate")?;
        let now = !g.invert;
        self.edit_gate(sample, node, g.inverted())?;
        Ok(now)
    }

    /// Add a copy of `gate` under `parent` of `sample` (the pasted gate keeps its channels and scales). One undo step.
    pub fn paste_gate(&mut self, sample: usize, parent: NodeId, gate: sf_gating::Gate, name: &str) -> Result<NodeId, String> {
        if matches!(gate.kind, GateKind::Indexed(_)) {
            return Err("Subsample and plugin populations hold event numbers of one file and cannot be pasted".into());
        }
        if !self.states[sample].tree.is_alive(parent) {
            return Err("That population no longer exists".into());
        }
        let have = self.sample(sample).channels.clone();
        for c in std::iter::once(&gate.x_chan).chain(gate.y_chan.iter()) {
            if !have.contains(c) {
                return Err(format!("This file has no channel '{c}', so the gate cannot be pasted here"));
            }
        }
        self.snapshot(sample);
        match self.states[sample].tree.replace_gate(parent, name, gate) {
            Ok(n) => {
                self.touch(sample);
                Ok(n)
            }
            Err(e) => {
                self.discard_snapshot(sample);
                Err(e.to_string())
            }
        }
    }

    /// A random subsample of `n` events of a population, as a new population named `name` (seeded, so the same
    /// request gives the same events). One undo step.
    pub fn subsample(&mut self, sample: usize, node: NodeId, n: usize, seed: u64, name: &str) -> Result<NodeId, String> {
        if n == 0 {
            return Err("Choose at least one event".into());
        }
        let rows: Vec<usize> = {
            let ev = self.evaluation(sample)?;
            ev.masks.get(node).and_then(|m| m.as_ref()).ok_or("That population no longer exists")?.indices()
        };
        if n >= rows.len() {
            return Err(format!("The population has only {} events; choose fewer", rows.len()));
        }
        let pick = sf_core::rng::Rng::new(seed).choose_sorted(rows.len(), n);
        let ids: Vec<u32> = pick.into_iter().map(|i| rows[i] as u32).collect();
        let made = self.add_index_populations(sample, node, vec![(name.to_string(), ids)])?;
        Ok(made[0])
    }

    /// Swap a window's X and Y axes (channels and scales). Gates drawn on the old axes stay put and are hidden while the
    /// plot shows other axes. A histogram has only one axis, so there is nothing to swap.
    pub fn transpose_axes(&mut self, id: u64) -> Result<(), String> {
        let w = self.window_mut(id).ok_or("No such plot")?;
        if w.kind == PlotKind::Histogram {
            return Err("A histogram has one axis; switch to a 2-D plot to transpose".into());
        }
        std::mem::swap(&mut w.x_chan, &mut w.y_chan);
        std::mem::swap(&mut w.x_scale, &mut w.y_scale);
        w.drawing.cancel();
        Ok(())
    }

    /// Move one axis to the next (`dir > 0`) or previous channel of the sample, wrapping round, and give it that
    /// channel's default scale. Time-like channels are not skipped: the list is the file's own order.
    pub fn step_axis(&mut self, id: u64, x_axis: bool, dir: i32) -> Result<String, String> {
        let sample = self.window(id).ok_or("No such plot")?.sample;
        let (channels, ranges) = {
            let s = self.sample(sample);
            (s.channels.clone(), s.ranges.clone())
        };
        if channels.is_empty() {
            return Err("This file has no channels".into());
        }
        let w = self.window_mut(id).ok_or("No such plot")?;
        if !x_axis && w.kind == PlotKind::Histogram {
            return Err("A histogram has no Y parameter".into());
        }
        let cur = if x_axis { &w.x_chan } else { &w.y_chan };
        let i = channels.iter().position(|c| c == cur).unwrap_or(0) as i64;
        let n = channels.len() as i64;
        let j = ((i + dir.signum() as i64) % n + n) % n;
        let name = channels[j as usize].clone();
        let scale = default_scale(&name, ranges[j as usize]);
        if x_axis {
            w.x_chan = name.clone();
            w.x_scale = scale;
        } else {
            w.y_chan = name.clone();
            w.y_scale = scale;
        }
        w.drawing.cancel();
        Ok(name)
    }

    pub fn undo(&mut self, sample: usize) -> bool {
        let Some(prev) = self.states.get_mut(sample).and_then(|s| s.undo.pop()) else { return false };
        let st = &mut self.states[sample];
        st.redo.push(Snap { tree: st.tree.clone(), meta: st.meta.clone() });
        self.restore(sample, prev);
        true
    }

    pub fn redo(&mut self, sample: usize) -> bool {
        let Some(next) = self.states.get_mut(sample).and_then(|s| s.redo.pop()) else { return false };
        let st = &mut self.states[sample];
        st.undo.push(Snap { tree: st.tree.clone(), meta: st.meta.clone() });
        self.restore(sample, next);
        true
    }

    /// Set the events of the index-set population `name` under `parent`, as one undoable step. An existing one is
    /// updated IN PLACE, so the gates drawn below it stay; otherwise it is added.
    pub fn set_index_population(&mut self, sample: usize, parent: NodeId, name: &str, rows: Vec<u32>) -> Result<NodeId, String> {
        if !self.states[sample].tree.is_alive(parent) {
            return Err("that population no longer exists".into());
        }
        let t = &self.states[sample].tree;
        let old = t.children(parent).map_err(|e| e.to_string())?.iter().copied().find(|&k| t.name(k).ok() == Some(name));
        self.snapshot(sample);
        let gate = sf_gating::Gate::indexed(rows);
        let r = match old {
            Some(k) => self.states[sample].tree.reset_gate(k, gate).map(|_| k),
            None => self.states[sample].tree.add_gate(parent, name, gate),
        };
        match r {
            Ok(n) => {
                self.touch(sample);
                Ok(n)
            }
            Err(e) => {
                self.discard_snapshot(sample);
                Err(e.to_string())
            }
        }
    }

    /// Add several index-set populations under `parent` as ONE undoable step; same-named siblings are replaced
    /// (never stacked).
    pub fn add_index_populations(&mut self, sample: usize, parent: NodeId, groups: Vec<(String, Vec<u32>)>) -> Result<Vec<NodeId>, String> {
        if !self.states[sample].tree.is_alive(parent) {
            return Err("that population no longer exists".into());
        }
        self.snapshot(sample);
        let mut ids = Vec::new();
        for (name, rows) in groups {
            match self.states[sample].tree.replace_gate(parent, &name, sf_gating::Gate::indexed(rows)) {
                Ok(n) => ids.push(n),
                Err(e) => {
                    self.discard_snapshot(sample);
                    return Err(e.to_string());
                }
            }
        }
        self.touch(sample);
        Ok(ids)
    }

    // ---- quadrant gates -------------------------------------------------------------------------

    /// A quadrant gate: four populations around `(cx, cy)` in the window's display space, named after the
    /// channels (`CD3+ CD4-` ...). All four are added together or none are.
    pub fn commit_quadrants(&mut self, id: u64, cx: f64, cy: f64) -> Result<Vec<NodeId>, String> {
        let w = self.window(id).ok_or("no such window")?.clone();
        let (x, y) = (short_name(&w.x_chan), short_name(&w.y_chan));
        let specs = [(sf_gating::Quad::Q1, format!("{x}+ {y}+")), (sf_gating::Quad::Q2, format!("{x}- {y}+")), (sf_gating::Quad::Q3, format!("{x}- {y}-")), (sf_gating::Quad::Q4, format!("{x}+ {y}-"))];
        let existing: Vec<String> = self.states[w.sample].tree.children(w.node).map_err(|e| e.to_string())?.iter().filter_map(|&c| self.states[w.sample].tree.name(c).ok().map(str::to_string)).collect();
        if let Some((_, n)) = specs.iter().find(|(_, n)| existing.contains(n)) {
            return Err(format!("'{n}' already exists at this level"));
        }
        self.snapshot(w.sample);
        let mut ids = Vec::new();
        for (q, name) in specs {
            let g = sf_gating::Gate::quadrant(&w.x_chan, &w.y_chan, q, cx, cy).map_err(|e| e.to_string())?.with_scales(w.x_scale.clone(), w.y_scale.clone());
            match self.states[w.sample].tree.add_gate(w.node, &name, g) {
                Ok(n) => ids.push(n),
                Err(e) => {
                    // all four or none: put the tree back exactly as it was
                    self.undo(w.sample);
                    self.states[w.sample].redo.clear();
                    return Err(e.to_string());
                }
            }
        }
        self.touch(w.sample);
        for &n in &ids {
            self.apply_new_gate(w.sample, n);
        }
        Ok(ids)
    }

    /// The populations made together with `node` by one quadrant gate (itself included): the quadrant sectors under
    /// the same parent that share its axes, scales and crosshair.
    pub fn quadrant_set(&self, sample: usize, node: NodeId) -> Vec<NodeId> {
        let t = &self.states[sample].tree;
        let Some(g) = self.gate_of(sample, node).filter(|g| matches!(g.kind, GateKind::Quadrant(_)) && g.xs.len() == 1) else { return vec![] };
        let Some(parent) = t.parent(node).ok().flatten() else { return vec![] };
        t.children(parent)
            .map(|c| c.to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|&k| matches!(t.kind(k), Ok(NodeKind::Gate(h)) if matches!(h.kind, GateKind::Quadrant(_)) && h.xs.len() == 1 && h.x_chan == g.x_chan && h.y_chan == g.y_chan && h.x_scale == g.x_scale && h.y_scale == g.y_scale && h.xs[0] == g.xs[0] && h.ys[0] == g.ys[0]))
            .collect()
    }

    /// Move a quadrant gate's crosshair to `(cx, cy)` (display space). The four populations it made move together,
    /// as ONE undoable step, and keep their names and everything gated below them. Returns how many moved.
    pub fn move_quadrants(&mut self, sample: usize, node: NodeId, cx: f64, cy: f64) -> Result<usize, String> {
        if !(cx.is_finite() && cy.is_finite()) {
            return Err("The crosshair must stay on the plot".into());
        }
        let set = self.quadrant_set(sample, node);
        if set.is_empty() {
            return Err("That population is not part of a quadrant gate".into());
        }
        self.snapshot(sample);
        for &k in &set {
            let Some(mut g) = self.gate_of(sample, k) else { continue };
            g.xs[0] = cx;
            g.ys[0] = cy;
            if let Err(e) = self.states[sample].tree.set_gate(k, g) {
                // all or none: put the tree back
                self.undo(sample);
                self.states[sample].redo.clear();
                return Err(e.to_string());
            }
        }
        self.touch(sample);
        for &k in &set {
            self.propagate_if_synced(sample, k);
        }
        Ok(set.len())
    }

    // ---- compensation ---------------------------------------------------------------------------

    /// Apply a spillover matrix to one sample. Gates and windows that use `Comp-` channels follow.
    pub fn set_compensation(&mut self, sample: usize, info: &CompInfo, mode: Mode) -> Result<(), String> {
        self.samples[sample].set_compensation(info, mode)?;
        self.touch(sample);
        Ok(())
    }

    /// Attach `Unmix-` channels (`data[c][event]`) to a sample, replacing earlier ones. Gates and axes on them follow.
    pub fn set_unmixed(&mut self, sample: usize, names: Vec<String>, data: Vec<Vec<f32>>, range: f64) -> Result<(), String> {
        self.samples[sample].set_unmixed(names, data, range)?;
        self.touch(sample);
        Ok(())
    }

    /// Attach a pooled map's coordinates to a sample as `Map-1` / `Map-2` (see [`LoadedSample::set_map`]).
    pub fn set_map(&mut self, sample: usize, x: Vec<f32>, y: Vec<f32>, range: f64) -> Result<(), String> {
        self.samples[sample].set_map(x, y, range)?;
        self.touch(sample);
        Ok(())
    }

    pub fn clear_map(&mut self, sample: usize) {
        self.samples[sample].clear_map();
        self.touch(sample);
    }

    pub fn clear_unmixed(&mut self, sample: usize) {
        self.samples[sample].clear_unmixed();
        self.touch(sample);
    }

    pub fn clear_compensation(&mut self, sample: usize) {
        self.samples[sample].clear_compensation();
        self.touch(sample);
    }

    /// Apply the same matrix to every sample that has all its detectors; returns the names of the samples
    /// it could not be applied to.
    pub fn set_compensation_all(&mut self, info: &CompInfo, mode: Mode) -> Vec<String> {
        let mut failed = Vec::new();
        for i in 0..self.samples.len() {
            if let Err(e) = self.set_compensation(i, info, mode) {
                failed.push(format!("{}: {e}", self.samples[i].name));
            }
        }
        failed
    }

    /// Mutable access to the raw samples is deliberately not offered; modules read through this.
    pub fn sample(&self, i: usize) -> &LoadedSample {
        &self.samples[i]
    }

    // ---- FMO reference levels --------------------------------------------------------------------

    /// The marker on a channel: the name the user gave it, else the label the FCS files carry (`$PnS`).
    /// Compensated channels share the name of the detector they come from.
    pub fn marker(&self, channel: &str) -> Option<String> {
        let base = channel.strip_prefix(crate::sample::COMP_PREFIX).unwrap_or(channel);
        for key in [channel, base] {
            if let Some(m) = self.markers.get(key) {
                return Some(m.trim().to_string()).filter(|m| !m.is_empty());
            }
        }
        for key in [channel, base] {
            for s in &self.samples {
                if let Some(i) = s.channels.iter().position(|c| c == key) {
                    let l = s.labels.get(i).map(|l| l.trim()).unwrap_or("");
                    if !l.is_empty() && !l.eq_ignore_ascii_case(key) {
                        return Some(l.to_string());
                    }
                }
            }
        }
        None
    }

    /// What the user typed for a channel, if anything (see [`Session::marker`] for what is shown).
    pub fn marker_override(&self, channel: &str) -> Option<&str> {
        self.markers.get(channel).map(|s| s.as_str())
    }

    /// Name a channel's marker for the whole session. `None` goes back to the label in the FCS files.
    pub fn set_marker(&mut self, channel: &str, name: Option<&str>) {
        match name {
            Some(n) => self.markers.insert(channel.to_string(), n.trim().to_string()),
            None => self.markers.remove(channel),
        };
    }

    /// A channel as it is shown on axes and in menus: `BV711-A :: CD45`, or just the channel.
    pub fn channel_label(&self, channel: &str) -> String {
        match self.marker(channel) {
            Some(m) => format!("{channel} :: {m}"),
            None => channel.to_string(),
        }
    }

    pub fn fmo(&self, channel: &str) -> Option<&Fmo> {
        self.fmo.get(channel)
    }

    pub fn fmo_all(&self) -> &BTreeMap<String, Fmo> {
        &self.fmo
    }

    pub fn clear_fmo(&mut self, channel: &str) {
        self.fmo.remove(channel);
    }

    /// Move an existing level (dragging its line); the source and percentile it was set with are kept.
    pub fn move_fmo(&mut self, channel: &str, level: f64) {
        if let (Some(f), true) = (self.fmo.get_mut(channel), level.is_finite()) {
            f.level = level;
        }
    }

    /// Record the `percentile` of the window's population as the FMO level of the channel(s) it shows (the Y channel
    /// only when the plot is not a histogram). Returns `(channel, level)` for each one set.
    pub fn set_fmo_from_window(&mut self, id: u64, percentile: f64) -> Result<Vec<(String, f64)>, String> {
        let w = self.window(id).ok_or("no such window")?.clone();
        if !(0.0..=100.0).contains(&percentile) {
            return Err("the percentile must be between 0 and 100".into());
        }
        let mut channels = vec![w.x_chan.clone()];
        if w.kind != PlotKind::Histogram && w.y_chan != w.x_chan {
            channels.push(w.y_chan.clone());
        }
        let source = format!("{} · {}", self.samples[w.sample].name, self.states[w.sample].tree.path(w.node).map(|p| if p.is_empty() { "All Events".to_string() } else { p.join(" ▸ ") }).unwrap_or_default());
        let mut done = Vec::new();
        for ch in channels {
            let vals: Vec<f64> = self.population_values(w.sample, w.node, &ch)?.into_iter().filter(|v| v.is_finite()).collect();
            if vals.is_empty() {
                return Err(format!("there are no events with a value for {ch} in this population"));
            }
            let level = sf_core::stats::percentile(&vals, percentile);
            self.fmo.insert(ch.clone(), Fmo { level, source: source.clone(), n: vals.len(), percentile });
            done.push((ch, level));
        }
        Ok(done)
    }

    /// Fraction (0..=1) of a population's events at or above the channel's FMO level.
    pub fn fraction_above_fmo(&mut self, sample: usize, node: NodeId, channel: &str) -> Option<f64> {
        let level = self.fmo.get(channel)?.level;
        let vals = self.population_values(sample, node, channel).ok()?;
        let finite: Vec<f64> = vals.into_iter().filter(|v| v.is_finite()).collect();
        (!finite.is_empty()).then(|| finite.iter().filter(|&&v| v >= level).count() as f64 / finite.len() as f64)
    }

    // ---- workspace persistence ------------------------------------------------------------------

    pub fn to_workspace_json(&self) -> Json {
        let mut gates = BTreeMap::new();
        for (s, st) in self.samples.iter().zip(&self.states) {
            gates.insert(s.name.clone(), SampleGates { tree: st.tree.clone(), meta: st.meta.clone() });
        }
        let samples: Vec<Json> = self
            .samples
            .iter()
            .map(|s| Json::obj().with("name", Json::Str(s.name.clone())).with("file", s.path.as_ref().map_or(Json::Null, |p| Json::Str(p.display().to_string()))))
            .collect();
        let mut comp = Json::obj();
        for s in &self.samples {
            if let Some(c) = &s.comp {
                let det = Json::Arr(c.detectors.iter().map(|d| Json::Str(d.clone())).collect());
                let sp = Json::Arr(c.spill.iter().map(|v| Json::num(*v)).collect());
                comp = comp.with(&s.name, Json::obj().with("detectors", det).with("spill", sp).with("source", Json::Str(c.source.clone())));
            }
        }
        let mut fmo = Json::obj();
        for (ch, f) in &self.fmo {
            fmo = fmo.with(ch, Json::obj().with("level", Json::num(f.level)).with("source", Json::Str(f.source.clone())).with("n", Json::num(f.n as f64)).with("percentile", Json::num(f.percentile)));
        }
        // the pooled map: each sample's pooled events and their positions, so map gates reopen exactly
        let mut maps = Json::obj();
        for s in &self.samples {
            let (Some(cx), Some(cy)) = (s.channel_index("Map-1"), s.channel_index("Map-2")) else { continue };
            let d = s.channels.len();
            let rows: Vec<usize> = (0..s.n_events).filter(|&r| s.events[r * d + cx].is_finite() && s.events[r * d + cy].is_finite()).collect();
            let col = |c: usize| Json::Arr(rows.iter().map(|&r| Json::num(s.events[r * d + c] as f64)).collect());
            maps = maps.with(&s.name, Json::obj().with("rows", Json::Arr(rows.iter().map(|&r| Json::num(r as f64)).collect())).with("x", col(cx)).with("y", col(cy)).with("range", Json::num(s.ranges[cx])));
        }
        let mut markers = Json::obj();
        for (ch, m) in &self.markers {
            markers = markers.with(ch, Json::Str(m.clone()));
        }
        Json::obj().with("format", Json::Str("streamflow-rust-1".into())).with("samples", Json::Arr(samples)).with("gates", workspace_to_json(&gates)).with("compensation", comp).with("fmo", fmo).with("markers", markers).with("maps", maps).with("map_info", self.map_info.clone().map_or(Json::Null, Json::Str)).with("map_features", Json::Arr(self.map_features.iter().map(|f| Json::Str(f.clone())).collect())).with("map_cofactor", Json::num(self.map_cofactor)).with("groups", self.groups_to_json())
    }

    /// Replace the gating trees of the currently loaded samples with those in a saved workspace (matched by
    /// sample name). Returns warnings: gates for samples that are not loaded, and any repair the loader made.
    pub fn apply_workspace_json(&mut self, doc: &Json) -> Result<Vec<String>, String> {
        let gates = doc.get("gates").ok_or("this file has no 'gates' section")?;
        let mut notes = Vec::new();
        if let Some(Json::Obj(comp)) = doc.get("compensation") {
            for (name, c) in comp {
                let Some(i) = self.samples.iter().position(|s| &s.name == name) else { continue };
                let detectors: Option<Vec<String>> = c.get("detectors").and_then(Json::as_arr).map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect());
                let spill: Option<Vec<f64>> = c.get("spill").and_then(Json::as_arr).map(|a| a.iter().filter_map(Json::as_f64).collect());
                match (detectors, spill) {
                    (Some(detectors), Some(spill)) if spill.len() == detectors.len() * detectors.len() => {
                        let info = CompInfo { detectors, spill, source: c.get("source").and_then(Json::as_str).unwrap_or("workspace").to_string() };
                        if let Err(e) = self.set_compensation(i, &info, Mode::Standard) {
                            notes.push(format!("{name}: could not re-apply the saved compensation: {e}"));
                        }
                    }
                    _ => notes.push(format!("{name}: the saved compensation matrix is malformed and was ignored")),
                }
            }
        }
        // the pooled map goes back on first, so the gates drawn on Map-1 / Map-2 find their channels
        self.map_info = doc.get("map_info").and_then(Json::as_str).map(str::to_string);
        self.map_features = doc.get("map_features").and_then(Json::as_arr).map(|a| a.iter().filter_map(|f| f.as_str().map(str::to_string)).collect()).unwrap_or_default();
        self.map_cofactor = doc.get("map_cofactor").and_then(Json::as_f64).unwrap_or(150.0);
        for i in 0..self.samples.len() {
            self.samples[i].clear_map();
        }
        if let Some(Json::Obj(maps)) = doc.get("maps") {
            for (name, m) in maps {
                let Some(i) = self.samples.iter().position(|s| &s.name == name) else { continue };
                let nums = |k: &str| -> Vec<f64> { m.get(k).and_then(Json::as_arr).map(|a| a.iter().filter_map(Json::as_f64).collect()).unwrap_or_default() };
                let (rows, x, y) = (nums("rows"), nums("x"), nums("y"));
                let ne = self.samples[i].n_events;
                if rows.len() != x.len() || rows.len() != y.len() || rows.iter().any(|&r| r < 0.0 || r as usize >= ne) {
                    notes.push(format!("{name}: the saved map does not fit this file and was ignored; gates on Map-1 / Map-2 will be empty"));
                    continue;
                }
                let (mut cx, mut cy) = (vec![f32::NAN; ne], vec![f32::NAN; ne]);
                for (k, &r) in rows.iter().enumerate() {
                    cx[r as usize] = x[k] as f32;
                    cy[r as usize] = y[k] as f32;
                }
                let range = m.get("range").and_then(Json::as_f64).unwrap_or(1.0);
                if let Err(e) = self.set_map(i, cx, cy, range) {
                    notes.push(format!("{name}: could not restore the saved map: {e}"));
                }
            }
        }
        let (mut loaded, mut warnings) = workspace_from_json(gates, "All Events").map_err(|e| e.to_string())?;
        warnings.extend(notes);
        self.markers.clear();
        if let Some(Json::Obj(markers)) = doc.get("markers") {
            for (ch, m) in markers {
                if let Some(m) = m.as_str() {
                    self.markers.insert(ch.clone(), m.to_string());
                }
            }
        }
        self.fmo.clear();
        if let Some(Json::Obj(fmo)) = doc.get("fmo") {
            for (ch, f) in fmo {
                match f.get("level").and_then(Json::as_f64).filter(|v| v.is_finite()) {
                    Some(level) => {
                        let source = f.get("source").and_then(Json::as_str).unwrap_or("workspace").to_string();
                        let n = f.get("n").and_then(Json::as_f64).unwrap_or(0.0) as usize;
                        let percentile = f.get("percentile").and_then(Json::as_f64).unwrap_or(95.0);
                        self.fmo.insert(ch.clone(), Fmo { level, source, n, percentile });
                    }
                    None => warnings.push(format!("the saved FMO level for {ch} is not a number and was ignored")),
                }
            }
        }
        for i in 0..self.samples.len() {
            let name = self.samples[i].name.clone();
            if let Some(sg) = loaded.remove(&name) {
                self.states[i].tree = sg.tree;
                self.states[i].meta = sg.meta;
                self.states[i].undo.clear();
                self.states[i].redo.clear();
                self.touch(i);
            }
        }
        for orphan in loaded.keys() {
            warnings.push(format!("the workspace has gates for '{orphan}', which is not loaded"));
        }
        let sharing = doc.get("groups").cloned().unwrap_or_else(Json::obj);
        warnings.extend(self.groups_from_json(&sharing));
        self.windows.clear();
        self.selected = None;
        Ok(warnings)
    }
}

// ---- FlowJo workspace import ----------------------------------------------------------------------

/// What an import did, in terms a user can check.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    /// Samples whose gating tree was imported.
    pub imported: Vec<String>,
    /// FCS files that were not already loaded and were opened from next to the workspace.
    pub opened: Vec<String>,
    /// Workspace samples that were skipped (FCS file not found, ...), with the reason.
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    pub populations: usize,
    /// Populations (user-drawn gates only) compared against the counts FlowJo saved, and their mean relative difference.
    pub compared: usize,
    pub mean_diff_pct: f64,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut s = format!("Imported {} sample(s), {} population(s)", self.imported.len(), self.populations);
        if self.compared > 0 {
            s.push_str(&format!("; counts agree with FlowJo's saved counts to {:.2}% on average ({} populations)", self.mean_diff_pct, self.compared));
        }
        if !self.skipped.is_empty() {
            s.push_str(&format!("; {} skipped ({})", self.skipped.len(), self.skipped.join("; ")));
        }
        if !self.warnings.is_empty() {
            s.push_str(&format!("; {} note(s) in the Analysis Log", self.warnings.len()));
        }
        s.push('.');
        s
    }
}

impl Session {
    /// Import a FlowJo `.wsp`: for every workspace sample whose FCS file is loaded (or sits in `dir`), apply the
    /// workspace's compensation and replace that sample's gating tree with FlowJo's. Gate coordinates stay in
    /// FlowJo's display space, so each gate records the scale of each axis (see `Gate::x_scale`).
    pub fn import_wsp(&mut self, text: &str, dir: &Path) -> Result<ImportReport, String> {
        let wsamples = sf_gating::wsp::parse_wsp(text).map_err(|e| e.to_string())?;
        let mut rep = ImportReport::default();
        let mut diff_sum = 0.0;
        for ws in wsamples {
            let label = ws.file_name.clone().unwrap_or_else(|| ws.name.clone());
            let idx = match self.samples.iter().position(|s| s.name.eq_ignore_ascii_case(&label) || s.name.eq_ignore_ascii_case(&ws.name)) {
                Some(i) => i,
                None => {
                    let p = dir.join(&label);
                    if p.is_file() {
                        match self.load_fcs(&p) {
                            Ok(i) => {
                                rep.opened.push(label.clone());
                                i
                            }
                            Err(e) => {
                                rep.skipped.push(format!("{label}: {e}"));
                                continue;
                            }
                        }
                    } else {
                        rep.skipped.push(format!("{label}: FCS file not found next to the workspace"));
                        continue;
                    }
                }
            };
            for w in &ws.warnings {
                rep.warnings.push(format!("{label}: {w}"));
            }
            if let Some(c) = &ws.comp {
                let info = CompInfo { detectors: c.detectors.clone(), spill: c.spill.clone(), source: "FlowJo workspace".into() };
                if let Err(e) = self.set_compensation(idx, &info, Mode::Standard) {
                    rep.warnings.push(format!("{label}: compensation not applied: {e}"));
                }
            }
            // rebuild FlowJo's tree with each gate's axes tagged with their scales
            let order = ws.tree.self_and_descendants(GateTree::ROOT).map_err(|e| e.to_string())?;
            let mut tree = GateTree::new("All Events");
            let mut map: HashMap<NodeId, NodeId> = HashMap::new();
            map.insert(GateTree::ROOT, GateTree::ROOT);
            for &n in order.iter().skip(1) {
                let NodeKind::Gate(g) = ws.tree.kind(n).map_err(|e| e.to_string())? else { continue };
                let scale = |chan: &str| ws.transforms.get(chan).cloned().unwrap_or(ScaleSpec::Identity);
                let (xs, ys) = (scale(&g.x_chan), g.y_chan.as_deref().map_or(ScaleSpec::Identity, scale));
                let parent = ws.tree.parent(n).map_err(|e| e.to_string())?.and_then(|p| map.get(&p).copied()).unwrap_or(GateTree::ROOT);
                let name = ws.tree.name(n).map_err(|e| e.to_string())?.to_string();
                let nid = tree.add_gate(parent, &name, g.clone().with_scales(xs, ys)).map_err(|e| e.to_string())?;
                map.insert(n, nid);
            }
            rep.populations += map.len() - 1;
            self.snapshot(idx);
            self.states[idx].tree = tree;
            self.states[idx].meta.clear();
            self.touch(idx);
            rep.imported.push(label.clone());
            // check against the counts FlowJo itself saved (user-drawn gates only; FlowJo's automatic gates differ)
            let counts = self.evaluation(idx)?.counts.clone();
            for (old, want) in &ws.flowjo_counts {
                if *old == GateTree::ROOT || ws.auto_gates.contains(old) || *want < 50 {
                    continue;
                }
                if let Some(&new) = map.get(old) {
                    diff_sum += (counts[new] as f64 - *want as f64).abs() / *want as f64;
                    rep.compared += 1;
                }
            }
        }
        if rep.compared > 0 {
            rep.mean_diff_pct = 100.0 * diff_sum / rep.compared as f64;
        }
        for w in &rep.warnings {
            self.log.push(w.clone());
        }
        self.windows.clear();
        self.selected = None;
        if rep.imported.is_empty() {
            return Err(format!("no sample of the workspace could be matched to an FCS file ({})", rep.skipped.join("; ")));
        }
        Ok(rep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::rng::Rng;

    fn sample(name: &str, n: usize, seed: u64) -> LoadedSample {
        let mut rng = Rng::new(seed);
        let events = (0..n).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 100_000.0) as f32, (rng.next_f64() * 20_000.0 - 500.0) as f32, (rng.next_f64() * 20_000.0 - 500.0) as f32]).collect();
        LoadedSample::synthetic(name, &["FSC-A", "SSC-A", "CD3", "CD4"], vec![262_144.0; 4], events)
    }

    fn rect(s: &mut Session, win: u64, x0: f64, x1: f64, y0: f64, y1: f64, name: &str) -> NodeId {
        s.commit_shape(win, &Shape::Rect { x0, x1, y0, y1 }, name).unwrap()
    }

    #[test]
    fn default_scales_follow_the_channel_kind() {
        assert!(default_scale("FSC-A", 262144.0).is_identity() && default_scale("SSC-H", 1.0).is_identity() && default_scale("Time", 1.0).is_identity());
        assert!(matches!(default_scale("FITC-A", 262144.0), ScaleSpec::Logicle { .. }));
    }

    #[test]
    fn golden_invariant_any_depth_opens_in_a_graph_window() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 3000, 1));
        let w0 = s.open_population(i, GateTree::ROOT).unwrap();
        // five levels like Lymphocytes -> CD3+ -> CD4+ -> CD25+ -> FoxP3+, each keeping most events
        let mut win = w0;
        let mut nodes = vec![];
        for lvl in 0..5 {
            let node = rect(&mut s, win, -1e9, 1e9, -1e9, 1e9, &format!("L{lvl}"));
            nodes.push(node);
            win = s.open_population(i, node).unwrap();
        }
        assert_eq!(s.windows.len(), 6);
        assert!(s.tree(i).depth(*nodes.last().unwrap()).unwrap() == 5);
        // and 300 levels (no depth guard anywhere)
        let mut cur = *nodes.last().unwrap();
        for lvl in 0..300 {
            let w = s.open_population(i, cur).unwrap();
            cur = rect(&mut s, w, -1e9, 1e9, -1e9, 1e9, &format!("deep{lvl}"));
            s.close_window(w);
        }
        let w = s.open_population(i, cur).unwrap();
        assert!(s.window_points(w).unwrap().len() > 2000, "the deepest population still has its events");
        assert_eq!(s.open_population(i, cur).unwrap(), w, "opening again focuses the same window");
    }

    #[test]
    fn drawing_a_gate_updates_counts_and_the_child_is_gated_inside_its_parent() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 4000, 2));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        assert_eq!((s.window(w).unwrap().x_chan.as_str(), s.window(w).unwrap().y_chan.as_str()), ("FSC-A", "SSC-A"));
        let half = rect(&mut s, w, 0.0, 100_000.0, -1e9, 1e9, "Half");
        let n_half = s.count(i, half);
        let expect = s.samples()[i].events.chunks(4).filter(|r| r[0] >= 0.0 && r[0] < 100_000.0).count();
        assert_eq!(n_half, expect);
        let w2 = s.open_population(i, half).unwrap();
        let sub = rect(&mut s, w2, -1e9, 1e9, 0.0, 50_000.0, "Sub");
        assert!(s.count(i, sub) < n_half && s.count(i, sub) > 0);
        assert_eq!(s.window_points(w2).unwrap().len(), n_half, "a window shows only its own population");
        assert!(s.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1.0, y0: 0.0, y1: 1.0 }, "Half").is_err(), "duplicate sibling name refused");
        assert_eq!(s.next_name(i, GateTree::ROOT), "Population 1");
    }

    #[test]
    fn a_gate_on_a_scaled_axis_selects_by_display_space() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 4000, 3));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        {
            let win = s.window_mut(w).unwrap();
            win.x_chan = "CD3".into();
            win.y_chan = "CD4".into();
            win.x_scale = ScaleSpec::Logicle { t: 262_144.0, w: 0.5, m: 4.5, a: 0.0 };
            win.y_scale = ScaleSpec::Identity;
        }
        let t = s.window(w).unwrap().x_scale.build().unwrap();
        let g = rect(&mut s, w, t.forward(1000.0), t.forward(10_000.0), -1e9, 1e9, "Mid");
        let want = s.samples()[i].events.chunks(4).filter(|r| (1000.0..10_000.0).contains(&(r[2] as f64))).count();
        assert!((s.count(i, g) as i64 - want as i64).abs() <= 2, "{} vs {want}", s.count(i, g));
    }

    #[test]
    fn apply_to_all_replaces_instead_of_stacking_and_reports_missing_parents() {
        let mut s = Session::new();
        let a = s.add_sample(sample("a.fcs", 500, 4));
        let b = s.add_sample(sample("b.fcs", 500, 5));
        let c = s.add_sample(sample("c.fcs", 500, 6));
        let wa = s.open_population(a, GateTree::ROOT).unwrap();
        let p = rect(&mut s, wa, 0.0, 1e5, 0.0, 1e5, "P");
        let wp = s.open_population(a, p).unwrap();
        let child = rect(&mut s, wp, 0.0, 5e4, 0.0, 5e4, "Child");
        // b has P, c does not
        assert_eq!(s.apply_to_all(a, p).unwrap(), 2);
        assert_eq!(s.apply_to_all(a, p).unwrap(), 2, "idempotent");
        for o in [b, c] {
            assert_eq!(s.tree(o).children(GateTree::ROOT).unwrap().len(), 1, "no duplicate stacked");
        }
        let log_before = s.log.len();
        // remove P from c, then the child cannot be applied there
        let cp = s.tree(c).find(&["P"]).unwrap();
        s.release_gate(c, cp).unwrap(); // a group-owned gate is released before this sample can drop it
        s.delete(c, cp).unwrap();
        assert_eq!(s.apply_to_all(a, child).unwrap(), 1);
        assert!(s.log[log_before..].iter().any(|m| m.contains("c.fcs") && m.contains("skipped")), "{:?}", s.log);
        assert!(s.apply_to_all(a, GateTree::ROOT).is_err());
    }

    #[test]
    fn deleting_closes_windows_beneath_and_rename_checks_siblings() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 500, 7));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        let a = rect(&mut s, w, 0.0, 1e5, 0.0, 1e5, "A");
        let b = rect(&mut s, w, 0.0, 1e5, 0.0, 1e5, "B");
        let wa = s.open_population(i, a).unwrap();
        let kid = rect(&mut s, wa, 0.0, 5e4, 0.0, 5e4, "Kid");
        s.open_population(i, kid).unwrap();
        assert_eq!(s.windows.len(), 3);
        assert_eq!(s.delete(i, a).unwrap(), 2);
        assert_eq!(s.windows.len(), 1, "windows on A and Kid are closed, the root window stays");
        assert!(s.open_population(i, kid).is_err(), "a deleted population cannot be opened");
        assert!(s.rename(i, b, "").is_err() && s.rename(i, GateTree::ROOT, "x").is_err());
        s.rename(i, b, "Renamed").unwrap();
        assert_eq!(s.tree(i).name(b).unwrap(), "Renamed");
    }

    #[test]
    fn a_gate_on_a_missing_channel_warns_without_breaking_siblings() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 500, 8));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        s.window_mut(w).unwrap().x_chan = "PE-A".into(); // not in this sample
        let bad = rect(&mut s, w, 0.0, 1.0, 0.0, 1.0, "Bad");
        s.window_mut(w).unwrap().x_chan = "FSC-A".into();
        let good = rect(&mut s, w, 0.0, 1e5, 0.0, 1e5, "Good");
        assert_eq!(s.count(i, bad), 0);
        assert!(s.count(i, good) > 0);
        assert!(s.log.iter().any(|m| m.contains("Bad") && m.contains("PE-A")), "{:?}", s.log);
    }

    #[test]
    fn workspace_round_trip_restores_gates_scales_and_remembered_views() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 800, 9));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        {
            let win = s.window_mut(w).unwrap();
            win.x_chan = "CD3".into();
            win.x_scale = ScaleSpec::Arcsinh { cofactor: 150.0 };
        }
        let g = rect(&mut s, w, 2.0, 4.0, 0.0, 1e5, "T");
        let w2 = s.open_population(i, g).unwrap();
        s.window_mut(w2).unwrap().y_chan = "CD4".into();
        s.remember_view(w2);
        let before = s.count(i, g);
        let text = s.to_workspace_json().write();

        let mut s2 = Session::new();
        let j = s2.add_sample(sample("a.fcs", 800, 9));
        let warns = s2.apply_workspace_json(&Json::parse(&text).unwrap()).unwrap();
        assert!(warns.is_empty(), "{warns:?}");
        let g2 = s2.tree(j).find(&["T"]).unwrap();
        assert_eq!(s2.count(j, g2), before);
        let w3 = s2.open_population(j, g2).unwrap();
        assert_eq!(s2.window(w3).unwrap().y_chan, "CD4", "the remembered axes come back");
        // gates for a sample that is not loaded are reported, not silently dropped
        let mut s3 = Session::new();
        s3.add_sample(sample("other.fcs", 10, 1));
        let w4 = s3.apply_workspace_json(&Json::parse(&text).unwrap()).unwrap();
        assert!(w4.iter().any(|m| m.contains("a.fcs") && m.contains("not loaded")), "{w4:?}");
        assert!(s3.apply_workspace_json(&Json::parse("{}").unwrap()).is_err());
    }

    #[test]
    fn auto_range_covers_the_population() {
        let mut s = Session::new();
        let i = s.add_sample(sample("a.fcs", 1000, 10));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        let r = s.auto_range(w).unwrap();
        assert!(r.is_valid());
        let pts = s.window_points(w).unwrap();
        assert!(pts.iter().all(|p| p.0 >= r.x0 && p.0 <= r.x1 && p.1 >= r.y0 && p.1 <= r.y1));
    }
}

#[cfg(test)]
mod history_and_comp_tests {
    use super::*;
    use crate::drawing::Shape;
    use sf_core::rng::Rng;

    fn session_with(n: usize) -> (Session, usize, u64) {
        let mut rng = Rng::new(5);
        let events = (0..n).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 100_000.0) as f32, (rng.next_f64() * 20_000.0) as f32, (rng.next_f64() * 20_000.0) as f32]).collect();
        let mut s = Session::new();
        let i = s.add_sample(LoadedSample::synthetic("a.fcs", &["FSC-A", "SSC-A", "FITC-A", "PE-A"], vec![262_144.0; 4], events));
        let w = s.open_population(i, GateTree::ROOT).unwrap();
        (s, i, w)
    }

    fn rect(s: &mut Session, w: u64, name: &str) -> NodeId {
        s.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, name).unwrap()
    }

    #[test]
    fn undo_and_redo_walk_back_and_forth_through_every_kind_of_edit() {
        let (mut s, i, w) = session_with(500);
        assert!(!s.can_undo(i) && !s.undo(i) && !s.redo(i));
        let a = rect(&mut s, w, "A");
        let b = rect(&mut s, w, "B");
        s.rename(i, a, "A2").unwrap();
        s.delete(i, b).unwrap();
        let names = |s: &Session| s.tree(i).children(GateTree::ROOT).unwrap().iter().map(|&c| s.tree(i).name(c).unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(names(&s), vec!["A2"]);
        assert!(s.undo(i)); // un-delete
        assert_eq!(names(&s), vec!["A2", "B"]);
        assert!(s.undo(i)); // un-rename
        assert_eq!(names(&s), vec!["A", "B"]);
        assert!(s.redo(i));
        assert_eq!(names(&s), vec!["A2", "B"]);
        // a new edit after undo drops the redo branch
        s.undo(i);
        rect(&mut s, w, "C");
        assert!(!s.can_redo(i));
        assert_eq!(names(&s), vec!["A", "B", "C"]);
        // undo everything
        while s.undo(i) {}
        assert!(names(&s).is_empty() && !s.can_undo(i));
    }

    #[test]
    fn failed_operations_leave_no_phantom_undo_step() {
        let (mut s, i, w) = session_with(200);
        rect(&mut s, w, "A");
        assert!(s.can_undo(i));
        assert!(s.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 1.0, y0: 0.0, y1: 1.0 }, "A").is_err(), "duplicate refused");
        assert!(s.delete(i, GateTree::ROOT).is_err(), "root cannot be deleted");
        s.undo(i); // undoes the ONE real edit, not a phantom
        assert!(s.tree(i).children(GateTree::ROOT).unwrap().is_empty());
        assert!(!s.can_undo(i), "only one step existed");
    }

    #[test]
    fn undo_closes_windows_on_populations_that_no_longer_exist() {
        let (mut s, i, w) = session_with(300);
        let a = rect(&mut s, w, "A");
        let wa = s.open_population(i, a).unwrap();
        assert!(s.window(wa).is_some());
        s.undo(i); // the gate is gone
        assert!(s.window(wa).is_none() && s.window(w).is_some());
        assert!(s.selected.is_none());
        s.redo(i);
        assert!(s.open_population(i, a).is_ok(), "ids are restored with the tree");
    }

    #[test]
    fn quadrants_make_four_partitioning_populations_atomically() {
        let (mut s, i, w) = session_with(2000);
        {
            let win = s.window_mut(w).unwrap();
            win.x_chan = "FITC-A".into();
            win.y_chan = "PE-A".into();
            win.x_scale = ScaleSpec::Identity;
            win.y_scale = ScaleSpec::Identity;
        }
        let ids = s.commit_quadrants(w, 10_000.0, 10_000.0).unwrap();
        assert_eq!(ids.len(), 4);
        let names: Vec<String> = ids.iter().map(|&n| s.tree(i).name(n).unwrap().to_string()).collect();
        assert_eq!(names, vec!["FITC+ PE+", "FITC- PE+", "FITC- PE-", "FITC+ PE-"]);
        let total: usize = ids.iter().map(|&n| s.count(i, n)).sum();
        assert_eq!(total, 2000, "the quadrants partition the population exactly");
        assert!(ids.iter().all(|&n| s.count(i, n) > 300));
        // asking again would collide: nothing is added, and one undo removes the original four
        assert!(s.commit_quadrants(w, 5_000.0, 5_000.0).is_err());
        assert_eq!(s.tree(i).children(GateTree::ROOT).unwrap().len(), 4);
        s.undo(i);
        assert!(s.tree(i).children(GateTree::ROOT).unwrap().is_empty());
    }

    #[test]
    fn compensation_adds_channels_gates_follow_and_it_survives_save_load() {
        let (mut s, i, w) = session_with(1500);
        let info = CompInfo { detectors: vec!["FITC-A".into(), "PE-A".into()], spill: vec![1.0, 0.25, 0.0, 1.0], source: "test".into() };
        s.set_compensation(i, &info, Mode::Standard).unwrap();
        assert!(s.sample(i).channel_index("Comp-PE-A").is_some());
        // a gate drawn on the compensated channel works and is persisted by name
        {
            let win = s.window_mut(w).unwrap();
            win.x_chan = "Comp-FITC-A".into();
            win.y_chan = "Comp-PE-A".into();
            win.x_scale = ScaleSpec::Identity;
            win.y_scale = ScaleSpec::Identity;
        }
        let g = s.commit_shape(w, &Shape::Rect { x0: 0.0, x1: 10_000.0, y0: -1e9, y1: 5_000.0 }, "Dim").unwrap();
        let before = s.count(i, g);
        assert!(before > 0 && before < 1500);
        let text = s.to_workspace_json().write();
        // reload into a fresh session that has NO compensation yet
        let (mut s2, j, _) = session_with(1500);
        let warns = s2.apply_workspace_json(&Json::parse(&text).unwrap()).unwrap();
        assert!(warns.is_empty(), "{warns:?}");
        assert_eq!(s2.sample(j).comp.as_ref().map(|c| c.spill.clone()), Some(info.spill.clone()));
        let g2 = s2.tree(j).find(&["Dim"]).unwrap();
        assert_eq!(s2.count(j, g2), before, "same gate, same compensated numbers");
        // clearing compensation makes the Comp- gate report a missing channel instead of silently using raw data
        s2.clear_compensation(j);
        assert_eq!(s2.count(j, g2), 0);
        assert!(s2.log.iter().any(|m| m.contains("Dim") && m.contains("Comp-")), "{:?}", s2.log);
        // applying to all reports samples that lack the detectors
        let failed = s2.set_compensation_all(&CompInfo { detectors: vec!["BV421-A".into()], spill: vec![1.0], source: "x".into() }, Mode::Standard);
        assert_eq!(failed.len(), 1);
    }
}

#[cfg(test)]
mod navigation_and_plot_tests {
    use super::*;
    use crate::density::{bin, histogram, render_dots, PlotKind, Range};
    use crate::drawing::{Drawing, Shape, Tool};
    use crate::sample::LoadedSample;
    use sf_core::rng::Rng;

    fn two_samples() -> (Session, usize, usize) {
        let mk = |seed: u64| {
            let mut rng = Rng::new(seed);
            let ev: Vec<f32> = (0..600).flat_map(|_| [(rng.next_f64() * 200_000.0) as f32, (rng.next_f64() * 100_000.0) as f32]).collect();
            LoadedSample::synthetic(&format!("s{seed}.fcs"), &["FSC-A", "SSC-A"], vec![262_144.0; 2], ev)
        };
        let mut s = Session::new();
        let a = s.add_sample(mk(1));
        let b = s.add_sample(mk(2));
        (s, a, b)
    }

    #[test]
    fn breadcrumb_lists_the_path_root_first_at_any_depth() {
        let (mut s, a, _) = two_samples();
        let mut w = s.open_population(a, GateTree::ROOT).unwrap();
        let mut node = GateTree::ROOT;
        for i in 0..50 {
            node = s.commit_shape(w, &Shape::Rect { x0: -1e9, x1: 1e9, y0: -1e9, y1: 1e9 }, &format!("L{i}")).unwrap();
            s.close_window(w);
            w = s.open_population(a, node).unwrap();
        }
        let bc = s.breadcrumb(a, node);
        assert_eq!(bc.len(), 51);
        assert_eq!((bc[0].0.as_str(), bc[1].0.as_str(), bc[50].0.as_str()), ("All Events", "L0", "L49"));
        assert_eq!(bc.last().unwrap().1, node);
    }

    #[test]
    fn a_window_can_hop_to_the_same_population_in_another_sample() {
        let (mut s, a, b) = two_samples();
        let wa = s.open_population(a, GateTree::ROOT).unwrap();
        let g = s.commit_shape(wa, &Shape::Rect { x0: 0.0, x1: 1e5, y0: 0.0, y1: 1e5 }, "P").unwrap();
        let w = s.open_population(a, g).unwrap();
        let err = s.retarget_window(w, b).unwrap_err();
        assert!(err.contains("s2.fcs") && err.contains("'P'"), "{err}");
        assert_eq!(s.window(w).unwrap().sample, a, "a failed hop changes nothing");
        s.apply_to_all(a, g).unwrap();
        s.retarget_window(w, b).unwrap();
        assert_eq!((s.window(w).unwrap().sample, s.selected.map(|x| x.0)), (b, Some(b)));
        assert_eq!(s.tree(b).name(s.window(w).unwrap().node).unwrap(), "P");
        // hopping onto a population another tab already shows is refused rather than duplicating a tab
        let other = s.open_population(a, g).unwrap();
        assert!(s.retarget_window(other, b).unwrap_err().contains("already open"));
        assert!(s.retarget_window(w, 9).is_err());
    }

    #[test]
    fn the_quadrant_tool_returns_a_centre_and_cannot_be_turned_into_a_single_gate() {
        let mut d = Drawing::default();
        d.press(Tool::Quadrant, (1.0, 2.0));
        assert_eq!(d.release(Tool::Quadrant, (3.0, 4.0)), Some(Shape::Quadrant { cx: 3.0, cy: 4.0 }));
        let e = crate::drawing::to_gate(&Shape::Quadrant { cx: 0.0, cy: 0.0 }, "X", "Y", &ScaleSpec::Identity, &ScaleSpec::Identity);
        assert!(e.is_err());
        assert_eq!(Tool::ALL.len(), 6);
    }

    #[test]
    fn histogram_and_dot_rendering() {
        let h = histogram([0.0, 0.5, 1.0, 1.0, 2.0, f64::NAN, -1.0, 9.0].into_iter(), 0.0, 2.0, 4);
        assert_eq!(h, vec![1, 1, 2, 1], "0.0 -> bin0, 0.5 -> bin1, 1.0 x2 -> bin2, 2.0 (upper edge) -> last bin");
        let g = bin([(0.5, 0.5), (0.6, 0.5)].into_iter(), Range { x0: 0.0, x1: 1.0, y0: 0.0, y1: 1.0 }, 10, 10);
        let px = render_dots(&g, [10, 20, 30]);
        assert_eq!(px.iter().filter(|p| p[3] == 255).count(), 2);
        assert!(px.iter().filter(|p| p[3] == 255).all(|p| p[..3] == [10, 20, 30]));
        assert_eq!(PlotKind::default(), PlotKind::Pseudocolor);
    }

    #[test]
    fn population_points_work_for_any_sample_without_a_window() {
        let (mut s, a, b) = two_samples();
        let id = ScaleSpec::Identity;
        let pa = s.population_points(a, GateTree::ROOT, "FSC-A", "SSC-A", &id, &id).unwrap();
        let pb = s.population_points(b, GateTree::ROOT, "FSC-A", "SSC-A", &id, &id).unwrap();
        assert_eq!((pa.len(), pb.len()), (600, 600));
        assert!(pa != pb);
        assert!(s.population_points(a, GateTree::ROOT, "CD3", "SSC-A", &id, &id).is_err());
    }
}

#[cfg(test)]
mod gate_op_tests {
    use super::*;
    use sf_gating::{Gate, GateKind};

    fn session_with_sample() -> (Session, usize) {
        let mut s = Session::new();
        let channels: Vec<String> = ["FSC-A", "SSC-A"].iter().map(|c| c.to_string()).collect();
        let n = 2000;
        let data: Vec<f32> = (0..n).flat_map(|i| [((i * 37) % 1000) as f32, ((i * 11) % 500) as f32]).collect();
        let _ = &channels;
        let idx = s.add_sample(crate::sample::LoadedSample::synthetic("t.fcs", &["FSC-A", "SSC-A"], vec![1000.0, 1000.0], data));
        (s, idx)
    }

    fn add(s: &mut Session, si: usize, g: Gate, name: &str) -> NodeId {
        s.paste_gate(si, GateTree::ROOT, g, name).unwrap()
    }

    #[test]
    fn rectangles_and_ellipses_become_polygons_with_the_same_members() {
        let (mut s, si) = session_with_sample();
        let r = add(&mut s, si, Gate::new(GateKind::Rectangle, "FSC-A", Some("SSC-A"), vec![100.0, 600.0], vec![50.0, 300.0]).unwrap(), "R");
        let before = s.count(si, r);
        assert!(before > 0);
        s.convert_to_polygon(si, r).unwrap();
        assert!(matches!(s.gate_of(si, r).unwrap().kind, GateKind::Polygon));
        // a polygon with the rectangle's corners is half-open-agnostic: allow only events on the border to differ
        let after = s.count(si, r);
        assert!((after as i64 - before as i64).abs() < before as i64 / 20 + 5, "{before} vs {after}");
        let e = add(&mut s, si, Gate::new(GateKind::Ellipse, "FSC-A", Some("SSC-A"), vec![200.0, 800.0], vec![100.0, 400.0]).unwrap(), "E");
        let eb = s.count(si, e);
        s.convert_to_polygon(si, e).unwrap();
        let ea = s.count(si, e);
        assert!((ea as i64 - eb as i64).abs() < eb as i64 / 10 + 5, "48-gon ~ ellipse: {eb} vs {ea}");
        assert!(s.convert_to_polygon(si, e).unwrap_err().contains("already"));
        // one undo step each
        assert!(s.undo(si));
        assert!(matches!(s.gate_of(si, e).unwrap().kind, GateKind::Ellipse));
        assert!(s.convert_to_polygon(si, GateTree::ROOT).is_err());
    }

    #[test]
    fn a_node_is_inserted_on_the_nearest_edge_and_changes_nothing_else() {
        let (mut s, si) = session_with_sample();
        let p = add(&mut s, si, Gate::new(GateKind::Polygon, "FSC-A", Some("SSC-A"), vec![0.0, 400.0, 400.0, 0.0], vec![0.0, 0.0, 300.0, 300.0]).unwrap(), "P");
        let n0 = s.count(si, p);
        s.add_polygon_node(si, p, (200.0, -30.0)).unwrap();
        let g = s.gate_of(si, p).unwrap();
        assert_eq!(g.xs, vec![0.0, 200.0, 400.0, 400.0, 0.0]);
        assert_eq!(g.ys, vec![0.0, 0.0, 0.0, 300.0, 300.0], "the new corner sits on the bottom edge");
        assert_eq!(s.count(si, p), n0, "a corner on a straight edge changes no membership");
        s.add_polygon_node(si, p, (0.0, 250.0)).unwrap();
        assert_eq!(s.gate_of(si, p).unwrap().xs.len(), 6);
        assert!(s.undo(si) && s.undo(si));
        assert_eq!(s.gate_of(si, p).unwrap().xs.len(), 4);
        let r = add(&mut s, si, Gate::new(GateKind::Rectangle, "FSC-A", Some("SSC-A"), vec![0.0, 10.0], vec![0.0, 10.0]).unwrap(), "R");
        assert!(s.add_polygon_node(si, r, (1.0, 1.0)).is_err());
    }

    #[test]
    fn exclusion_flips_the_population_to_its_complement_and_back() {
        let (mut s, si) = session_with_sample();
        let r = add(&mut s, si, Gate::new(GateKind::Rectangle, "FSC-A", Some("SSC-A"), vec![100.0, 600.0], vec![50.0, 300.0]).unwrap(), "R");
        let inside = s.count(si, r);
        assert_eq!(s.toggle_exclusion(si, r), Ok(true));
        assert_eq!(s.count(si, r) + inside, 2000, "inside and outside partition the parent");
        assert_eq!(s.toggle_exclusion(si, r), Ok(false));
        assert_eq!(s.count(si, r), inside);
    }

    #[test]
    fn a_pasted_gate_lands_under_the_chosen_population_in_any_sample_and_checks_its_channels() {
        let (mut s, si) = session_with_sample();
        let g = Gate::new(GateKind::Rectangle, "FSC-A", Some("SSC-A"), vec![0.0, 500.0], vec![0.0, 250.0]).unwrap();
        let a = s.paste_gate(si, GateTree::ROOT, g.clone(), "A").unwrap();
        let b = s.paste_gate(si, a, g.clone(), "B").unwrap();
        assert_eq!(s.tree(si).parent(b).unwrap(), Some(a));
        let missing = Gate::new(GateKind::Interval, "CD3-A", None, vec![0.0, 1.0], vec![]).unwrap();
        assert!(s.paste_gate(si, a, missing, "C").unwrap_err().contains("CD3-A"));
        assert!(s.paste_gate(si, a, Gate::indexed(vec![1, 2]), "I").unwrap_err().contains("cannot be pasted"));
        assert!(s.undo(si));
        assert!(!s.tree(si).is_alive(b));
    }

    #[test]
    fn a_subsample_is_seeded_sorted_inside_the_population_and_undoable() {
        let (mut s, si) = session_with_sample();
        let r = add(&mut s, si, Gate::new(GateKind::Rectangle, "FSC-A", Some("SSC-A"), vec![0.0, 600.0], vec![0.0, 300.0]).unwrap(), "R");
        let total = s.count(si, r);
        let a = s.subsample(si, r, 100, 7, "100 events").unwrap();
        assert_eq!(s.count(si, a), 100);
        let rows = |s: &mut Session, n: NodeId| match s.gate_of(si, n).unwrap().kind {
            GateKind::Indexed(v) => v,
            _ => panic!(),
        };
        let first = rows(&mut s, a);
        assert!(first.windows(2).all(|w| w[0] < w[1]), "ascending, distinct");
        let parent_rows: std::collections::HashSet<usize> = s.evaluation(si).unwrap().masks[r].as_ref().unwrap().indices().into_iter().collect();
        assert!(first.iter().all(|i| parent_rows.contains(&(*i as usize))), "all events come from the parent");
        let b = s.subsample(si, r, 100, 7, "again").unwrap();
        assert_eq!(rows(&mut s, b), first, "same seed, same events");
        let c = s.subsample(si, r, 100, 8, "other").unwrap();
        assert_ne!(rows(&mut s, c), first);
        assert!(s.subsample(si, r, 0, 1, "x").is_err());
        assert!(s.subsample(si, r, total, 1, "x").unwrap_err().contains("only"));
        assert!(s.undo(si));
        assert!(!s.tree(si).is_alive(c));
    }
}
