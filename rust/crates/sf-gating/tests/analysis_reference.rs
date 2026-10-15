//! Real-data parity: cell cycle, apoptosis and proliferation run by the Rust port on the SAME data must agree
//! with the Python engine (`run_cell_cycle`, `run_apoptosis`, `run_proliferation`; SciPy `curve_fit`,
//! `find_peaks`, `savgol_filter`). Reference numbers were produced by importing `engine/streamflow_engine.py`
//! and calling the real command handlers (script: `rust/tools/gen_analysis_reference.py`).

use sf_core::analysis::*;
use sf_fcs::Fcs;

fn fixture(rel: &str) -> Option<Fcs> {
    let p = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/")).join(rel);
    if !p.exists() {
        eprintln!("SKIPPED: {rel} missing");
        return None;
    }
    Some(Fcs::read(&p).unwrap())
}

#[test]
fn apoptosis_matches_the_engine_on_the_real_annexin_files() {
    // (file, python quadrants live/early/late/necrotic %, python thresholds annexin/pi)
    let cases: [(&str, [f64; 4], f64, f64); 2] = [
        ("dual stain/annexin alone.fcs", [78.32, 0.0, 6.84, 14.84], 53060.8407, 12676.5777),
        ("dual stain/dual-stain-pe-fitc.fcs", [4.12, 3.62, 88.21, 4.05], 19.1808, -8.5293),
    ];
    for (file, quad, ann_t, pi_t) in cases {
        let Some(f) = fixture(file) else { return };
        let (a, p) = (f.column("FL1-A").unwrap(), f.column("FL2-A").unwrap());
        // the engine transforms with arcsinh(150) by default
        let r = apoptosis(&a, &p, 150.0).unwrap();
        assert_eq!(r.n, 10_000, "{file}");
        let q = r.quadrants;
        for (got, want) in [(q.live, quad[0]), (q.early, quad[1]), (q.late, quad[2]), (q.necrotic, quad[3])] {
            assert!((got - want).abs() < 0.011, "{file}: quadrants {q:?} vs python {quad:?}");
        }
        assert!((r.annexin_threshold - ann_t).abs() < 1e-3 * ann_t.abs().max(1.0), "{file}: annexin threshold {} vs python {ann_t}", r.annexin_threshold);
        assert!((r.pi_threshold - pi_t).abs() < 1e-3 * pi_t.abs().max(1.0), "{file}: PI threshold {} vs python {pi_t}", r.pi_threshold);
    }
}

#[test]
fn cell_cycle_matches_the_engine_on_the_real_pi_file() {
    let Some(f) = fixture("single stain/single-stain-pe.fcs") else { return };
    let v = f.column("PI-A").unwrap();
    // (model, G1, S, G2 %, CV G1, CV G2, mu_g1) from the engine. This file is a PE single stain, not a DNA
    // histogram, so the fit is poor (R2 about -0.015 in Python): agreement here means both optimisers reached the
    // same minimum of a hard, flat problem.
    for (model, want, mu1) in [(CycleModel::Watson, [37.27, 39.09, 23.64, 8.94, 8.94], 50909.6), (CycleModel::DeanJettFox, [36.32, 31.23, 32.45, 8.85, 15.03], 51051.3)] {
        let r = cell_cycle(&v, model, &CycleOptions::default()).unwrap();
        assert_eq!(r.n, 26_696);
        let got = [r.g1_pct, r.s_pct, r.g2_pct, r.cv_g1, r.cv_g2];
        for (g, w) in got.iter().zip(want) {
            assert!((g - w).abs() < 1.0, "{model:?}: {got:?} vs python {want:?}");
        }
        assert!((r.mu_g1 - mu1).abs() < 0.01 * mu1, "{model:?}: mu_g1 {} vs {mu1}", r.mu_g1);
        assert!((r.boundaries[0] - 4338.94970703125).abs() < 1e-6 && (r.boundaries[3] - 262143.0).abs() < 1e-6);
    }
}

/// Closed-form dye-dilution data (no RNG, so Python and Rust build identical numbers): generations 0..3 hold
/// 1:2:4:8 of the cells, spaced log10(2), width 0.05 in log10.
fn dye(per_unit: usize) -> Vec<f64> {
    fn vdc(k: usize) -> f64 {
        let (mut x, mut f, mut n) = (0.0, 0.5, k + 1);
        while n > 0 {
            x += f * (n & 1) as f64;
            n >>= 1;
            f *= 0.5;
        }
        x
    }
    let mut vals = Vec::new();
    for (g, w) in [1usize, 2, 4, 8].iter().enumerate() {
        let m = per_unit * w;
        for k in 0..m {
            let u1 = (k as f64 + 0.5) / m as f64;
            let u2 = vdc(k) * 0.999 + 0.0005;
            let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
            vals.push(10f64.powf(4.2 - g as f64 * 2f64.log10() + 0.05 * z));
        }
    }
    vals
}

#[test]
fn proliferation_matches_the_engine_on_the_closed_form_dye_data() {
    let r = proliferation(&dye(2000), 8, None).unwrap();
    assert_eq!(r.n, 29_760);
    // python: PI 2.0, DI 1.5, divided 74.99 %, generations 0..3 at 6.67 / 13.33 / 26.66 / 53.33 %
    assert!((r.pi - 2.0).abs() < 0.01 && (r.di - 1.5).abs() < 0.01 && (r.pct_divided - 74.99).abs() < 0.1, "PI {} DI {} divided {}", r.pi, r.di, r.pct_divided);
    let pct: Vec<f64> = r.generations.iter().map(|g| g.pct).collect();
    assert_eq!(pct.len(), 4);
    for (g, w) in pct.iter().zip([6.67, 13.33, 26.66, 53.33]) {
        assert!((g - w).abs() < 0.1, "{pct:?}");
    }
    // true event counts (python reports the Gaussian area in bin units here): they add up to the events fitted
    let cells: f64 = r.generations.iter().map(|g| g.count).sum();
    assert!((cells - 29_760.0).abs() < 0.03 * 29_760.0, "{cells}");
}
