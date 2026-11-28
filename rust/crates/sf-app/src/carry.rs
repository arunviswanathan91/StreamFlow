//! Carrying a population found on a pooled map to every event of every file. A map holds only the pooled events,
//! so a gate drawn on it says nothing about the rest. Two ways to reach them, both learned from the pooled events
//! inside and outside the gate, and both reporting how well they reproduce it:
//!
//! * a random-forest classifier on the map's markers, applied to all events ([`carry_classifier`]);
//! * a short conventional gating path, a few one-marker cuts in sequence, that anyone can read, write in a methods
//!   section and redraw by hand ([`suggest_path`], [`make_path_gates`]).
//!
//! Nothing is random beyond the given seed.

use crate::embed::MAP_POPULATION;
use crate::session::{Session, ALL_SAMPLES};
use sf_core::classify::Forest;
use sf_core::scale::ScaleSpec;
use sf_gating::{Gate, GateTree, NodeId};

/// The pooled events of every sample that has the population, labelled inside (1) or outside (0) it.
#[derive(Debug, Clone)]
pub struct Training {
    /// `n x d`, transformed (arcsinh) marker values.
    pub x: Vec<f64>,
    pub y: Vec<usize>,
    pub features: Vec<String>,
    pub cofactor: f64,
    pub n_in: usize,
    pub n_out: usize,
    /// Path of the population the map was made from (empty = All Events), of the map events, and of the population.
    pub source: Vec<String>,
    pub node: Vec<String>,
    pub samples: usize,
}

impl Training {
    pub fn n(&self) -> usize {
        self.y.len()
    }
    pub fn d(&self) -> usize {
        self.features.len()
    }
    pub fn name(&self) -> &str {
        self.node.last().map_or("population", |s| s.as_str())
    }
}

/// How well a rule reproduces the population on pooled events it was not fitted on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quality {
    /// Of the population's events, the share the rule finds.
    pub recall: f64,
    /// Of the events the rule selects, the share that are in the population.
    pub precision: f64,
    pub tested_in: usize,
    pub tested_out: usize,
}

impl Quality {
    pub fn of(truth: &[usize], said: &[bool]) -> Quality {
        let (mut tp, mut fp, mut pos) = (0usize, 0usize, 0usize);
        for (&t, &s) in truth.iter().zip(said) {
            pos += t;
            match (t == 1, s) {
                (true, true) => tp += 1,
                (false, true) => fp += 1,
                _ => {}
            }
        }
        Quality { recall: if pos == 0 { 0.0 } else { tp as f64 / pos as f64 }, precision: if tp + fp == 0 { 0.0 } else { tp as f64 / (tp + fp) as f64 }, tested_in: pos, tested_out: truth.len() - pos }
    }
    pub fn f1(&self) -> f64 {
        if self.recall + self.precision == 0.0 {
            0.0
        } else {
            2.0 * self.recall * self.precision / (self.recall + self.precision)
        }
    }
    pub fn words(&self) -> String {
        format!("finds {:.0} % of the population, and {:.0} % of what it selects is the population", 100.0 * self.recall, 100.0 * self.precision)
    }
}

fn find(session: &Session, sample: usize, path: &[String]) -> Option<NodeId> {
    if path.is_empty() {
        return Some(GateTree::ROOT);
    }
    let p: Vec<&str> = path.iter().map(String::as_str).collect();
    session.tree(sample).find(&p)
}

fn transform(v: f64, cofactor: f64) -> f64 {
    if cofactor > 0.0 {
        (v / cofactor).asinh()
    } else {
        v
    }
}

/// Collect the training events for a population that lies under "Map events": from every sample that has it, the
/// pooled events inside and outside it, thinned evenly (true proportions kept) to at most `cap`.
pub fn training(session: &mut Session, sample: usize, node: NodeId, features: &[String], cofactor: f64, cap: usize) -> Result<Training, String> {
    let path = session.tree(sample).path(node).map_err(|e| e.to_string())?;
    let at = path.iter().position(|p| p == MAP_POPULATION).filter(|&i| i + 1 < path.len()).ok_or_else(|| format!("choose a population gated on the map (under '{MAP_POPULATION}')"))?;
    let (source, pooled) = (path[..at].to_vec(), path[..=at].to_vec());
    if features.len() < 2 {
        return Err("the map's markers are not known; run the map again".into());
    }
    let mut rows: Vec<(usize, usize, usize)> = Vec::new(); // (sample, event, label)
    let mut samples = 0;
    for s in 0..session.samples().len() {
        let (Some(n), Some(p)) = (find(session, s, &path), find(session, s, &pooled)) else { continue };
        if features.iter().any(|f| session.sample(s).channel_index(f).is_none()) {
            continue;
        }
        let ne = session.sample(s).n_events;
        let ev = session.evaluation(s)?;
        let idx = |k: NodeId| ev.masks.get(k).and_then(|m| m.as_ref()).map(|m| m.indices()).unwrap_or_default();
        let (inside, all) = (idx(n), idx(p));
        let mut is_in = vec![false; ne];
        inside.iter().for_each(|&r| is_in[r] = true);
        rows.extend(all.into_iter().map(|r| (s, r, usize::from(is_in[r]))));
        samples += 1;
    }
    let step = (rows.len() / cap.max(1)).max(1);
    let rows: Vec<(usize, usize, usize)> = rows.into_iter().step_by(step).collect();
    let n_in: usize = rows.iter().map(|r| r.2).sum();
    let n_out = rows.len() - n_in;
    if n_in < 30 || n_out < 30 {
        return Err(format!("too few pooled events to learn from ({n_in} inside the population, {n_out} outside)"));
    }
    let mut x = Vec::with_capacity(rows.len() * features.len());
    let mut y = Vec::with_capacity(rows.len());
    for &(s, r, l) in &rows {
        let smp = session.sample(s);
        let d = smp.channels.len();
        for f in features {
            let c = smp.channel_index(f).unwrap();
            let v = transform(smp.events[r * d + c] as f64, cofactor);
            x.push(if v.is_finite() { v } else { 0.0 });
        }
        y.push(l);
    }
    Ok(Training { x, y, features: features.to_vec(), cofactor, n_in, n_out, source, node: path, samples })
}

// ---- classifier -------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Carried {
    pub name: String,
    /// On a fifth of the pooled events, held back from training.
    pub quality: Quality,
    /// `(sample name, events put in the population, events of the source population)`.
    pub per_sample: Vec<(String, usize, usize)>,
}

/// Train on four fifths of the pooled events, test on the held-back fifth, then (re-trained on all of them) put
/// every event of the source population of every sample inside or outside. The result is the population
/// "`<name>` (all events)" next to "Map events" in each sample.
pub fn carry_classifier(session: &mut Session, tr: &Training, seed: u64) -> Result<Carried, String> {
    let (n, d) = (tr.n(), tr.d());
    let (mut xa, mut ya, mut xb, mut yb) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..n {
        let row = &tr.x[i * d..(i + 1) * d];
        if i % 5 == 4 {
            xb.extend_from_slice(row);
            yb.push(tr.y[i]);
        } else {
            xa.extend_from_slice(row);
            ya.push(tr.y[i]);
        }
    }
    let held = Forest::fit(&xa, ya.len(), d, &ya, 2, 60, seed);
    let said: Vec<bool> = (0..yb.len()).map(|i| held.predict(&xb[i * d..(i + 1) * d]) == 1).collect();
    let quality = Quality::of(&yb, &said);
    let forest = Forest::fit(&tr.x, n, d, &tr.y, 2, 60, seed);
    let name = format!("{} (all events)", tr.name());
    let mut per_sample = Vec::new();
    for s in 0..session.samples().len() {
        let Some(src) = find(session, s, &tr.source) else { continue };
        if tr.features.iter().any(|f| session.sample(s).channel_index(f).is_none()) {
            continue;
        }
        let rows: Vec<usize> = session.evaluation(s)?.masks.get(src).and_then(|m| m.as_ref()).map(|m| m.indices()).unwrap_or_default();
        let smp = session.sample(s);
        let dch = smp.channels.len();
        let cols: Vec<usize> = tr.features.iter().map(|f| smp.channel_index(f).unwrap()).collect();
        let mut row = vec![0.0; d];
        let mut inside: Vec<u32> = Vec::new();
        for &r in &rows {
            for (k, &c) in cols.iter().enumerate() {
                let v = transform(smp.events[r * dch + c] as f64, tr.cofactor);
                row[k] = if v.is_finite() { v } else { 0.0 };
            }
            if forest.predict(&row) == 1 {
                inside.push(r as u32);
            }
        }
        let sname = smp.name.clone();
        let count = inside.len();
        session.set_index_population(s, src, &name, inside)?;
        per_sample.push((sname, count, rows.len()));
    }
    if per_sample.is_empty() {
        return Err("no loaded sample has the population the map was made from".into());
    }
    Ok(Carried { name, quality, per_sample })
}

// ---- a readable gating path -------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PathStep {
    pub channel: String,
    /// Keep events at or above the threshold (`+`), or below it (`-`).
    pub above: bool,
    /// The cut in raw units and on the arcsinh scale it is drawn on.
    pub threshold: f64,
    pub t: f64,
    /// How well the path up to and including this step reproduces the population.
    pub quality: Quality,
}

/// A short sequence of one-marker cuts whose intersection best reproduces the population on the pooled events.
/// Greedy: each step adds the single cut that most improves F1, and the search stops when a further cut gains
/// less than one point or `max_steps` is reached. The same marker is not cut twice in the same direction.
pub fn suggest_path(tr: &Training, max_steps: usize) -> Vec<PathStep> {
    let (n, d) = (tr.n(), tr.d());
    let mut keep = vec![true; n];
    let mut steps: Vec<PathStep> = Vec::new();
    let mut best_f1 = Quality::of(&tr.y, &keep).f1();
    for _ in 0..max_steps {
        let mut best: Option<(f64, usize, bool, f64, Quality)> = None;
        for j in 0..d {
            // candidate thresholds: quantiles of this marker among the events still kept
            let mut vals: Vec<f64> = (0..n).filter(|&i| keep[i]).map(|i| tr.x[i * d + j]).collect();
            if vals.len() < 40 {
                continue;
            }
            vals.sort_by(|a, b| a.total_cmp(b));
            for q in 1..48 {
                let t = vals[q * (vals.len() - 1) / 48];
                for above in [true, false] {
                    if steps.iter().any(|s| s.channel == tr.features[j] && s.above == above) {
                        continue;
                    }
                    let said: Vec<bool> = (0..n).map(|i| keep[i] && ((tr.x[i * d + j] >= t) == above)).collect();
                    let ql = Quality::of(&tr.y, &said);
                    if best.as_ref().map_or(true, |b| ql.f1() > b.0) {
                        best = Some((ql.f1(), j, above, t, ql));
                    }
                }
            }
        }
        let Some((f1, j, above, t, quality)) = best else { break };
        if f1 < best_f1 + 0.01 {
            break;
        }
        best_f1 = f1;
        for i in 0..n {
            keep[i] = keep[i] && ((tr.x[i * d + j] >= t) == above);
        }
        let threshold = if tr.cofactor > 0.0 { t.sinh() * tr.cofactor } else { t };
        steps.push(PathStep { channel: tr.features[j].clone(), above, threshold, t, quality });
    }
    steps
}

/// Draw the path as nested interval gates under the population the map was made from, in `sample`, and give them
/// to All Samples. Each gate is named after its cut (`CD3+`), the last one "`<name>` (gated)". Returns the last.
pub fn make_path_gates(session: &mut Session, sample: usize, tr: &Training, steps: &[PathStep], label: &dyn Fn(&str) -> String) -> Result<NodeId, String> {
    if steps.is_empty() {
        return Err("there is no gating path to draw".into());
    }
    let mut parent = find(session, sample, &tr.source).ok_or("this sample does not have the population the map was made from")?;
    let scale = if tr.cofactor > 0.0 { ScaleSpec::Arcsinh { cofactor: tr.cofactor } } else { ScaleSpec::Identity };
    // far beyond any real value on the arcsinh scale, so the gate is open on that side
    const FAR: f64 = 40.0;
    for (k, s) in steps.iter().enumerate() {
        let (lo, hi) = if s.above { (s.t, s.t.max(0.0) + FAR) } else { (s.t.min(0.0) - FAR, s.t) };
        let gate = Gate::interval(&s.channel, lo, hi).map_err(|e| e.to_string())?.with_scales(scale.clone(), ScaleSpec::Identity);
        let name = if k + 1 == steps.len() { format!("{} (gated)", tr.name()) } else { format!("{}{}", label(&s.channel), if s.above { "+" } else { "-" }) };
        parent = session.paste_gate(sample, parent, gate, &name)?;
        let _ = session.apply_to_group(sample, parent, ALL_SAMPLES);
    }
    Ok(parent)
}
