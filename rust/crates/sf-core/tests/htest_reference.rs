//! Parity with SciPy 1.18 as the engine calls it: Mann-Whitney U, Wilcoxon signed-rank, Kruskal-Wallis, Dunn
//! and the distribution functions behind them. `reference_htest.txt` is written by `rust/tools/gen_htest_reference.py`.

use sf_core::htest::*;

fn nums(s: &str) -> Vec<f64> {
    s.split_whitespace().map(|t| t.parse().unwrap_or_else(|_| panic!("bad number {t:?}"))).collect()
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-300)
}

#[test]
fn the_tests_reproduce_scipy_on_every_reference_case() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference_htest.txt")).expect("reference_htest.txt (run rust/tools/gen_htest_reference.py)");
    let (mut n_mwu, mut n_wil, mut n_kw, mut n_dunn, mut n_dist) = (0, 0, 0, 0, 0);
    let mut methods = std::collections::BTreeSet::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let parts: Vec<&str> = line.split(';').collect();
        let head: Vec<&str> = parts[0].split_whitespace().collect();
        match head[0] {
            "mwu" => {
                let (want_p, want_u) = (head[2].parse::<f64>().unwrap(), head[3].parse::<f64>().unwrap());
                let r = mann_whitney_u(&nums(parts[1]), &nums(parts[2])).unwrap();
                assert!(rel(r.p, want_p) < 1e-9, "{}: p {} vs scipy {want_p} ({})", head[1], r.p, r.method);
                assert!((r.statistic - want_u).abs() < 1e-9, "{}: U", head[1]);
                methods.insert(format!("mwu-{}", r.method));
                n_mwu += 1;
            }
            "wil" => {
                let (want_p, want_s) = (head[2].parse::<f64>().unwrap(), head[3].parse::<f64>().unwrap());
                let r = wilcoxon_signed_rank(&nums(parts[1])).unwrap();
                assert!(rel(r.p, want_p) < 1e-9, "{}: p {} vs scipy {want_p} ({})", head[1], r.p, r.method);
                assert!((r.statistic - want_s).abs() < 1e-9, "{}: statistic", head[1]);
                methods.insert(format!("wil-{}", r.method));
                n_wil += 1;
            }
            "kw" => {
                let (want_p, want_h) = (head[2].parse::<f64>().unwrap(), head[3].parse::<f64>().unwrap());
                let g: Vec<Vec<f64>> = parts[1..].iter().map(|s| nums(s)).collect();
                let refs: Vec<&[f64]> = g.iter().map(|v| v.as_slice()).collect();
                let r = kruskal_wallis(&refs).unwrap();
                assert!(rel(r.p, want_p) < 1e-8 && rel(r.statistic, want_h) < 1e-10, "{}: H {} p {} vs scipy {want_h} {want_p}", head[1], r.statistic, r.p);
                n_kw += 1;
            }
            "dunn" => {
                let want: Vec<f64> = head[2..].iter().map(|t| t.parse().unwrap()).collect();
                let groups: Vec<(String, Vec<f64>)> = parts[1..].iter().enumerate().map(|(i, s)| (format!("G{}", i + 1), nums(s))).collect();
                let got = dunn_bonferroni(&groups);
                assert_eq!(got.len(), want.len());
                for (g, w) in got.iter().zip(&want) {
                    // the engine rounds to 5 decimals
                    assert!((g.p - w).abs() < 6e-6, "{}: {} p {} vs engine {w}", head[1], g.pair, g.p);
                }
                n_dunn += 1;
            }
            "norm" => {
                let (z, want) = (head[1].parse::<f64>().unwrap(), head[2].parse::<f64>().unwrap());
                assert!(rel(normal_sf(z), want) < 1e-9, "norm.sf({z}) = {} vs {want}", normal_sf(z));
                n_dist += 1;
            }
            "chi2" => {
                let (x, k, want) = (head[1].parse::<f64>().unwrap(), head[2].parse::<f64>().unwrap(), head[3].parse::<f64>().unwrap());
                assert!(rel(chi2_sf(x, k), want) < 1e-9, "chi2.sf({x}, {k}) = {} vs {want}", chi2_sf(x, k));
                n_dist += 1;
            }
            other => panic!("unknown case kind {other}"),
        }
    }
    eprintln!("checked {n_mwu} mwu, {n_wil} wilcoxon, {n_kw} kruskal, {n_dunn} dunn, {n_dist} distribution points; methods hit: {methods:?}");
    assert!(n_mwu >= 12 && n_wil >= 15 && n_kw == 4 && n_dunn == 4 && n_dist >= 14);
    for m in ["mwu-exact", "mwu-asymptotic", "wil-exact", "wil-permutation", "wil-asymptotic"] {
        assert!(methods.contains(m), "the reference set never exercises {m}: {methods:?}");
    }
}
