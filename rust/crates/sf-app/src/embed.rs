//! Dimensionality reduction, clustering and rule-based discovery over a pooled set of events, and the
//! step that turns approved results into populations of the gating tree.
//!
//! Pooling follows the engine's convention: every sample is downsampled to the SAME number of events
//! (seeded), because t-SNE/UMAP coordinates are per-run and a big sample would otherwise dominate. Clusters
//! are computed on the MARKER values, never on the 2-D coordinates (their distances are not metric).
//! Everything here is plain data in / plain data out so it can run on a worker thread.

use crate::session::Session;
use sf_core::cluster::{flowsom, FlowSom, Scale, Scaler};
use sf_core::discovery::{self, Rule};
use sf_core::graphcluster::phenograph;
use sf_core::kmeans::kmeans;
use sf_core::rng::Rng;
use sf_core::tsne::{tsne, TsneParams};
use sf_core::umap::{umap, UmapParams};
use sf_gating::{GateTree, NodeId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Method {
    Umap { n_neighbors: usize, min_dist: f64 },
    Tsne { perplexity: f64 },
}

impl Method {
    pub fn label(&self) -> &'static str {
        match self {
            Method::Umap { .. } => "UMAP",
            Method::Tsne { .. } => "t-SNE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FeatureTransform {
    None,
    Arcsinh(f64),
}

impl FeatureTransform {
    pub fn apply(self, v: f64) -> f64 {
        match self {
            FeatureTransform::None => v,
            FeatureTransform::Arcsinh(c) => (v / c).asinh(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct EmbedRequest {
    pub samples: Vec<usize>,
    /// Population path to restrict to in every sample (empty = all events).
    pub restrict: Vec<String>,
    pub per_sample: usize,
    pub features: Vec<String>,
    pub transform: FeatureTransform,
    pub method: Method,
    pub seed: u64,
    /// Technical noise removed from every sample before events are drawn.
    pub clean: crate::qc_clean::CleanSpec,
}

/// The pooled events, ready to embed.
#[derive(Debug, Clone)]
pub struct EmbedInput {
    /// `(sample index, row in that sample's file)` of every pooled event.
    pub rows: Vec<(usize, u32)>,
    /// Transformed marker values, `rows.len() x features.len()`.
    pub feats: Vec<f64>,
    pub request: EmbedRequest,
    pub notes: Vec<String>,
}

impl EmbedInput {
    pub fn n(&self) -> usize {
        self.rows.len()
    }
    pub fn d(&self) -> usize {
        self.request.features.len()
    }
    pub fn column(&self, j: usize) -> Vec<f64> {
        let d = self.d();
        (0..self.n()).map(|i| self.feats[i * d + j]).collect()
    }
}

#[derive(Debug, Clone)]
pub struct Embedding {
    pub input: EmbedInput,
    /// `n x 2`.
    pub xy: Vec<f64>,
}

fn restrict_node(session: &Session, sample: usize, path: &[String]) -> Option<NodeId> {
    if path.is_empty() {
        return Some(GateTree::ROOT);
    }
    let refs: Vec<&str> = path.iter().map(String::as_str).collect();
    session.tree(sample).find(&refs)
}

/// Pool `per_sample` events from every sample (equal N, seeded), restricted to a population if asked, and
/// extract the transformed marker matrix. Events with a non-finite marker value are dropped and counted.
pub fn build_input(session: &mut Session, req: &EmbedRequest) -> Result<EmbedInput, String> {
    if req.samples.is_empty() {
        return Err("choose at least one sample".into());
    }
    if req.features.len() < 2 {
        return Err("choose at least two marker channels".into());
    }
    if req.per_sample == 0 {
        return Err("events per sample must be at least 1".into());
    }
    let (mut rows, mut feats, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let mut dropped = 0usize;
    for (k, &si) in req.samples.iter().enumerate() {
        let name = session.sample(si).name.clone();
        let Some(node) = restrict_node(session, si, &req.restrict) else {
            notes.push(format!("{name}: has no population '{}', skipped", req.restrict.join(" > ")));
            continue;
        };
        let cols: Vec<usize> = req
            .features
            .iter()
            .map(|f| session.sample(si).channel_index(f).ok_or_else(|| format!("{name}: channel '{f}' is not in this sample")))
            .collect::<Result<_, _>>()?;
        let mut eligible: Vec<usize> = session.evaluation(si)?.masks.get(node).and_then(|m| m.as_ref()).ok_or("population has no events")?.indices();
        if req.clean.any() {
            let (keep, steps) = crate::qc_clean::keep_mask(session.sample(si), &req.clean);
            eligible.retain(|&r| keep[r]);
            if let Some(m) = crate::qc_clean::summary(&steps) {
                notes.push(format!("{name}: {m}"));
            }
            let heavy = crate::qc_clean::heavy_steps(&steps, session.sample(si).n_events);
            if !heavy.is_empty() {
                notes.push(format!("{name}: CHECK THIS: {} removed more than half of the file; untick that clean-up step if it is wrong for this sample", heavy.join(" and ")));
            }
        }
        if eligible.is_empty() {
            notes.push(format!("{name}: the population is empty, skipped"));
            continue;
        }
        let mut rng = Rng::new(req.seed.wrapping_add(0x9E37_79B9u64.wrapping_mul(k as u64 + 1)));
        let pick = rng.choose_sorted(eligible.len(), req.per_sample);
        let s = session.sample(si);
        let d = s.channels.len();
        for p in pick {
            let r = eligible[p];
            let vals: Vec<f64> = cols.iter().map(|&c| req.transform.apply(s.events[r * d + c] as f64)).collect();
            if vals.iter().all(|v| v.is_finite()) {
                rows.push((si, r as u32));
                feats.extend(vals);
            } else {
                dropped += 1;
            }
        }
    }
    if dropped > 0 {
        notes.push(format!("{dropped} event(s) with a non-finite marker value were dropped"));
    }
    if rows.len() < 50 {
        return Err(format!("only {} usable events: too few to embed", rows.len()));
    }
    Ok(EmbedInput { rows, feats, request: req.clone(), notes })
}

/// Run the embedding (CPU-heavy; call from a worker thread). Returns `n x 2` coordinates.
pub fn run_embedding(input: &EmbedInput) -> Result<Vec<f64>, String> {
    let (n, d) = (input.n(), input.d());
    match &input.request.method {
        Method::Umap { n_neighbors, min_dist } => umap(&input.feats, n, d, &UmapParams { n_neighbors: *n_neighbors, min_dist: *min_dist, seed: input.request.seed, ..Default::default() }),
        Method::Tsne { perplexity } => tsne(&input.feats, n, d, &TsneParams { perplexity: *perplexity, seed: input.request.seed, ..Default::default() }).map(|r| r.y),
    }
}

// ---- the map as channels ------------------------------------------------------------------------------

/// Name of the population that holds the events of a pooled map in each sample.
pub const MAP_POPULATION: &str = "Map events";

/// What [`attach_map`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct MapAttached {
    pub samples: usize,
    pub events: usize,
    /// `(sample, population)` of the first sample's map events, to open a plot on.
    pub first: Option<(usize, NodeId)>,
    /// Gates drawn on an EARLIER map that were turned into fixed event sets because the positions changed.
    pub frozen: usize,
}

/// Make a pooled map part of the gating system: every pooled sample gets the channels `Map-1` / `Map-2` (its
/// events' positions; NaN for events that were not pooled) and a population "Map events" holding exactly the
/// pooled events, under the population the map was made from. A plot of that population on the two map channels
/// IS the map, so every gate tool, group, comparison and export works on it. Attaching again replaces the earlier
/// map and its event list; gates drawn under "Map events" stay.
pub fn attach_map(session: &mut Session, emb: &Embedding) -> Result<MapAttached, String> {
    let input = &emb.input;
    let n = input.n();
    if emb.xy.len() != 2 * n {
        return Err("the map does not match its events".into());
    }
    let range = emb.xy.iter().fold(1.0_f64, |a, v| a.max(v.abs()));
    // the new positions of every pooled sample
    let mut plan: Vec<(usize, NodeId, Vec<(usize, u32)>, Vec<f32>, Vec<f32>)> = Vec::new();
    for &si in &input.request.samples {
        let rows: Vec<(usize, u32)> = input.rows.iter().enumerate().filter(|(_, r)| r.0 == si).map(|(i, r)| (i, r.1)).collect();
        if rows.is_empty() {
            continue;
        }
        let Some(parent) = restrict_node(session, si, &input.request.restrict) else { continue };
        let ne = session.sample(si).n_events;
        let (mut x, mut y) = (vec![f32::NAN; ne], vec![f32::NAN; ne]);
        for &(i, r) in &rows {
            x[r as usize] = emb.xy[2 * i] as f32;
            y[r as usize] = emb.xy[2 * i + 1] as f32;
        }
        plan.push((si, parent, rows, x, y));
    }
    // Is this the map the samples already carry? If any position differs (another seed, other markers, a map of
    // another population), gates drawn on the old map must not move onto the new one: freeze them first.
    let same = |s: &crate::sample::LoadedSample, x: &[f32], y: &[f32]| match (s.channel_index("Map-1"), s.channel_index("Map-2")) {
        (Some(cx), Some(cy)) => {
            let d = s.channels.len();
            (0..s.n_events).all(|r| s.events[r * d + cx].to_bits() == x[r].to_bits() && s.events[r * d + cy].to_bits() == y[r].to_bits())
        }
        _ => true, // no map yet: nothing to protect
    };
    let unchanged = plan.iter().all(|(si, _, _, x, y)| same(session.sample(*si), x, y)) && (0..session.samples().len()).all(|s| !session.sample(s).has_map() || plan.iter().any(|p| p.0 == s));
    let mut frozen = 0;
    if !unchanged {
        for s in 0..session.samples().len() {
            if session.sample(s).has_map() {
                frozen += session.freeze_gates_on(s, crate::sample::MAP_PREFIX)?;
                if !plan.iter().any(|p| p.0 == s) {
                    session.clear_map(s); // this sample is not part of the new map
                }
            }
        }
    }
    let (mut done, mut first) = (0, None);
    for (si, _, rows, x, y) in plan {
        // the population may have moved in the tree arena while gates were frozen: look it up again
        let Some(parent) = restrict_node(session, si, &input.request.restrict) else { continue };
        session.set_map(si, x, y, range)?;
        let node = session.set_index_population(si, parent, MAP_POPULATION, rows.iter().map(|r| r.1).collect())?;
        if first.is_none() {
            first = Some((si, node));
        }
        done += 1;
    }
    if done == 0 {
        return Err("none of the pooled samples is loaded any more".into());
    }
    let r = &input.request;
    session.map_features = r.features.clone();
    session.map_cofactor = match r.transform {
        FeatureTransform::None => 0.0,
        FeatureTransform::Arcsinh(c) => c,
    };
    session.map_info = Some(format!(
        "{} of {n} events pooled from {done} sample(s) ({} per sample{}), on {} ({}), seed {}",
        r.method.label(),
        r.per_sample,
        if r.restrict.is_empty() { String::new() } else { format!(", within {}", r.restrict.join(" > ")) },
        r.features.join(", "),
        match r.transform {
            FeatureTransform::None => "no transform".to_string(),
            FeatureTransform::Arcsinh(c) => format!("arcsinh, cofactor {c}"),
        },
        r.seed
    ));
    Ok(MapAttached { samples: done, events: n, first, frozen })
}

// ---- clustering -------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClusterMethod {
    FlowSom,
    PhenoGraph,
    KMeans,
}

impl ClusterMethod {
    pub const ALL: [ClusterMethod; 3] = [ClusterMethod::FlowSom, ClusterMethod::PhenoGraph, ClusterMethod::KMeans];
    pub fn label(self) -> &'static str {
        match self {
            ClusterMethod::FlowSom => "FlowSOM",
            ClusterMethod::PhenoGraph => "PhenoGraph",
            ClusterMethod::KMeans => "k-means",
        }
    }
    /// PhenoGraph chooses its own number of clusters.
    pub fn uses_k(self) -> bool {
        !matches!(self, ClusterMethod::PhenoGraph)
    }
}

#[derive(Debug, Clone)]
pub struct ClusterRequest {
    pub method: ClusterMethod,
    pub k: usize,
    pub scale: Scale,
    pub seed: u64,
}

/// What is needed to assign NEW events (the whole file) to the clusters. Only FlowSOM has one.
#[derive(Debug, Clone)]
pub struct ClusterModel {
    pub scaler: Scaler,
    pub flowsom: FlowSom,
}

#[derive(Debug, Clone)]
pub struct ClusterResult {
    /// Per pooled event; `None` = unclustered (PhenoGraph's small clusters).
    pub labels: Vec<Option<usize>>,
    pub n_clusters: usize,
    pub counts: Vec<usize>,
    /// Per-cluster median of every marker in the TRANSFORMED (unscaled) space, `k x d`.
    pub medians: Vec<f64>,
    pub requested: ClusterMethod,
    /// What actually ran. There is no silent fallback, so this always equals `requested`.
    pub actual: ClusterMethod,
    pub model: Option<ClusterModel>,
    /// How separate the clusters are in the space they were found in: the mean silhouette of an even subsample
    /// (-1..1). Near 0 the events form one continuous cloud and the "clusters" are only a way of cutting it up.
    pub separation: f64,
}

/// Mean silhouette of an even subsample of at most `max` clustered events (`x` is `n x d`).
pub fn separation(x: &[f64], d: usize, labels: &[Option<usize>], k: usize, max: usize) -> f64 {
    let idx: Vec<usize> = (0..labels.len()).filter(|&i| labels[i].is_some()).collect();
    if idx.len() < 3 || k < 2 {
        return 0.0;
    }
    let step = (idx.len() / max.max(1)).max(1);
    let pick: Vec<usize> = idx.into_iter().step_by(step).collect();
    let dist = |a: usize, b: usize| (0..d).map(|j| (x[a * d + j] - x[b * d + j]).powi(2)).sum::<f64>().sqrt();
    let (mut total, mut used) = (0.0, 0usize);
    for &i in &pick {
        let own = labels[i].unwrap();
        let (mut sum, mut cnt) = (vec![0.0f64; k], vec![0usize; k]);
        for &j in &pick {
            if j != i {
                let l = labels[j].unwrap();
                sum[l] += dist(i, j);
                cnt[l] += 1;
            }
        }
        if cnt[own] == 0 {
            continue; // alone in its cluster within the subsample: no silhouette
        }
        let a = sum[own] / cnt[own] as f64;
        let b = (0..k).filter(|&c| c != own && cnt[c] > 0).map(|c| sum[c] / cnt[c] as f64).fold(f64::INFINITY, f64::min);
        if b.is_finite() && a.max(b) > 0.0 {
            total += (b - a) / a.max(b);
            used += 1;
        }
    }
    if used == 0 { 0.0 } else { total / used as f64 }
}

/// What a separation value means, in words.
pub fn separation_verdict(s: f64) -> &'static str {
    if s < 0.15 {
        "weak: the events form one continuous cloud, and any method asked for k clusters will still cut it into k pieces"
    } else if s < 0.35 {
        "moderate: some real structure, with overlapping clusters"
    } else {
        "clear: the clusters are distinct groups"
    }
}

pub fn cluster(input: &EmbedInput, req: &ClusterRequest) -> Result<ClusterResult, String> {
    let (n, d) = (input.n(), input.d());
    let scaler = Scaler::fit(&input.feats, n, d, req.scale);
    let x = scaler.transform(&input.feats, n);
    let (labels, k, model): (Vec<Option<usize>>, usize, Option<ClusterModel>) = match req.method {
        ClusterMethod::FlowSom => {
            let grid = if n >= 2000 { 10 } else { 6 };
            let k = req.k.clamp(2, grid * grid);
            let fs = flowsom(&x, n, d, grid, grid, k, 10, req.seed)?;
            (fs.labels.iter().map(|&l| Some(l)).collect(), k, Some(ClusterModel { scaler, flowsom: fs }))
        }
        ClusterMethod::KMeans => {
            let k = req.k.clamp(2, n.max(2));
            let r = kmeans(&x, n, d, k, 5, 100, req.seed)?;
            (r.labels.iter().map(|&l| Some(l)).collect(), k, None)
        }
        ClusterMethod::PhenoGraph => {
            let r = phenograph(&x, n, d, 30.min(n - 1), 10, 5, req.seed)?;
            (r.labels, r.n_clusters, None)
        }
    };
    let mut counts = vec![0usize; k];
    for l in labels.iter().flatten() {
        counts[*l] += 1;
    }
    let (okf, okl): (Vec<usize>, Vec<usize>) = labels.iter().enumerate().filter_map(|(i, l)| l.map(|l| (i, l))).unzip();
    let flat: Vec<f64> = okf.iter().flat_map(|&i| input.feats[i * d..(i + 1) * d].to_vec()).collect();
    let medians = discovery::cluster_medians(&flat, okf.len(), d, &okl, k);
    let separation = separation(&x, d, &labels, k, 1500);
    Ok(ClusterResult { labels, n_clusters: k, counts, medians, requested: req.method, actual: req.method, model, separation })
}

// ---- clusters between groups ------------------------------------------------------------------------

/// One cluster's share of each sample's pooled events, by group, with a test between the groups.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupRow {
    pub cluster: usize,
    /// For each group: the percentage in every one of its pooled samples.
    pub percents: Vec<Vec<f64>>,
    /// Two-sided p (Mann-Whitney for two groups, Kruskal-Wallis for more), and the same multiplied by the number
    /// of clusters tested (Bonferroni, capped at 1). `None` when a group has fewer than two samples.
    pub p: Option<f64>,
    pub p_adjusted: Option<f64>,
}

/// Compare how common each cluster is between the sample groups. A sample's value is the percentage of ITS pooled
/// events that fell in the cluster, so every sample counts once whatever its size. Groups without a pooled sample
/// are left out. Returns the group names and one row per cluster.
pub fn clusters_by_group(session: &Session, input: &EmbedInput, res: &ClusterResult) -> (Vec<String>, Vec<GroupRow>) {
    let k = res.n_clusters;
    // per sample: pooled events, and how many in each cluster
    let mut total: BTreeMap<usize, f64> = BTreeMap::new();
    let mut inside: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    for (i, &(s, _)) in input.rows.iter().enumerate() {
        *total.entry(s).or_default() += 1.0;
        if let Some(c) = res.labels[i] {
            inside.entry(s).or_insert_with(|| vec![0.0; k])[c] += 1.0;
        }
    }
    let groups: Vec<(String, Vec<usize>)> = session.groups.iter().map(|g| (g.name.clone(), session.member_indices(&g.name).into_iter().filter(|s| total.contains_key(s)).collect::<Vec<_>>())).filter(|(_, m)| !m.is_empty()).collect();
    let names: Vec<String> = groups.iter().map(|g| g.0.clone()).collect();
    let testable = groups.len() >= 2 && groups.iter().all(|g| g.1.len() >= 2);
    let rows = (0..k)
        .map(|c| {
            let percents: Vec<Vec<f64>> = groups.iter().map(|(_, m)| m.iter().map(|s| 100.0 * inside.get(s).map_or(0.0, |v| v[c]) / total[s]).collect()).collect();
            let p = if !testable {
                None
            } else if groups.len() == 2 {
                sf_core::htest::mann_whitney_u(&percents[0], &percents[1]).ok().map(|t| t.p)
            } else {
                let refs: Vec<&[f64]> = percents.iter().map(|v| v.as_slice()).collect();
                sf_core::htest::kruskal_wallis(&refs).ok().map(|t| t.p)
            };
            GroupRow { cluster: c, percents, p, p_adjusted: p.map(|p| (p * k as f64).min(1.0)) }
        })
        .collect();
    (names, rows)
}

// ---- naming and rules -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMethod {
    /// Positive when a cluster's median is above the average cluster (no thresholds needed).
    ZScore,
    OtsuEvents,
    GlobalMedian,
    /// Positive when the cluster median exceeds the per-marker threshold the user set.
    Threshold,
}

impl NameMethod {
    pub const ALL: [NameMethod; 4] = [NameMethod::ZScore, NameMethod::OtsuEvents, NameMethod::GlobalMedian, NameMethod::Threshold];
    pub fn label(self) -> &'static str {
        match self {
            NameMethod::ZScore => "Relative to other clusters (z-score)",
            NameMethod::OtsuEvents => "Otsu split of each marker",
            NameMethod::GlobalMedian => "Above the marker's median",
            NameMethod::Threshold => "My marker thresholds",
        }
    }
}

/// Per-marker default threshold: Otsu on the pooled events.
pub fn default_thresholds(input: &EmbedInput) -> BTreeMap<String, f64> {
    input.request.features.iter().enumerate().map(|(j, f)| (f.clone(), sf_core::stats::otsu(&input.column(j)))).collect()
}

/// Name every cluster from its marker medians. `tag_of` maps a channel to its marker name.
pub fn name_clusters(input: &EmbedInput, res: &ClusterResult, how: NameMethod, thresholds: &BTreeMap<String, f64>, rules: &[Rule], tag_of: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let (d, k) = (input.d(), res.n_clusters);
    let feats = &input.request.features;
    let all_med: Vec<Vec<f64>> = (0..k).map(|c| res.medians[c * d..(c + 1) * d].to_vec()).collect();
    let (method, cuts): (discovery::CallMethod, Option<Vec<f64>>) = match how {
        NameMethod::ZScore => (discovery::CallMethod::ZScore, None),
        NameMethod::OtsuEvents => (discovery::CallMethod::OtsuEvents, Some((0..d).map(|j| sf_core::stats::otsu(&input.column(j))).collect())),
        NameMethod::GlobalMedian => (discovery::CallMethod::GlobalMedian, Some((0..d).map(|j| sf_core::stats::median(&input.column(j))).collect())),
        // a marker with no threshold set can never be called positive
        NameMethod::Threshold => (discovery::CallMethod::Threshold, Some(feats.iter().map(|f| thresholds.get(f).copied().unwrap_or(f64::INFINITY)).collect())),
    };
    (0..k)
        .map(|c| {
            let calls = discovery::relative_calls(feats, &all_med[c], &all_med, cuts.as_deref(), method);
            discovery::name_cluster(&calls, rules, tag_of, &format!("Cluster {}", c + 1))
        })
        .collect()
}

/// Which pooled events satisfy each rule (thresholds in transformed space).
pub fn rule_membership(input: &EmbedInput, rules: &[Rule], thresholds: &BTreeMap<String, f64>) -> Vec<Vec<bool>> {
    rules.iter().map(|r| discovery::evaluate(&input.feats, input.n(), &input.request.features, r, thresholds)).collect()
}

// ---- approval into populations ----------------------------------------------------------------------

/// Which events an approved population covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only the pooled (downsampled) events that were embedded.
    Embedded,
    /// Every event of each sample (FlowSOM clusters via the trained map; rules directly from the thresholds).
    FullFile,
}

fn feature_matrix(session: &Session, sample: usize, features: &[String], tf: FeatureTransform) -> Result<Vec<f64>, String> {
    let s = session.sample(sample);
    let cols: Vec<usize> = features.iter().map(|f| s.channel_index(f).ok_or_else(|| format!("{}: channel '{f}' is not in this sample", s.name))).collect::<Result<_, _>>()?;
    let d = s.channels.len();
    Ok((0..s.n_events).flat_map(|r| cols.iter().map(move |&c| (r, c))).map(|(r, c)| tf.apply(s.events[r * d + c] as f64)).collect())
}

/// Row sets per sample for each selected group, grouped for [`Session::add_index_populations`].
#[derive(Debug, Clone, Copy)]
pub struct Approval {
    pub populations: usize,
    pub samples: usize,
}

/// Approve clusters (`which[c]` selects) as populations named `names[c]` under the root of every pooled sample.
pub fn approve_clusters(session: &mut Session, input: &EmbedInput, res: &ClusterResult, names: &[String], which: &[bool], scope: Scope) -> Result<Approval, String> {
    if scope == Scope::FullFile && res.model.is_none() {
        return Err(format!("{} cannot place new events; use the embedded events, or cluster with FlowSOM", res.actual.label()));
    }
    let d = input.d();
    let mut pops = 0;
    let mut touched = 0;
    for &si in &input.request.samples {
        let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
        match scope {
            Scope::Embedded => {
                let mut per: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
                for (i, &(s, r)) in input.rows.iter().enumerate() {
                    if s != si {
                        continue;
                    }
                    if let Some(c) = res.labels[i] {
                        if which.get(c).copied().unwrap_or(false) {
                            per.entry(c).or_default().push(r);
                        }
                    }
                }
                for (c, rows) in per {
                    groups.push((names[c].clone(), rows));
                }
            }
            Scope::FullFile => {
                let model = res.model.as_ref().unwrap();
                let x = feature_matrix(session, si, &input.request.features, input.request.transform)?;
                let n = x.len() / d;
                let labels = model.flowsom.predict(&model.scaler.transform(&x, n), n)?;
                let mut per: BTreeMap<usize, Vec<u32>> = BTreeMap::new();
                for (r, l) in labels.into_iter().enumerate() {
                    if let Some(c) = l {
                        if which.get(c).copied().unwrap_or(false) {
                            per.entry(c).or_default().push(r as u32);
                        }
                    }
                }
                for (c, rows) in per {
                    groups.push((names[c].clone(), rows));
                }
            }
        }
        if groups.is_empty() {
            continue;
        }
        // two clusters with the same phenotype name must not overwrite each other
        let mut seen = std::collections::HashSet::new();
        for (name, _) in groups.iter_mut() {
            if !seen.insert(name.clone()) {
                let base = name.clone();
                let mut k = 2;
                while !seen.insert(format!("{base} ({k})")) {
                    k += 1;
                }
                *name = format!("{base} ({k})");
            }
        }
        pops += groups.len();
        touched += 1;
        session.add_index_populations(si, GateTree::ROOT, groups)?;
    }
    if pops == 0 {
        return Err("no events fall in the selected clusters".into());
    }
    Ok(Approval { populations: pops, samples: touched })
}

/// Approve rules as populations. `FullFile` evaluates the rule on every event of each sample.
pub fn approve_rules(session: &mut Session, input: &EmbedInput, rules: &[Rule], which: &[bool], thresholds: &BTreeMap<String, f64>, scope: Scope) -> Result<Approval, String> {
    let d = input.d();
    let member = rule_membership(input, rules, thresholds);
    let (mut pops, mut touched) = (0, 0);
    for &si in &input.request.samples {
        let mut groups: Vec<(String, Vec<u32>)> = Vec::new();
        for (ri, rule) in rules.iter().enumerate() {
            if !which.get(ri).copied().unwrap_or(false) {
                continue;
            }
            let rows: Vec<u32> = match scope {
                Scope::Embedded => input.rows.iter().enumerate().filter(|(i, &(s, _))| s == si && member[ri][*i]).map(|(_, &(_, r))| r).collect(),
                Scope::FullFile => {
                    let x = feature_matrix(session, si, &input.request.features, input.request.transform)?;
                    let n = x.len() / d;
                    discovery::evaluate(&x, n, &input.request.features, rule, thresholds).into_iter().enumerate().filter(|(_, m)| *m).map(|(r, _)| r as u32).collect()
                }
            };
            if !rows.is_empty() {
                groups.push((rule.name.clone(), rows));
            }
        }
        if groups.is_empty() {
            continue;
        }
        pops += groups.len();
        touched += 1;
        session.add_index_populations(si, GateTree::ROOT, groups)?;
    }
    if pops == 0 {
        return Err("no events satisfy the selected rules (are the thresholds set?)".into());
    }
    Ok(Approval { populations: pops, samples: touched })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::LoadedSample;
    use sf_core::cluster::adjusted_rand_index;

    const CENTRES: [[f64; 4]; 4] = [[5.0, 5.0, 0.0, 0.0], [5.0, 0.0, 5.0, 0.0], [5.0, 5.0, 0.0, 5.0], [0.0, 0.0, 0.0, 0.0]];

    fn normal(rng: &mut Rng) -> f64 {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// Two samples of `n` events, markers CD3 CD4 CD8 FoxP3 in arcsinh-ish raw units (cofactor 1 so transform = asinh).
    fn session(n: usize) -> (Session, Vec<usize>) {
        let mut s = Session::new();
        let mut truth = Vec::new();
        for (k, name) in ["a.fcs", "b.fcs"].iter().enumerate() {
            let mut rng = Rng::new(10 + k as u64);
            let mut ev = Vec::new();
            for i in 0..n {
                let c = i % 4;
                truth.push(c);
                for m in 0..4 {
                    let v = CENTRES[c][m] + 0.5 * normal(&mut rng);
                    ev.push((v * 100.0 + 50.0) as f32); // raw units: roughly 100 x display value
                }
            }
            s.add_sample(LoadedSample::synthetic(name, &["CD3", "CD4", "CD8", "FoxP3"], vec![1e5; 4], ev));
        }
        (s, truth)
    }

    fn req(samples: Vec<usize>, per: usize) -> EmbedRequest {
        EmbedRequest { samples, restrict: vec![], per_sample: per, features: ["CD3", "CD4", "CD8", "FoxP3"].iter().map(|s| s.to_string()).collect(), transform: FeatureTransform::Arcsinh(150.0), method: Method::Umap { n_neighbors: 15, min_dist: 0.1 }, seed: 7, clean: Default::default() }
    }

    #[test]
    fn technical_noise_is_removed_before_events_are_drawn_and_the_removal_is_reported() {
        // 2000 events: every fifth is a doublet (area twice the height-implied area) and every tenth is dead
        let mut s = Session::new();
        let mut rng = Rng::new(3);
        let mut ev = Vec::new();
        for i in 0..2000 {
            let a = 100_000.0 + 8_000.0 * normal(&mut rng);
            let (fa, fh) = if i % 5 == 0 { (a * 1.9, a * 0.8) } else { (a, a * 0.8) };
            let via = if i % 10 == 3 { 9000.0 } else { 80.0 + 30.0 * normal(&mut rng) };
            ev.extend_from_slice(&[fa as f32, fh as f32, via as f32, (500.0 + 100.0 * normal(&mut rng)) as f32, (500.0 + 100.0 * normal(&mut rng)) as f32]);
        }
        s.add_sample(LoadedSample::synthetic("n.fcs", &["FSC-A", "FSC-H", "Viability-A", "CD3", "CD4"], vec![262_144.0; 5], ev));
        let mut r = req(vec![0], 2000);
        r.features = vec!["CD3".into(), "CD4".into()];
        let plain = build_input(&mut s, &r).unwrap();
        assert_eq!(plain.n(), 2000);
        r.clean = crate::qc_clean::CleanSpec { doublets: true, dead: Some(crate::qc_clean::DeadSpec { channel: "Viability-A".into(), threshold: None }), ..Default::default() };
        let cleaned = build_input(&mut s, &r).unwrap();
        assert!(cleaned.n() < 1500 && cleaned.n() > 1100, "kept {}", cleaned.n());
        assert!(cleaned.rows.iter().all(|&(_, row)| row % 5 != 0 && row % 10 != 3), "no flagged event is pooled");
        assert!(cleaned.notes.iter().any(|n| n.contains("doublets") && n.contains("dead cells")), "{:?}", cleaned.notes);
    }

    #[test]
    fn pooling_is_equal_n_seeded_restrictable_and_reports_what_it_skipped() {
        let (mut s, _) = session(1200);
        let a = build_input(&mut s, &req(vec![0, 1], 300)).unwrap();
        assert_eq!(a.n(), 600);
        assert_eq!(a.rows.iter().filter(|r| r.0 == 0).count(), 300, "equal N per sample");
        assert_eq!(a.rows.iter().filter(|r| r.0 == 1).count(), 300);
        let b = build_input(&mut s, &req(vec![0, 1], 300)).unwrap();
        assert_eq!(a.rows, b.rows, "same seed, same pool");
        assert_eq!(a.feats, b.feats);
        let mut other = req(vec![0, 1], 300);
        other.seed = 8;
        assert_ne!(build_input(&mut s, &other).unwrap().rows, a.rows);
        // restrict to a gated population in sample 0 only: sample 1 lacks it and is reported, not silently skipped
        let w = s.open_population(0, GateTree::ROOT).unwrap();
        {
            let win = s.window_mut(w).unwrap();
            win.x_chan = "CD3".into();
            win.y_chan = "CD4".into();
            win.x_scale = sf_core::scale::ScaleSpec::Identity;
            win.y_scale = sf_core::scale::ScaleSpec::Identity;
        }
        s.commit_shape(w, &crate::drawing::Shape::Rect { x0: 300.0, x1: 1e6, y0: -1e9, y1: 1e9 }, "T").unwrap();
        let mut r = req(vec![0, 1], 10_000);
        r.restrict = vec!["T".into()];
        let c = build_input(&mut s, &r).unwrap();
        assert!(c.rows.iter().all(|r| r.0 == 0));
        assert!(c.n() > 400 && c.n() < 1000, "about three of the four phenotypes are CD3+: {}", c.n());
        assert!(c.notes.iter().any(|n| n.contains("b.fcs") && n.contains("'T'")), "{:?}", c.notes);
        // bad requests
        assert!(build_input(&mut s, &req(vec![], 10)).is_err());
        let mut one = req(vec![0], 100);
        one.features = vec!["CD3".into()];
        assert!(build_input(&mut s, &one).unwrap_err().contains("two marker"));
        let mut missing = req(vec![0], 100);
        missing.features.push("BV421".into());
        assert!(build_input(&mut s, &missing).unwrap_err().contains("BV421"));
        assert!(build_input(&mut s, &req(vec![0], 20)).unwrap_err().contains("too few"));
    }

    #[test]
    fn umap_separates_the_phenotypes_and_flowsom_finds_them_on_the_markers() {
        let (mut s, truth) = session(1500);
        let input = build_input(&mut s, &req(vec![0, 1], 1000)).unwrap();
        let xy = run_embedding(&input).unwrap();
        assert_eq!(xy.len(), input.n() * 2);
        let km = kmeans(&xy, input.n(), 2, 4, 5, 100, 1).unwrap();
        let t: Vec<usize> = input.rows.iter().map(|&(si, r)| truth[si * 1500 + r as usize]).collect();
        let ari_map = adjusted_rand_index(&km.labels, &t);
        assert!(ari_map > 0.95, "the four phenotypes form four islands (ARI {ari_map})");
        for method in ClusterMethod::ALL {
            let res = cluster(&input, &ClusterRequest { method, k: 4, scale: Scale::ZScore, seed: 3 }).unwrap();
            assert_eq!((res.requested, res.actual), (method, method), "no silent substitution");
            let (l, tr): (Vec<usize>, Vec<usize>) = res.labels.iter().zip(&t).filter_map(|(l, t)| l.map(|l| (l, *t))).unzip();
            if method == ClusterMethod::PhenoGraph {
                // it picks its own cluster count and may split a phenotype into sub-clusters, so ARI against 4 classes is
                // the wrong yardstick: what matters is that no cluster mixes phenotypes (purity)
                let mut best = std::collections::HashMap::<(usize, usize), usize>::new();
                for (a, b) in l.iter().zip(&tr) {
                    *best.entry((*a, *b)).or_default() += 1;
                }
                let mut per_cluster_max = std::collections::HashMap::<usize, usize>::new();
                for ((c, _), n) in &best {
                    let e = per_cluster_max.entry(*c).or_default();
                    *e = (*e).max(*n);
                }
                let purity = per_cluster_max.values().sum::<usize>() as f64 / l.len() as f64;
                assert!(purity > 0.97, "PhenoGraph purity {purity}");
            } else {
                let ari = adjusted_rand_index(&l, &tr);
                assert!(ari > 0.9, "{method:?}: ARI {ari}");
            }
            assert_eq!(res.counts.iter().sum::<usize>(), l.len());
            assert_eq!(res.model.is_some(), method == ClusterMethod::FlowSom);
        }
    }

    #[test]
    fn clusters_are_named_from_their_markers_by_every_method() {
        let (mut s, _) = session(1000);
        let input = build_input(&mut s, &req(vec![0], 800)).unwrap();
        let res = cluster(&input, &ClusterRequest { method: ClusterMethod::KMeans, k: 4, scale: Scale::ZScore, seed: 2 }).unwrap();
        let rules = vec![Rule::new("Treg", &[("CD3", true), ("CD4", true), ("FoxP3", true)]), Rule::new("CD8 T", &[("CD3", true), ("CD8", true)]), Rule::new("CD4 T", &[("CD3", true), ("CD4", true)])];
        let thr = default_thresholds(&input);
        for how in [NameMethod::OtsuEvents, NameMethod::Threshold] {
            let names = name_clusters(&input, &res, how, &thr, &rules, &|c| Some(c.to_string()));
            let mut sorted = names.clone();
            sorted.sort();
            assert!(names.contains(&"Treg".to_string()) && names.contains(&"CD8 T".to_string()) && names.contains(&"CD4 T".to_string()), "{how:?}: {names:?}");
            assert_eq!(sorted.iter().filter(|n| n.starts_with("Cluster ")).count(), 1, "{how:?}: only the non-T cluster has no rule: {names:?}");
        }
        // unset thresholds: nothing can be positive, so everything falls back to plain names
        let none = name_clusters(&input, &res, NameMethod::Threshold, &BTreeMap::new(), &rules, &|c| Some(c.to_string()));
        assert!(none.iter().all(|n| n.starts_with("Cluster ")), "{none:?}");
    }

    #[test]
    fn approving_clusters_adds_embedded_or_whole_file_populations_with_one_undo_step() {
        let (mut s, truth) = session(2000);
        let input = build_input(&mut s, &req(vec![0, 1], 500)).unwrap();
        let res = cluster(&input, &ClusterRequest { method: ClusterMethod::FlowSom, k: 4, scale: Scale::ZScore, seed: 5 }).unwrap();
        let names: Vec<String> = (0..4).map(|c| format!("C{c}")).collect();
        let all = vec![true; 4];
        // embedded scope: only the pooled events
        let a = approve_clusters(&mut s, &input, &res, &names, &all, Scope::Embedded).unwrap();
        assert_eq!((a.populations, a.samples), (8, 2));
        let total: usize = (0..4).map(|c| s.count(0, s.tree(0).find(&[&format!("C{c}")]).unwrap())).sum();
        assert_eq!(total, 500, "sample 0 contributed 500 pooled events");
        // one undo removes all four populations of the sample
        assert!(s.undo(0));
        assert!(s.tree(0).children(GateTree::ROOT).unwrap().is_empty());
        assert_eq!(s.tree(1).children(GateTree::ROOT).unwrap().len(), 4, "the other sample is untouched by that undo");
        // whole-file scope: every event, trained on the pool; accuracy against the truth on all 2000 events
        approve_clusters(&mut s, &input, &res, &names, &all, Scope::FullFile).unwrap();
        let mut labels = vec![usize::MAX; 2000];
        for c in 0..4 {
            let node = s.tree(0).find(&[&format!("C{c}")]).unwrap();
            for r in s.evaluation(0).unwrap().masks[node].as_ref().unwrap().indices() {
                labels[r] = c;
            }
        }
        assert!(labels.iter().all(|&l| l != usize::MAX), "every event of the file is in a cluster population");
        assert!(adjusted_rand_index(&labels, &truth[..2000]) > 0.95);
        // re-approving replaces, never stacks
        approve_clusters(&mut s, &input, &res, &names, &all, Scope::FullFile).unwrap();
        assert_eq!(s.tree(0).children(GateTree::ROOT).unwrap().len(), 4);
        // methods that cannot place new events say so
        let km = cluster(&input, &ClusterRequest { method: ClusterMethod::KMeans, k: 4, scale: Scale::ZScore, seed: 5 }).unwrap();
        assert!(approve_clusters(&mut s, &input, &km, &names, &all, Scope::FullFile).unwrap_err().contains("FlowSOM"));
        // same names for two clusters get a suffix
        let dup = vec!["X".to_string(); 4];
        let (mut s2, _) = session(800);
        let inp2 = build_input(&mut s2, &req(vec![0], 400)).unwrap();
        let r2 = cluster(&inp2, &ClusterRequest { method: ClusterMethod::KMeans, k: 4, scale: Scale::ZScore, seed: 1 }).unwrap();
        approve_clusters(&mut s2, &inp2, &r2, &dup, &all, Scope::Embedded).unwrap();
        let kids = s2.tree(0).children(GateTree::ROOT).unwrap().len();
        assert_eq!(kids, 4, "all four survived under distinct names");
    }

    #[test]
    fn rules_are_label_based_discovery_independent_of_clustering() {
        let (mut s, truth) = session(2000);
        let input = build_input(&mut s, &req(vec![0], 1000)).unwrap();
        let thr = default_thresholds(&input);
        let rules = vec![Rule::new("Treg", &[("CD3", true), ("CD4", true), ("FoxP3", true)]), Rule::new("CD8 T", &[("CD3", true), ("CD8", true)])];
        let m = rule_membership(&input, &rules, &thr);
        let t: Vec<usize> = input.rows.iter().map(|&(si, r)| truth[si * 2000 + r as usize]).collect();
        let (treg_true, hit): (usize, usize) = (t.iter().filter(|&&c| c == 2).count(), m[0].iter().zip(&t).filter(|(m, &c)| **m && c == 2).count());
        assert!(hit as f64 > 0.97 * treg_true as f64, "rule recall {hit}/{treg_true}");
        assert!(m[0].iter().zip(&t).filter(|(m, &c)| **m && c != 2).count() < 10, "and almost no false positives");
        // approving full-file rules gives populations evaluated on all 2000 events
        let a = approve_rules(&mut s, &input, &rules, &[true, true], &thr, Scope::FullFile).unwrap();
        assert_eq!((a.populations, a.samples), (2, 1));
        let n_treg = s.count(0, s.tree(0).find(&["Treg"]).unwrap());
        let want = truth[..2000].iter().filter(|&&c| c == 2).count();
        assert!((n_treg as f64 - want as f64).abs() < 0.03 * want as f64, "{n_treg} vs {want}");
        // no thresholds -> refused with a reason
        assert!(approve_rules(&mut s, &input, &rules, &[true, true], &BTreeMap::new(), Scope::Embedded).unwrap_err().contains("thresholds"));
    }

    #[test]
    fn tsne_also_runs_on_the_pool() {
        let (mut s, _) = session(600);
        let mut r = req(vec![0, 1], 300);
        r.method = Method::Tsne { perplexity: 20.0 };
        let input = build_input(&mut s, &r).unwrap();
        let xy = run_embedding(&input).unwrap();
        assert!(xy.iter().all(|v| v.is_finite()) && xy.len() == 1200);
        assert_eq!(input.request.method.label(), "t-SNE");
    }
}
