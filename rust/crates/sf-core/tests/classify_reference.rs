//! The classifier against scikit-learn (the engine's own pipeline). PCA is compared exactly. A Random Forest is a
//! random object, so the Rust forest is compared with the scikit-learn forest AVERAGED over 20 seeds: mean
//! out-of-bag accuracy and which features the permutation importance picks. Reference file written by
//! `rust/tools/gen_classify_reference.py`.

use sf_core::classify::{classify, pca};

fn nums(s: &str) -> Vec<f64> {
    s.split_whitespace().map(|t| t.parse().unwrap()).collect()
}

fn corr(a: &[f64], b: &[f64]) -> f64 {
    let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    sab / (saa.sqrt() * sbb.sqrt())
}

#[test]
fn pca_matches_scikit_learn_exactly_and_the_forest_agrees_on_average() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_classify.txt")).expect("reference_classify.txt (run rust/tools/gen_classify_reference.py)");
    let (mut n_pca, mut n_rf) = (0, 0);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let parts: Vec<&str> = line.split(';').collect();
        let head: Vec<&str> = parts[0].split_whitespace().collect();
        let tag = head[1];
        match head[0] {
            "pca" => {
                let (n, p, k) = (head[2].parse::<usize>().unwrap(), head[3].parse::<usize>().unwrap(), head[4].parse::<usize>().unwrap());
                let (x, z, l, v) = (nums(parts[1]), nums(parts[2]), nums(parts[3]), nums(parts[4]));
                let (got, _) = pca(&x, n, p).unwrap();
                assert_eq!(got.var_ratio.len(), k, "{tag}");
                for c in 0..k {
                    assert!((got.var_ratio[c] - v[c]).abs() < 1e-9, "{tag}: variance ratio {c}: {} vs {}", got.var_ratio[c], v[c]);
                    for i in 0..n {
                        assert!((got.scores[i][c] - z[i * k + c]).abs() < 1e-8, "{tag}: score[{i}][{c}] {} vs {}", got.scores[i][c], z[i * k + c]);
                    }
                    for j in 0..p {
                        assert!((got.loadings[j][c] - l[j * k + c]).abs() < 1e-8, "{tag}: loading[{j}][{c}] {} vs {}", got.loadings[j][c], l[j * k + c]);
                    }
                }
                n_pca += 1;
            }
            "rf" => {
                let (n, p) = (head[2].parse::<usize>().unwrap(), head[3].parse::<usize>().unwrap());
                let (x, y, oob_ref, imp_ref) = (nums(parts[1]), nums(parts[2]), nums(parts[3])[0], nums(parts[4]));
                let samples: Vec<String> = (0..n).map(|i| format!("s{i}")).collect();
                let feats: Vec<String> = (0..p).map(|j| format!("f{j}")).collect();
                let groups: Vec<Option<String>> = y.iter().map(|v| Some(format!("g{}", *v as usize))).collect();
                let (mut oob_sum, mut imp_sum) = (0.0, vec![0.0; p]);
                for seed in 0..20u64 {
                    let r = classify(&samples, &feats, &x, &groups, seed).unwrap();
                    oob_sum += r.oob_accuracy.unwrap();
                    for (name, v) in &r.importance {
                        imp_sum[name[1..].parse::<usize>().unwrap()] += v / 20.0;
                    }
                }
                let oob = oob_sum / 20.0;
                assert!((oob - oob_ref).abs() < 0.08, "{tag}: mean OOB accuracy {oob:.3} vs scikit-learn {oob_ref:.3}");
                let top = |v: &[f64], k: usize| {
                    let mut idx: Vec<usize> = (0..v.len()).collect();
                    idx.sort_by(|&a, &b| v[b].total_cmp(&v[a]));
                    idx.truncate(k);
                    idx.sort();
                    idx
                };
                match tag {
                    "one-informative" => {
                        assert_eq!(top(&imp_sum, 1), top(&imp_ref, 1), "{tag}: {imp_sum:?} vs {imp_ref:?}");
                        assert!(corr(&imp_sum, &imp_ref) > 0.9, "{tag}: importance correlation {}", corr(&imp_sum, &imp_ref));
                    }
                    "two-informative" => {
                        assert_eq!(top(&imp_sum, 2), top(&imp_ref, 2), "{tag}: {imp_sum:?} vs {imp_ref:?}");
                        assert!(corr(&imp_sum, &imp_ref) > 0.9, "{tag}: importance correlation {}", corr(&imp_sum, &imp_ref));
                    }
                    _ => {}
                }
                eprintln!("{tag}: OOB {oob:.3} (scikit-learn {oob_ref:.3}), top feature {:?}", top(&imp_sum, 1));
                n_rf += 1;
            }
            other => panic!("unknown case {other}"),
        }
    }
    assert_eq!((n_pca, n_rf), (5, 4));
}
