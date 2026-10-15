//! Cluster discovery and label-based (rule) discovery: the Rust port of `EmbeddingRules` plus the cluster
//! summaries the app builds around FlowSOM.
//!
//! * [`cluster_medians`]: per-cluster median of every marker (the numbers the heat map and the naming use).
//! * [`relative_calls`] / [`name_cluster`]: call each marker positive/negative for a cluster, then name it
//!   (first matching [`Rule`] wins; otherwise the positive tags; 2 positives = "Mixed: A+ B+", more = "Ambiguous (n)").
//! * [`evaluate`] / [`labels`] / [`counts`]: user rules such as `Treg = CD4+ AND FoxP3+` applied to events.
//!   A rule with an unknown channel, an unset threshold or no clauses matches nothing — an empty population,
//!   never a wrong one. Labels colour by the first matching rule; counts are per-rule membership.
//!
//! Semantics are those of the Java `EmbeddingRules` (its tests are mirrored below), so existing rules and
//! names keep their meaning.

use crate::stats;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallMethod {
    /// Positive when the cluster median exceeds a per-marker cut (a threshold, Otsu cut or global median).
    Threshold,
    /// Positive when the cluster median is above the average cluster, judged against the spread between clusters.
    ZScore,
    OtsuEvents,
    GlobalMedian,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clause {
    pub channel: String,
    pub positive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub name: String,
    pub clauses: Vec<Clause>,
}

impl Rule {
    pub fn new(name: &str, clauses: &[(&str, bool)]) -> Rule {
        Rule { name: name.to_string(), clauses: clauses.iter().map(|(c, p)| Clause { channel: c.to_string(), positive: *p }).collect() }
    }
}

/// Median of every column for each cluster, `k x d` row-major. An empty cluster yields zeros (as the app does).
pub fn cluster_medians(data: &[f64], n: usize, d: usize, labels: &[usize], k: usize) -> Vec<f64> {
    assert_eq!(data.len(), n * d);
    assert_eq!(labels.len(), n);
    let mut out = vec![0.0; k * d];
    for c in 0..k {
        let rows: Vec<usize> = (0..n).filter(|&i| labels[i] == c).collect();
        if rows.is_empty() {
            continue;
        }
        for j in 0..d {
            let col: Vec<f64> = rows.iter().map(|&i| data[i * d + j]).collect();
            out[c * d + j] = stats::median(&col);
        }
    }
    out
}

/// Positive/negative call per channel for one cluster. `all_med[c][j]` is every cluster's median (needed for
/// `ZScore`); `event_cut[j]` is the per-channel cut for the cut-based methods.
pub fn relative_calls(channels: &[String], cluster_med: &[f64], all_med: &[Vec<f64>], event_cut: Option<&[f64]>, method: CallMethod) -> Vec<(String, bool)> {
    let mut calls = Vec::with_capacity(channels.len());
    for (j, ch) in channels.iter().enumerate() {
        let pos = match method {
            CallMethod::ZScore => {
                let n = all_med.len() as f64;
                let mean = all_med.iter().map(|m| m[j]).sum::<f64>() / n;
                let var = all_med.iter().map(|m| (m[j] - mean) * (m[j] - mean)).sum::<f64>() / n;
                let sd = var.sqrt();
                sd > 0.0 && (cluster_med[j] - mean) / sd > 0.0
            }
            CallMethod::Threshold | CallMethod::OtsuEvents | CallMethod::GlobalMedian => event_cut.map_or(false, |c| cluster_med[j] > c[j]),
        };
        calls.push((ch.clone(), pos));
    }
    calls
}

/// Name a cluster from its calls. `tag_of` maps a channel to its marker name (e.g. "BV421-A" -> "CD4").
pub fn name_cluster(calls: &[(String, bool)], rules: &[Rule], tag_of: &dyn Fn(&str) -> Option<String>, fallback: &str) -> String {
    for r in rules {
        if r.clauses.is_empty() {
            continue;
        }
        let all = r.clauses.iter().all(|c| calls.iter().find(|(ch, _)| *ch == c.channel).map_or(false, |(_, p)| *p == c.positive));
        if all {
            return r.name.clone();
        }
    }
    let pos: Vec<String> = calls.iter().filter(|(_, p)| *p).map(|(ch, _)| format!("{}+", tag_of(ch).unwrap_or_else(|| ch.clone()))).collect();
    // more than two simultaneous positives is far more likely a boundary/doublet artefact than a phenotype
    match pos.len() {
        0 => fallback.to_string(),
        1 => pos[0].clone(),
        2 => format!("Mixed: {}", pos.join(" ")),
        n => format!("Ambiguous ({n})"),
    }
}

/// The per-channel cut for a cut-based method (`Otsu` / `GlobalMedian`) from the events themselves.
pub fn event_cut(values: &[f64], method: CallMethod) -> Option<f64> {
    match method {
        CallMethod::OtsuEvents => Some(stats::otsu(values)),
        CallMethod::GlobalMedian => Some(stats::median(values)),
        _ => None,
    }
}

/// Events satisfying every clause (strictly above the threshold for `+`, otherwise below-or-equal).
pub fn evaluate(data: &[f64], n: usize, channels: &[String], rule: &Rule, thresholds: &BTreeMap<String, f64>) -> Vec<bool> {
    let d = channels.len();
    assert_eq!(data.len(), n * d);
    let mut out = vec![false; n];
    if rule.clauses.is_empty() {
        return out;
    }
    let mut cols = Vec::new();
    for c in &rule.clauses {
        match (channels.iter().position(|ch| *ch == c.channel), thresholds.get(&c.channel)) {
            (Some(j), Some(&t)) => cols.push((j, t, c.positive)),
            _ => return out,
        }
    }
    for (r, o) in out.iter_mut().enumerate() {
        *o = cols.iter().all(|&(j, t, want)| (data[r * d + j] > t) == want);
    }
    out
}

/// Rule index of the FIRST rule each event matches, or `None`.
pub fn labels(data: &[f64], n: usize, channels: &[String], rules: &[Rule], thresholds: &BTreeMap<String, f64>) -> Vec<Option<usize>> {
    let mut lab = vec![None; n];
    for (i, r) in rules.iter().enumerate().rev() {
        for (row, m) in evaluate(data, n, channels, r, thresholds).into_iter().enumerate() {
            if m {
                lab[row] = Some(i);
            }
        }
    }
    lab
}

/// Per-rule membership counts (an event can belong to several rules even though it is painted once).
pub fn counts(data: &[f64], n: usize, channels: &[String], rules: &[Rule], thresholds: &BTreeMap<String, f64>) -> Vec<(String, usize)> {
    rules.iter().map(|r| (r.name.clone(), evaluate(data, n, channels, r, thresholds).into_iter().filter(|&b| b).count())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn ident(c: &str) -> Option<String> {
        Some(c.to_string())
    }

    fn thr() -> BTreeMap<String, f64> {
        [("CD4".to_string(), 0.0), ("FoxP3".to_string(), 0.0)].into_iter().collect()
    }

    /// columns: tSNE 1, tSNE 2, CD4, FoxP3
    fn grid() -> (Vec<f64>, Vec<String>) {
        let rows = [[0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 1.0, -1.0], [0.0, 0.0, -1.0, 1.0], [0.0, 0.0, -1.0, -1.0]];
        (rows.iter().flatten().cloned().collect(), s(&["tSNE 1", "tSNE 2", "CD4", "FoxP3"]))
    }

    #[test]
    fn zscore_calls_clusters_above_the_average_cluster_positive() {
        let all = vec![vec![100.0, 5.0], vec![10.0, 200.0], vec![12.0, 6.0]];
        let ch = s(&["CD4", "FoxP3"]);
        let c0 = relative_calls(&ch, &all[0], &all, None, CallMethod::ZScore);
        assert_eq!((c0[0].1, c0[1].1), (true, false));
        let c1 = relative_calls(&ch, &all[1], &all, None, CallMethod::ZScore);
        assert_eq!((c1[0].1, c1[1].1), (false, true));
        // identical clusters: sd = 0 -> never positive
        let flat = vec![vec![5.0, 5.0]; 3];
        assert!(relative_calls(&ch, &flat[0], &flat, None, CallMethod::ZScore).iter().all(|c| !c.1));
    }

    #[test]
    fn cut_based_calls_use_the_given_cut() {
        let c = relative_calls(&s(&["CD4", "FoxP3"]), &[100.0, 10.0], &[], Some(&[50.0, 50.0]), CallMethod::OtsuEvents);
        assert_eq!((c[0].1, c[1].1), (true, false));
        assert!(relative_calls(&s(&["CD4"]), &[100.0], &[], None, CallMethod::Threshold)[0].1 == false, "no cut -> never positive");
    }

    #[test]
    fn naming_follows_the_first_matching_rule_then_tags_then_the_caps() {
        let rules = vec![Rule::new("Treg", &[("CD4", true), ("FoxP3", true)]), Rule::new("Tconv", &[("CD4", true), ("FoxP3", false)])];
        let both = vec![("CD4".to_string(), true), ("FoxP3".to_string(), true)];
        assert_eq!(name_cluster(&both, &rules, &ident, "Cluster"), "Treg");
        let cd4 = vec![("CD4".to_string(), true), ("FoxP3".to_string(), false)];
        assert_eq!(name_cluster(&cd4, &[], &ident, "Cluster"), "CD4+");
        let none = vec![("CD4".to_string(), false), ("FoxP3".to_string(), false)];
        assert_eq!(name_cluster(&none, &[], &ident, "Cluster"), "Cluster");
        let two = vec![("CD4".to_string(), true), ("CD8".to_string(), true), ("CD19".to_string(), false)];
        assert_eq!(name_cluster(&two, &[], &ident, "Cluster"), "Mixed: CD4+ CD8+");
        let four: Vec<_> = ["CD3", "CD4", "CD8", "CD19"].iter().map(|c| (c.to_string(), true)).collect();
        assert_eq!(name_cluster(&four, &[], &ident, "Cluster"), "Ambiguous (4)");
        // the tag mapper renames channels to markers
        let mapped = name_cluster(&cd4, &[], &|c| (c == "CD4").then(|| "BV421".to_string()), "x");
        assert_eq!(mapped, "BV421+");
        // a rule with no clauses never matches
        assert_eq!(name_cluster(&cd4, &[Rule::new("Empty", &[])], &ident, "x"), "CD4+");
    }

    #[test]
    fn rules_and_their_clauses() {
        let (d, ch) = grid();
        assert_eq!(evaluate(&d, 4, &ch, &Rule::new("Treg", &[("CD4", true), ("FoxP3", true)]), &thr()), vec![true, false, false, false]);
        assert_eq!(evaluate(&d, 4, &ch, &Rule::new("Tconv", &[("CD4", true), ("FoxP3", false)]), &thr()), vec![false, true, false, false]);
    }

    #[test]
    fn unknown_channel_unset_threshold_or_no_clauses_is_an_empty_population() {
        let (d, ch) = grid();
        assert_eq!(evaluate(&d, 4, &ch, &Rule::new("x", &[("CD8", true)]), &thr()), vec![false; 4]);
        assert_eq!(evaluate(&d, 4, &ch, &Rule::new("x", &[("CD4", true)]), &BTreeMap::new()), vec![false; 4]);
        assert_eq!(evaluate(&d, 4, &ch, &Rule::new("x", &[]), &thr()), vec![false; 4]);
        // strictly above: a value exactly on the threshold is negative
        let on = vec![0.0, 0.0, 0.0, 5.0];
        assert_eq!(evaluate(&on, 1, &ch, &Rule::new("x", &[("CD4", true)]), &thr()), vec![false]);
    }

    #[test]
    fn labels_are_first_match_but_counts_are_per_rule() {
        let (d, ch) = grid();
        let rules = vec![Rule::new("Treg", &[("CD4", true), ("FoxP3", true)]), Rule::new("CD4+", &[("CD4", true)])];
        assert_eq!(labels(&d, 4, &ch, &rules, &thr()), vec![Some(0), Some(1), None, None]);
        let c = counts(&d, 4, &ch, &rules, &thr());
        assert_eq!(c, vec![("Treg".to_string(), 1), ("CD4+".to_string(), 2)]);
    }

    #[test]
    fn cluster_medians_per_marker_and_empty_clusters_are_zero() {
        // cluster 0: rows 0,1 ; cluster 1: row 2 ; cluster 2: empty
        let d = [1.0, 10.0, 3.0, 30.0, 5.0, 50.0];
        let m = cluster_medians(&d, 3, 2, &[0, 0, 1], 3);
        assert_eq!(m, vec![2.0, 20.0, 5.0, 50.0, 0.0, 0.0]);
    }

    #[test]
    fn event_cuts_come_from_the_events() {
        let v: Vec<f64> = (0..100).map(|i| if i < 50 { 1.0 } else { 9.0 }).collect();
        assert_eq!(event_cut(&v, CallMethod::GlobalMedian), Some(stats::median(&v)));
        let o = event_cut(&v, CallMethod::OtsuEvents).unwrap();
        assert!(o > 1.0 && o < 9.0, "Otsu must fall in the valley, got {o}");
        assert_eq!(event_cut(&v, CallMethod::ZScore), None);
    }
}
