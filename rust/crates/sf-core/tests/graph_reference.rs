//! Rust graph clustering against the Python engine's libraries (`rust/tools/gen_graph_reference.py`):
//! the PhenoGraph graph must match the `phenograph` package edge for edge, Louvain must reach modularity
//! comparable to networkx / phenograph, and the clusters must agree with Python's as partitions.

use sf_core::cluster::adjusted_rand_index;
use sf_core::graphcluster::{jaccard_graph, knn, louvain, modularity, phenograph, Graph};
use std::collections::HashMap;

struct Ref {
    karate_q_best: f64,
    karate_edges: Vec<(usize, usize)>,
    n: usize,
    d: usize,
    x: Vec<f64>,
    py_labels: Vec<i64>,
    py_q: f64,
    py_edges: HashMap<(usize, usize), f64>,
}

fn load() -> Ref {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_graph.txt")).expect("run rust/tools/gen_graph_reference.py");
    let mut r = Ref { karate_q_best: 0.0, karate_edges: vec![], n: 0, d: 0, x: vec![], py_labels: vec![], py_q: 0.0, py_edges: HashMap::new() };
    for l in text.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        match t[0] {
            "karate_q_best" => r.karate_q_best = t[1].parse().unwrap(),
            "e" => r.karate_edges.push((t[1].parse().unwrap(), t[2].parse().unwrap())),
            "cells" => {
                r.n = t[1].parse().unwrap();
                r.d = t[2].parse().unwrap();
            }
            "pheno_q" => r.py_q = t[1].parse().unwrap(),
            "g" => {
                // lower triangle: (row > col)
                let (i, j): (usize, usize) = (t[1].parse().unwrap(), t[2].parse().unwrap());
                r.py_edges.insert((i.min(j), i.max(j)), t[3].parse().unwrap());
            }
            "c" => {
                r.x.extend(t[1..=r.d].iter().map(|v| v.parse::<f64>().unwrap()));
                r.py_labels.push(t[r.d + 1].parse().unwrap());
            }
            _ => {}
        }
    }
    r
}

#[test]
fn the_jaccard_graph_matches_phenograph_edge_for_edge() {
    let r = load();
    let k = 30;
    let nb = knn(&r.x, r.n, r.d, k).unwrap();
    let g = jaccard_graph(&nb, r.n, k);
    let mine: HashMap<(usize, usize), f64> = (0..g.n).flat_map(|i| g.adj[i].iter().filter(move |e| (e.0 as usize) > i).map(move |e| ((i, e.0 as usize), e.1))).collect();
    assert_eq!(mine.len(), r.py_edges.len(), "different number of edges");
    let mut worst = 0.0f64;
    for (e, w) in &r.py_edges {
        let m = mine.get(e).unwrap_or_else(|| panic!("edge {e:?} missing from the Rust graph"));
        worst = worst.max((m - w).abs());
    }
    eprintln!("{} edges, worst weight difference {worst:e}", mine.len());
    assert!(worst < 1e-6, "edge weights differ by up to {worst:e} (reference stores float32)");
}

#[test]
fn louvain_reaches_the_modularity_networkx_does_on_the_karate_club() {
    let r = load();
    let g = Graph::from_edges(34, r.karate_edges.iter().map(|&(a, b)| (a, b, 1.0)));
    assert_eq!(g.n_edges(), 78);
    let best = (0..20).map(|s| louvain(&g, s).1).fold(f64::MIN, f64::max);
    eprintln!("karate: rust best Q {best:.4}, networkx best Q {:.4}", r.karate_q_best);
    assert!(best >= r.karate_q_best - 0.01, "rust {best:.4} vs networkx {:.4}", r.karate_q_best);
    assert!(best > 0.41, "known Louvain optimum for the karate club is about 0.4188-0.4198");
    // reported Q equals an independent recomputation
    let (lab, q) = louvain(&g, 3);
    assert!((q - modularity(&g, &lab)).abs() < 1e-12);
}

#[test]
fn phenograph_clusters_agree_with_the_python_package() {
    let r = load();
    let ours = phenograph(&r.x, r.n, r.d, 30, 10, 10, 1).unwrap();
    // unclustered (-1 in Python, None here) are compared as their own label
    let a: Vec<usize> = ours.labels.iter().map(|l| l.map_or(usize::MAX, |v| v)).collect();
    let b: Vec<usize> = r.py_labels.iter().map(|&v| if v < 0 { usize::MAX } else { v as usize }).collect();
    let ari = adjusted_rand_index(&a, &b);
    // modularity of PYTHON's partition on the RUST graph tells whether the two optimisers find equally good structure
    let py_on_rust = {
        let mut map = HashMap::new();
        let comm: Vec<usize> = r.py_labels.iter().map(|&v| { let n = map.len(); *map.entry(v).or_insert(n) }).collect();
        modularity(&ours.graph, &comm)
    };
    eprintln!("rust Q {:.4} ({} clusters) | python Q {:.4} | python partition on rust graph {py_on_rust:.4} | ARI {ari:.3}", ours.modularity, ours.n_clusters, r.py_q);
    assert!(ours.modularity > 0.97 * py_on_rust, "rust optimiser is worse than python's partition on the same graph");
    assert!(ari > 0.85, "partitions disagree: ARI {ari:.3}");
}
