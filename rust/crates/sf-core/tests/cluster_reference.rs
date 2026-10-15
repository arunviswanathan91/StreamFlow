//! The Rust FlowSOM pipeline against the Python engine's backends on one data set with known populations
//! (`rust/tools/gen_cluster_reference.py`). Clusterings are compared as partitions with the adjusted Rand
//! index: the Rust result must recover the truth about as well as Python FlowSOM does, and must agree with
//! Python FlowSOM at least as well as two good clusterings of overlapping data can.

use sf_core::cluster::{adjusted_rand_index, flowsom, scale, Scale};

struct Ref {
    n: usize,
    d: usize,
    x: Vec<f64>,
    truth: Vec<usize>,
    py_flowsom: Vec<usize>,
    py_kmeans: Vec<usize>,
    ari_flowsom_truth: f64,
    ari_kmeans_truth: f64,
}

fn load() -> Ref {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_cluster.txt")).expect("run rust/tools/gen_cluster_reference.py");
    let mut lines = text.lines();
    let dims: Vec<usize> = lines.next().unwrap().split_whitespace().skip(1).map(|v| v.parse().unwrap()).collect();
    let (n, d) = (dims[0], dims[1]);
    let a: f64 = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let b: f64 = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let (mut x, mut t, mut f, mut k) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for l in lines {
        let v: Vec<&str> = l.split_whitespace().collect();
        x.extend(v[..d].iter().map(|s| s.parse::<f64>().unwrap()));
        t.push(v[d].parse().unwrap());
        f.push(v[d + 1].parse().unwrap());
        k.push(v[d + 2].parse().unwrap());
    }
    assert_eq!(t.len(), n);
    Ref { n, d, x, truth: t, py_flowsom: f, py_kmeans: k, ari_flowsom_truth: a, ari_kmeans_truth: b }
}

#[test]
fn rust_flowsom_recovers_the_populations_as_well_as_python_flowsom() {
    let r = load();
    let z = scale(&r.x, r.n, r.d, Scale::ZScore);
    let mut aris = Vec::new();
    for seed in [42u64, 1, 2, 3, 4] {
        let f = flowsom(&z, r.n, r.d, 10, 10, 5, 10, seed).unwrap();
        aris.push((adjusted_rand_index(&f.labels, &r.truth), adjusted_rand_index(&f.labels, &r.py_flowsom)));
    }
    let mean_truth = aris.iter().map(|a| a.0).sum::<f64>() / aris.len() as f64;
    let mean_py = aris.iter().map(|a| a.1).sum::<f64>() / aris.len() as f64;
    eprintln!("python flowsom vs truth {:.3}, python kmeans vs truth {:.3}", r.ari_flowsom_truth, r.ari_kmeans_truth);
    eprintln!("rust flowsom vs truth per seed {:?}", aris.iter().map(|a| (a.0 * 1000.0).round() / 1000.0).collect::<Vec<_>>());
    eprintln!("mean ARI vs truth {mean_truth:.3}, vs python flowsom {mean_py:.3}");
    // not worse than Python FlowSOM on the same data (small allowance for seed noise)
    assert!(mean_truth > r.ari_flowsom_truth - 0.05, "rust {mean_truth:.3} vs python {:.3}", r.ari_flowsom_truth);
    assert!(mean_truth > 0.8, "populations barely separated: {mean_truth:.3}");
    // the two implementations agree with each other nearly as much as each agrees with the truth
    assert!(mean_py > 0.75, "rust and python FlowSOM disagree: ARI {mean_py:.3}");
    let _ = &r.py_kmeans;
}
