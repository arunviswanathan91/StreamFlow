//! FlowSOM-based discovery and gating end to end: train on a downsample, map the WHOLE file, name the
//! clusters from marker medians, approve them as populations, then gate inside them.

use sf_core::cluster::{adjusted_rand_index, flowsom, Scale, Scaler};
use sf_core::discovery::{self, CallMethod, Rule};
use sf_core::rng::Rng;
use sf_gating::discover::{approve_all, approve_cluster, cluster_rows};
use sf_gating::{EventView, Gate, GateTree};
use std::collections::BTreeMap;

const MARKERS: [&str; 4] = ["CD3", "CD4", "CD8", "FoxP3"];

/// Four phenotypes over four markers (~0 = negative, ~5 = positive).
const PHENOTYPES: [[f64; 4]; 4] = [
    [5.0, 5.0, 0.0, 0.0], // CD4 T  (CD3+ CD4+)
    [5.0, 0.0, 5.0, 0.0], // CD8 T  (CD3+ CD8+)
    [5.0, 5.0, 0.0, 5.0], // Treg   (CD3+ CD4+ FoxP3+)
    [0.0, 0.0, 0.0, 0.0], // non-T
];

fn normal(rng: &mut Rng) -> f64 {
    let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
    (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
}

fn file(n: usize, seed: u64) -> (Vec<f64>, Vec<usize>) {
    let mut rng = Rng::new(seed);
    let (mut x, mut truth) = (Vec::with_capacity(n * 4), Vec::with_capacity(n));
    for i in 0..n {
        let c = i % PHENOTYPES.len();
        for m in 0..4 {
            x.push(PHENOTYPES[c][m] + 0.7 * normal(&mut rng));
        }
        truth.push(c);
    }
    (x, truth)
}

#[test]
fn train_on_a_downsample_map_the_whole_file_name_approve_and_gate_inside() {
    let n = 20_000;
    let (mut x, truth) = file(n, 5);
    // two corrupt events (instrument glitch): they must end up in NO population, not in node 0
    x[7 * 4 + 1] = f64::NAN;
    x[9 * 4] = f64::INFINITY;

    // --- train on 2,000 events
    let mut rng = Rng::new(99);
    let idx: Vec<usize> = rng.choose_sorted(n, 2_000).into_iter().filter(|&i| i != 7 && i != 9).collect();
    let sub: Vec<f64> = idx.iter().flat_map(|&i| x[i * 4..i * 4 + 4].to_vec()).collect();
    let scaler = Scaler::fit(&sub, idx.len(), 4, Scale::ZScore);
    let fs = flowsom(&scaler.transform(&sub, idx.len()), idx.len(), 4, 6, 6, 4, 10, 3).unwrap();

    // --- map ALL events with the training scaling
    let finite: Vec<f64> = x.iter().map(|v| if v.is_finite() { *v } else { 0.0 }).collect();
    let mut scaled = scaler.transform(&finite, n);
    for (q, v) in x.iter().enumerate() {
        if !v.is_finite() {
            scaled[q] = f64::NAN; // keep the corruption visible to predict
        }
    }
    let lab = fs.predict(&scaled, n).unwrap();
    assert_eq!(lab[7], None);
    assert_eq!(lab[9], None);
    assert_eq!(lab.iter().filter(|l| l.is_none()).count(), 2);
    let (ok_labels, ok_truth): (Vec<usize>, Vec<usize>) = lab.iter().zip(&truth).filter_map(|(l, t)| l.map(|l| (l, *t))).unzip();
    let ari = adjusted_rand_index(&ok_labels, &ok_truth);
    assert!(ari > 0.97, "whole-file ARI {ari}");

    // --- discovery: name each cluster from its marker medians
    let rows = cluster_rows(&lab, 4);
    assert_eq!(rows.iter().map(|r| r.len()).sum::<usize>(), n - 2, "every finite event is in exactly one cluster");
    let k = 4;
    let ok_data: Vec<f64> = lab.iter().enumerate().filter(|(_, l)| l.is_some()).flat_map(|(i, _)| x[i * 4..i * 4 + 4].to_vec()).collect();
    let ok_lab: Vec<usize> = lab.iter().flatten().copied().collect();
    let med = discovery::cluster_medians(&ok_data, ok_lab.len(), 4, &ok_lab, k);
    let all_med: Vec<Vec<f64>> = (0..k).map(|c| med[c * 4..c * 4 + 4].to_vec()).collect();
    let channels: Vec<String> = MARKERS.iter().map(|s| s.to_string()).collect();
    let rules = vec![
        Rule::new("Treg", &[("CD3", true), ("CD4", true), ("FoxP3", true)]),
        Rule::new("CD4 T", &[("CD3", true), ("CD4", true), ("FoxP3", false)]),
        Rule::new("CD8 T", &[("CD3", true), ("CD8", true)]),
    ];
    // per-marker cut from the events (Otsu)
    let cuts: Vec<f64> = (0..4)
        .map(|j| {
            let col: Vec<f64> = (0..n).filter(|i| lab[*i].is_some()).map(|i| x[i * 4 + j]).collect();
            discovery::event_cut(&col, CallMethod::OtsuEvents).unwrap()
        })
        .collect();
    let names: Vec<String> = (0..k)
        .map(|c| {
            let calls = discovery::relative_calls(&channels, &all_med[c], &all_med, Some(&cuts), CallMethod::OtsuEvents);
            discovery::name_cluster(&calls, &rules, &|ch| Some(ch.to_string()), &format!("Cluster {}", c + 1))
        })
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    let unnamed: Vec<&String> = sorted.iter().filter(|n| n.starts_with("Cluster ")).collect();
    assert_eq!(unnamed.len(), 1, "only the non-T cluster has no rule: {names:?}");
    for want in ["CD4 T", "CD8 T", "Treg"] {
        assert!(names.iter().any(|n| n == want), "missing {want}: {names:?}");
    }

    // --- approval: populations in the tree, whole-file rows; approving twice does not stack
    let mut tree = GateTree::new("All Events");
    approve_all(&mut tree, GateTree::ROOT, &names, rows.clone()).unwrap();
    let ids = approve_all(&mut tree, GateTree::ROOT, &names, rows.clone()).unwrap(); // second approval replaces the first (new ids)
    assert_eq!(ids.len(), 4);
    assert_eq!(tree.children(GateTree::ROOT).unwrap().len(), 4, "re-approving replaces (BUG-15)");

    // --- evaluate on float32 events (what the gating engine sees), and gate INSIDE a cluster population
    let f32data: Vec<f32> = finite.iter().map(|v| *v as f32).collect();
    let treg = tree.find(&["Treg"]).unwrap();
    let inner = tree.add_gate(treg, "FoxP3 high", Gate::interval("FoxP3", 2.5, 1e9).unwrap()).unwrap();
    let deeper = tree.add_gate(inner, "CD3 high", Gate::interval("CD3", 2.5, 1e9).unwrap()).unwrap();
    let ev = tree.evaluate(&EventView::new(&f32data, 4, &channels).unwrap()).unwrap();
    assert!(ev.warnings.is_empty(), "{:?}", ev.warnings);
    let treg_cluster = names.iter().position(|n| n == "Treg").unwrap();
    assert_eq!(ev.counts[treg], rows[treg_cluster].len());
    assert!(ev.counts[treg] > 4_500, "Treg cluster should hold about a quarter of the file, got {}", ev.counts[treg]);
    assert!(ev.counts[inner] > 4_000 && ev.counts[inner] <= ev.counts[treg]);
    assert!(ev.counts[deeper] > 4_000 && ev.counts[deeper] <= ev.counts[inner], "gating at depth 3 inside a cluster population works");
    let total: usize = ids.iter().map(|&i| ev.counts[i]).sum();
    assert_eq!(total, n - 2);

    // --- label-based discovery on the same events: rules vs thresholds, independent of the clustering
    let thr: BTreeMap<String, f64> = MARKERS.iter().zip(&cuts).map(|(m, c)| (m.to_string(), *c)).collect();
    let rule_counts = discovery::counts(&finite, n, &channels, &rules, &thr);
    let truth_treg = truth.iter().filter(|&&t| t == 2).count();
    let rc = |name: &str| rule_counts.iter().find(|(n, _)| n == name).unwrap().1;
    assert!((rc("Treg") as f64 - truth_treg as f64).abs() < 0.03 * truth_treg as f64, "rule Treg {} vs truth {truth_treg}", rc("Treg"));
    // the rule population and the cluster population are two routes to the same cells
    let rule_mask = discovery::evaluate(&finite, n, &channels, &rules[0], &thr);
    let overlap = rows[treg_cluster].iter().filter(|&&r| rule_mask[r as usize]).count();
    assert!(overlap as f64 > 0.95 * ev.counts[treg] as f64, "cluster and rule definitions of Treg should coincide: {overlap}/{}", ev.counts[treg]);
}

#[test]
fn predict_refuses_a_matrix_of_the_wrong_width_and_cluster_gates_replace_not_stack() {
    let (x, _) = file(400, 1);
    let scaler = Scaler::fit(&x, 400, 4, Scale::ZScore);
    let fs = flowsom(&scaler.transform(&x, 400), 400, 4, 4, 4, 3, 5, 1).unwrap();
    assert!(fs.predict(&[1.0, 2.0, 3.0], 1).is_err());
    let mut t = GateTree::new("r");
    let a = approve_cluster(&mut t, GateTree::ROOT, "C", vec![1, 2, 3]).unwrap();
    let b = approve_cluster(&mut t, GateTree::ROOT, "C", vec![4]).unwrap();
    assert_ne!(a, b);
    assert_eq!(t.children(GateTree::ROOT).unwrap().len(), 1);
    // two clusters with the same phenotype name stay separate populations
    let mut t2 = GateTree::new("r");
    let ids = approve_all(&mut t2, GateTree::ROOT, &["Mixed".to_string(), "Mixed".to_string(), "Empty".to_string()], vec![vec![0], vec![1], vec![]]).unwrap();
    assert_eq!(ids.len(), 2, "empty clusters are not approved");
    assert_eq!(t2.name(ids[1]).unwrap(), "Mixed (2)");
}
