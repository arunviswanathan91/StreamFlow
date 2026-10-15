//! Rust UMAP against umap-learn (`rust/tools/gen_umap_reference.py`): the curve parameters, the fuzzy graph
//! (deterministic given the neighbours, so pinned edge for edge) and the layout quality.

use sf_core::embedmetrics::{knn_preservation, trustworthiness};
use sf_core::umap::{find_ab, fuzzy_graph, umap, UmapParams};
use std::collections::HashMap;

struct Ref {
    ab: Vec<(f64, f64, f64)>,
    d: usize,
    zg: Vec<f64>,
    sigmas: Vec<f64>,
    rhos: Vec<f64>,
    edges: HashMap<(u32, u32), f64>,
    ze: Vec<f64>,
    ye: Vec<f64>,
    trust: f64,
}

fn load() -> Ref {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_umap.txt")).expect("run rust/tools/gen_umap_reference.py");
    let mut r = Ref { ab: vec![], d: 0, zg: vec![], sigmas: vec![], rhos: vec![], edges: HashMap::new(), ze: vec![], ye: vec![], trust: 0.0 };
    for l in text.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        match t[0] {
            "ab" => r.ab.push((t[1].parse().unwrap(), t[2].parse().unwrap(), t[3].parse().unwrap())),
            "graph" => r.d = t[2].parse().unwrap(),
            "z" => {
                r.zg.extend(t[1..=r.d].iter().map(|v| v.parse::<f64>().unwrap()));
                r.sigmas.push(t[r.d + 1].parse().unwrap());
                r.rhos.push(t[r.d + 2].parse().unwrap());
            }
            "w" => {
                r.edges.insert((t[1].parse().unwrap(), t[2].parse().unwrap()), t[3].parse().unwrap());
            }
            "embed" => r.trust = t[3].parse().unwrap(),
            "y" => {
                r.ze.extend(t[1..=r.d].iter().map(|v| v.parse::<f64>().unwrap()));
                r.ye.extend(t[r.d + 1..r.d + 3].iter().map(|v| v.parse::<f64>().unwrap()));
            }
            _ => {}
        }
    }
    r
}

#[test]
fn a_and_b_match_umap_learns_curve_fit() {
    for (md, a, b) in load().ab {
        let (ma, mb) = find_ab(md, 1.0).unwrap();
        eprintln!("min_dist {md}: rust ({ma:.5}, {mb:.5}) umap-learn ({a:.5}, {b:.5})");
        assert!((ma - a).abs() < 5e-3 * a.max(1.0) && (mb - b).abs() < 5e-3 * b.max(1.0), "min_dist {md}");
    }
}

#[test]
fn the_fuzzy_graph_matches_umap_learn_edge_for_edge() {
    let r = load();
    let n = r.zg.len() / r.d;
    let g = fuzzy_graph(&r.zg, n, r.d, 15).unwrap();
    let (mut sig, mut rho, mut wmax) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        sig = sig.max((g.sigmas[i] - r.sigmas[i]).abs());
        rho = rho.max((g.rhos[i] - r.rhos[i]).abs());
    }
    assert_eq!(g.edges.len(), r.edges.len(), "different number of edges");
    for &(i, j, w) in &g.edges {
        let want = r.edges.get(&(i, j)).unwrap_or_else(|| panic!("edge ({i},{j}) not in umap-learn's graph"));
        wmax = wmax.max((w - want).abs());
    }
    eprintln!("{} edges: max |d weight| {wmax:e}, max |d sigma| {sig:e}, max |d rho| {rho:e}", g.edges.len());
    assert!(rho < 1e-5 && sig < 1e-4 && wmax < 1e-4, "rho {rho:e} sigma {sig:e} weight {wmax:e}");
}

#[test]
fn rust_umap_preserves_neighbourhoods_as_well_as_umap_learn() {
    let r = load();
    let n = r.ye.len() / 2;
    let ours = umap(&r.ze, n, r.d, &UmapParams::default()).unwrap();
    let t = trustworthiness(&r.ze, r.d, &ours, 2, n, 12).unwrap();
    let t_ref = trustworthiness(&r.ze, r.d, &r.ye, 2, n, 12).unwrap();
    let kp = knn_preservation(&r.ze, r.d, &ours, 2, n, 12).unwrap();
    let kp_ref = knn_preservation(&r.ze, r.d, &r.ye, 2, n, 12).unwrap();
    eprintln!("trustworthiness rust {t:.4} | umap-learn {t_ref:.4} (sklearn said {:.4})", r.trust);
    eprintln!("12-NN preservation rust {kp:.4} | umap-learn {kp_ref:.4}");
    assert!((t_ref - r.trust).abs() < 1e-9, "metric must reproduce scikit-learn on umap-learn's embedding");
    assert!(t > t_ref - 0.02, "rust {t:.4} vs umap-learn {t_ref:.4}");
    assert!(kp > kp_ref - 0.05, "rust {kp:.4} vs umap-learn {kp_ref:.4}");
}

