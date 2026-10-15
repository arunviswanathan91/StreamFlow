//! Pins the Rust maths to the Python engine's libraries (FlowKit / flowutils / SciPy).
//! The expected values are in `reference.txt`, produced at full float64 precision by
//! `rust/tools/gen_reference.py`; regenerate that file if the engine's maths ever changes on purpose.

use sf_core::compensation::{Compensation, Mode};
use sf_core::logicle::Logicle;
use sf_core::transforms::{Arcsinh, Log, Transform};
use std::collections::HashMap;

fn blocks() -> HashMap<String, Vec<Vec<f64>>> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference.txt"))
        .expect("reference.txt (run rust/tools/gen_reference.py)");
    let mut out: HashMap<String, Vec<Vec<f64>>> = HashMap::new();
    let mut cur = String::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("# ") {
            cur = h.trim().to_string();
            out.entry(cur.clone()).or_default();
        } else if !line.trim().is_empty() {
            out.get_mut(&cur).unwrap().push(line.split_whitespace().map(|v| v.parse().unwrap()).collect());
        }
    }
    out
}

fn max_err(rows: &[Vec<f64>], f: impl Fn(f64) -> f64) -> (f64, f64) {
    let mut worst = (0.0f64, 0.0f64);
    for r in rows {
        let (x, want) = (r[0], r[1]);
        let got = f(x);
        let e = if want.is_finite() { (got - want).abs() } else if got == want || (got.is_nan() && want.is_nan()) { 0.0 } else { f64::INFINITY };
        if e > worst.0 {
            worst = (e, x);
        }
    }
    worst
}

#[test]
fn logicle_matches_flowutils_for_every_parameter_set() {
    let b = blocks();
    let mut checked = 0;
    for (name, rows) in &b {
        if let Some(p) = name.strip_prefix("logicle ") {
            let v: Vec<f64> = p.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let lg = Logicle::new(v[0], v[1], v[2], v[3]).unwrap();
            let (err, at) = max_err(rows, |x| lg.scale(x));
            assert!(err < 1e-9, "logicle {p}: worst error {err:e} at x={at}");
            checked += 1;
        }
    }
    assert_eq!(checked, 4, "all four logicle parameter sets must be present");
}

#[test]
fn log_and_arcsinh_match() {
    let b = blocks();
    let lg = Log::new(262144.0, 4.5).unwrap();
    let (err, at) = max_err(&b["log 262144 4.5"], |x| lg.forward(x));
    assert!(err < 1e-12, "log error {err:e} at {at}");
    for cof in ["5.0", "150.0", "6000.0"] {
        let t = Arcsinh::new(cof.parse().unwrap());
        let (err, at) = max_err(&b[&format!("arcsinh {cof}")], |x| t.forward(x));
        assert!(err < 1e-13, "arcsinh {cof}: error {err:e} at {at}");
    }
}

fn matrix(rows: &[Vec<f64>]) -> Vec<f64> {
    rows.iter().flatten().copied().collect()
}

#[test]
fn compensation_matches_flowkit_on_real_events() {
    let b = blocks();
    let names = ["FITC-A", "PE-A", "APC-A", "BV421-A"].iter().map(|s| s.to_string()).collect();
    let c = Compensation::new(names, matrix(&b["compensation spillover 4x4"])).unwrap();
    let (raw, want) = (&b["compensation raw 6x4"], &b["compensation expected 6x4"]);
    for (r, w) in raw.iter().zip(want) {
        let got = c.row(r, Mode::Standard).unwrap();
        for (g, e) in got.iter().zip(w) {
            assert!((g - e).abs() <= 1e-9 * e.abs().max(1.0), "got {g}, FlowKit {e}");
        }
    }
}

#[test]
fn non_negative_compensation_matches_scipy_nnls() {
    let b = blocks();
    let names = ["FITC-A", "PE-A", "APC-A", "BV421-A"].iter().map(|s| s.to_string()).collect();
    let c = Compensation::new(names, matrix(&b["compensation spillover 4x4"])).unwrap();
    let (raw, want) = (&b["nnls raw 6x4"], &b["nnls expected 6x4"]);
    for (r, w) in raw.iter().zip(want) {
        let got = c.row(r, Mode::NonNegative).unwrap();
        for (g, e) in got.iter().zip(w) {
            assert!((g - e).abs() <= 1e-8 * e.abs().max(1.0), "got {g}, SciPy {e} for event {r:?}");
        }
    }
}

// ---- spectral unmixing: Rust vs engine/spectral.py ----------------------------------------------------

use sf_core::linalg;
use sf_core::spectral::{self, Method};

const D: usize = 12;

fn flat(b: &HashMap<String, Vec<Vec<f64>>>, name: &str) -> Vec<f64> {
    b.get(name).unwrap_or_else(|| panic!("reference block '{name}' missing")).iter().flatten().copied().collect()
}

/// Relative comparison that also demands `inf == inf`.
fn assert_close(what: &str, got: &[f64], want: &[f64], rel: f64) {
    assert_eq!(got.len(), want.len(), "{what}: length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        if w.is_infinite() || g.is_infinite() {
            assert_eq!(g, w, "{what}[{i}]: {g} vs {w}");
        } else {
            assert!((g - w).abs() <= rel * w.abs().max(1.0), "{what}[{i}]: Rust {g} vs Python {w} (rel {rel:e})");
        }
    }
}

#[test]
fn unmixing_matrices_weights_abundances_and_residuals_match_python() {
    let b = blocks();
    let (m, ev) = (flat(&b, "spectral M 4x12"), flat(&b, "spectral events 8x12"));
    let w = spectral::poisson_weights(&ev, 8, D, 1.0);
    assert_close("poisson weights", &w, &flat(&b, "spectral weights 12"), 1e-12);
    assert_close("W ols", &spectral::unmixing_matrix(&m, 4, D, None).unwrap(), &flat(&b, "spectral W ols 4x12"), 1e-9);
    assert_close("W wls", &spectral::unmixing_matrix(&m, 4, D, Some(&w)).unwrap(), &flat(&b, "spectral W wls 4x12"), 1e-9);
    let (ab_w, _) = spectral::unmix(&ev, 8, D, &m, 4, Method::Wls, None).unwrap();
    let (ab_o, _) = spectral::unmix(&ev, 8, D, &m, 4, Method::Ols, None).unwrap();
    assert_close("unmix wls", &ab_w, &flat(&b, "spectral unmix wls 8x4"), 1e-9);
    assert_close("unmix ols", &ab_o, &flat(&b, "spectral unmix ols 8x4"), 1e-9);
    assert_close("residuals", &spectral::residuals(&ev, 8, D, &m, 4, &ab_w), &flat(&b, "spectral residuals 8"), 1e-9);
}

#[test]
fn signature_qc_matches_python_including_the_dependent_and_near_collinear_cases() {
    let b = blocks();
    let m = flat(&b, "spectral M 4x12");
    // exactly dependent: rows 1, 2, 4 are mutually dependent -> inf in BOTH implementations
    let mut dep = m.clone();
    dep.extend((0..D).map(|j| 0.97 * m[D + j] + 0.03 * m[2 * D + j]));
    let q = spectral::signature_qc(&dep, 5, D);
    assert_close("dependent cosine", &q.cosine, &flat(&b, "spectral qc cosine 5x5"), 1e-9);
    assert_close("dependent collinearity", &q.collinearity, &flat(&b, "spectral qc collinearity 5"), 1e-6);
    let py_cond = flat(&b, "spectral qc condition 1")[0];
    assert!(q.condition > 1e12 && py_cond > 1e12, "dependent set must read as numerically singular: Rust {:e}, Python {py_cond:e}", q.condition);
    // near-collinear (finite)
    let near = flat(&b, "spectral qc near matrix 5x12");
    let qn = spectral::signature_qc(&near, 5, D);
    assert_close("near collinearity", &qn.collinearity, &flat(&b, "spectral qc near collinearity 5"), 1e-6);
    assert_close("near cosine", &qn.cosine, &flat(&b, "spectral qc near cosine 5x5"), 1e-9);
    // condition number of the clean set, from singular values (accurate; not from the Gram matrix)
    assert_close("cond(M)", &[spectral::signature_qc(&m, 4, D).condition], &flat(&b, "spectral cond M 1"), 1e-9);
}

#[test]
fn symmetric_eigenvalues_match_numpy() {
    let b = blocks();
    let a = flat(&b, "eig sym 6x6");
    let (vals, _) = linalg::sym_eig(&a, 6).unwrap();
    assert_close("eigenvalues", &vals, &flat(&b, "eig values desc 6"), 1e-10);
}

#[test]
fn signatures_peak_detector_and_saturation_match_python() {
    let b = blocks();
    let ctl = flat(&b, "spectral control 300x12");
    let neg = flat(&b, "spectral negative 12");
    let ceiling: Vec<f64> = (0..D).map(|j| (0..300).map(|i| ctl[i * D + j]).fold(f64::MIN, f64::max)).collect();
    assert_eq!(spectral::peak_detector(&ctl, 300, D) as f64, flat(&b, "spectral peak detector 1")[0]);
    let mask: Vec<f64> = spectral::saturation_mask(&ctl, 300, D, Some(&ceiling), 0.999).iter().map(|k| *k as u8 as f64).collect();
    assert_eq!(mask, flat(&b, "spectral saturation mask 300"));
    for (tag, negative, sat) in [("plain", None, None), ("negative", Some(&neg[..]), None), ("saturation", None, Some(&ceiling[..]))] {
        let r = spectral::build_signature(&ctl, 300, D, negative, 50, sat);
        assert!(r.ok, "{tag}: {}", r.reason);
        let head = flat(&b, &format!("spectral signature {tag} head")); // peak n_pos n_neg threshold n_saturated
        assert_eq!(r.peak.unwrap() as f64, head[0], "{tag}: peak");
        assert_eq!((r.n_pos as f64, r.n_neg as f64, r.n_saturated as f64), (head[1], head[2], head[4]), "{tag}: counts");
        assert_close(&format!("{tag} threshold"), &[r.threshold.unwrap()], &[head[3]], 1e-9);
        assert_close(&format!("{tag} signature"), &r.signature.unwrap(), &flat(&b, &format!("spectral signature {tag}")), 1e-9);
    }
}

#[test]
fn per_cell_autofluorescence_matches_python() {
    let b = blocks();
    let (m, lib, ev) = (flat(&b, "spectral M 4x12"), flat(&b, "spectral af library 2x12"), flat(&b, "spectral af events 8x12"));
    let r = spectral::unmix_with_af(&ev, 8, D, &m, 4, &lib, 2, Method::Wls, None).unwrap();
    assert_close("af abundances", &r.abundances, &flat(&b, "spectral af unmix wls 8x5"), 1e-8);
    assert_eq!(r.af_index.iter().map(|i| *i as f64).collect::<Vec<_>>(), flat(&b, "spectral af index 8"));
    assert_close("af residual^2", &r.residual2, &flat(&b, "spectral af residual2 8"), 1e-8);
}

#[test]
fn flowjo_biex_matches_flowkits_table_for_every_parameter_set_both_directions() {
    use sf_core::flowjo::Biex;
    let b = blocks();
    let mut checked = 0;
    for (name, rows) in &b {
        if let Some(p) = name.strip_prefix("biex ") {
            let v: Vec<f64> = p.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let bx = Biex::new(v[0], v[1], v[2], v[3]).unwrap();
            let (err, at) = max_err(rows, |x| bx.forward(x));
            assert!(err < 1e-9, "biex {p}: forward worst error {err:e} at x={at}");
            let inv = &b[&format!("biexinv {p}")];
            let (err, at) = max_err(inv, |y| bx.inverse(y));
            assert!(err < 1e-6, "biex {p}: inverse worst error {err:e} at y={at}");
            checked += 1;
        }
    }
    assert_eq!(checked, 5);
}

#[test]
fn flowjo_log_matches_flowkit() {
    use sf_core::flowjo::WspLog;
    let b = blocks();
    let mut checked = 0;
    for (name, rows) in &b {
        if let Some(p) = name.strip_prefix("wsplog ") {
            let v: Vec<f64> = p.split_whitespace().map(|x| x.parse().unwrap()).collect();
            let lg = WspLog::new(v[0], v[1]).unwrap();
            let (err, at) = max_err(rows, |x| lg.forward(x));
            assert!(err < 1e-12, "wsplog {p}: worst error {err:e} at x={at}");
            checked += 1;
        }
    }
    assert_eq!(checked, 2);
}

#[test]
fn flowjo_biex_has_the_shape_the_documentation_describes() {
    use sf_core::flowjo::Biex;
    let bx = Biex::flowjo_default();
    // strictly increasing, zero maps near the 1/10-ish "zero channel", symmetric table about it
    let mut prev = f64::NEG_INFINITY;
    for k in -3000..=262144 / 50 {
        let y = bx.forward(k as f64 * 50.0);
        assert!(y >= prev, "not monotone at {k}");
        prev = y;
    }
    assert_eq!(bx.forward(1e9), 4096.0);
    assert_eq!(bx.forward(-1e9), 0.0);
    assert!(bx.forward(f64::NAN).is_nan());
    // zero sits exactly on a channel, and the table is mirrored about it: x(z+k) = -x(z-k)
    let z = bx.forward(0.0);
    assert_eq!(z, z.trunc(), "zero must land on a whole channel");
    for k in [1.0, 10.0, 100.0, 400.0] {
        let (up, dn) = (bx.inverse(z + k), bx.inverse(z - k));
        assert!((up + dn).abs() < 1e-9 * up.abs(), "k={k}: {up} vs {dn}");
    }
    // with no extra negative decades the negative side is short: far-negative data piles at channel 0
    assert_eq!(bx.forward(-1000.0), 0.0);
    assert!((bx.inverse(0.0) + 132.287).abs() < 1e-3, "default scale bottoms out near -132 raw units");
    // exact inverse inside the table
    for x in [-100.0, 0.0, 17.0, 800.0, 12345.0, 200000.0] {
        assert!((bx.inverse(bx.forward(x)) - x).abs() < 1e-6 * x.abs().max(1.0), "x={x}");
    }
    assert!(Biex::new(0.0, 10.0, 4.4, 262144.0).is_err(), "positive width basis is meaningless");
    assert!(Biex::new(0.0, -10.0, 0.0, 262144.0).is_err());
}
