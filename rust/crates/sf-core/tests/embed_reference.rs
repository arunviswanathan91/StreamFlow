//! Rust t-SNE and embedding metrics against scikit-learn and openTSNE (`rust/tools/gen_embed_reference.py`).
//! Coordinates cannot be compared across implementations, so the comparisons are: (1) the Rust
//! trustworthiness equals scikit-learn's on a fixed embedding; (2) the Rust t-SNE preserves neighbourhoods
//! about as well as openTSNE; (3) under the SAME input affinities, the Rust embedding fits them about as well
//! (KL divergence) as openTSNE's.

use sf_core::embedmetrics::{knn_preservation, trustworthiness};
use sf_core::tsne::{affinities, kl_divergence, tsne, TsneParams};

struct Ref {
    n: usize,
    d: usize,
    z: Vec<f64>,
    pca: Vec<f64>,
    otsne: Vec<f64>,
    trust_pca: f64,
    trust_tsne: f64,
}

fn load() -> Ref {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_embed.txt")).expect("run rust/tools/gen_embed_reference.py");
    let mut lines = text.lines();
    let dims: Vec<usize> = lines.next().unwrap().split_whitespace().skip(1).map(|v| v.parse().unwrap()).collect();
    let (n, d) = (dims[0], dims[1]);
    let trust_pca: f64 = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let trust_tsne: f64 = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
    let (mut z, mut pca, mut otsne) = (Vec::new(), Vec::new(), Vec::new());
    for l in lines {
        let t: Vec<f64> = l.split_whitespace().map(|v| v.parse().unwrap()).collect();
        z.extend(&t[..d]);
        pca.extend(&t[d + 1..d + 3]);
        otsne.extend(&t[d + 3..d + 5]);
    }
    assert_eq!(z.len(), n * d);
    Ref { n, d, z, pca, otsne, trust_pca, trust_tsne }
}

#[test]
fn trustworthiness_equals_scikit_learn() {
    let r = load();
    let mine_pca = trustworthiness(&r.z, r.d, &r.pca, 2, r.n, 12).unwrap();
    let mine_tsne = trustworthiness(&r.z, r.d, &r.otsne, 2, r.n, 12).unwrap();
    eprintln!("pca {mine_pca:.6} vs sklearn {:.6} | openTSNE {mine_tsne:.6} vs sklearn {:.6}", r.trust_pca, r.trust_tsne);
    assert!((mine_pca - r.trust_pca).abs() < 1e-9, "{mine_pca} vs {}", r.trust_pca);
    assert!((mine_tsne - r.trust_tsne).abs() < 1e-9, "{mine_tsne} vs {}", r.trust_tsne);
}

#[test]
fn rust_tsne_preserves_neighbourhoods_and_fits_the_same_affinities_as_well_as_opentsne() {
    let r = load();
    let ours = tsne(&r.z, r.n, r.d, &TsneParams { seed: 42, ..Default::default() }).unwrap();
    let t_ours = trustworthiness(&r.z, r.d, &ours.y, 2, r.n, 12).unwrap();
    let kp_ours = knn_preservation(&r.z, r.d, &ours.y, 2, r.n, 12).unwrap();
    let kp_ref = knn_preservation(&r.z, r.d, &r.otsne, 2, r.n, 12).unwrap();
    let p = affinities(&r.z, r.n, r.d, 30.0).unwrap();
    let (kl_ours, kl_ref, kl_pca) = (kl_divergence(&p, &ours.y), kl_divergence(&p, &r.otsne), kl_divergence(&p, &r.pca));
    eprintln!("trustworthiness: rust {t_ours:.4} | openTSNE {:.4} | pca {:.4}", r.trust_tsne, r.trust_pca);
    eprintln!("12-NN preservation: rust {kp_ours:.4} | openTSNE {kp_ref:.4}");
    eprintln!("KL under the same P: rust {kl_ours:.4} | openTSNE {kl_ref:.4} | pca {kl_pca:.4}");
    assert!(t_ours > r.trust_tsne - 0.02, "rust {t_ours:.4} vs openTSNE {:.4}", r.trust_tsne);
    assert!(kp_ours > kp_ref - 0.05, "kNN preservation rust {kp_ours:.4} vs openTSNE {kp_ref:.4}");
    assert!(kl_ours < kl_ref * 1.15, "rust KL {kl_ours:.4} vs openTSNE {kl_ref:.4}");
    assert!(kl_ours < kl_pca, "t-SNE must beat PCA on its own objective");
}
