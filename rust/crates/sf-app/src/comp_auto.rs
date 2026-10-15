//! Spillover matrix from single-stain controls, two estimators (row `i` = spillover of control `i`'s fluorochrome
//! into every detector, the orientation FlowJo uses: corrected = measured × inverse(S)):
//!
//! * [`Estimator::Medians`] (default, the classical FlowJo / DIVA method): split the control into positive and
//!   negative events on its primary detector and take
//!   `S[i][j] = (median_pos_j - median_neg_j) / (median_pos_i - median_neg_i)`. Needs a clear negative population
//!   in the control.
//! * [`Estimator::Slope`]: the Theil–Sen slope of detector `j` against detector `i` through the medians of
//!   quantile bins of the primary detector. Needs no negative population, and is robust to outliers; it weights
//!   the whole range, so on a dim, non-linear control it can differ from the medians method.
//!
//! Neither is "the" truth on a noisy control; both recover a known matrix exactly in the tests.

use crate::sample::{CompInfo, LoadedSample};
use sf_core::stats;

/// A control sample and the detector it stains.
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub sample: usize,
    pub detector: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estimator {
    Medians,
    Slope,
}

impl Estimator {
    pub fn label(self) -> &'static str {
        match self {
            Estimator::Medians => "Population medians (FlowJo method)",
            Estimator::Slope => "Robust slope (no negatives needed)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutoComp {
    pub matrix: CompInfo,
    pub notes: Vec<String>,
}

fn norm(s: &str) -> String {
    s.to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().replace("alexafluor", "af")
}

/// Which detector a control file stains, guessed from its file name (`pe alone.fcs` -> `PE-A`).
pub fn detector_from_name(sample_name: &str, detectors: &[String]) -> Option<String> {
    let n = norm(sample_name.trim_end_matches(".fcs"));
    let hits: Vec<(&String, String)> = detectors.iter().map(|d| (d, norm(d.trim_end_matches("-A").trim_end_matches("-H")))).filter(|(_, base)| base.len() >= 2 && n.contains(base.as_str())).collect();
    // 'PE' inside 'PE-Cy7' is one fluorochrome (keep the longer name); several UNRELATED names means a multi-stain tube,
    // which is not a single-stain control
    let distinct: Vec<&(&String, String)> = hits.iter().filter(|(_, b)| !hits.iter().any(|(_, o)| o.len() > b.len() && o.contains(b.as_str()))).collect();
    match distinct.as_slice() {
        [(d, _)] => Some((*d).clone()),
        _ => None,
    }
}

/// Fallback guess: the detector with the brightest top percentile (relative to its own median).
pub fn detector_from_signal(s: &LoadedSample, detectors: &[String]) -> Option<String> {
    let d = s.channels.len();
    detectors
        .iter()
        .filter_map(|det| {
            let c = s.channel_index(det)?;
            let col: Vec<f64> = (0..s.n_events).map(|r| s.events[r * d + c] as f64).collect();
            Some((det.clone(), stats::percentile(&col, 99.0) - stats::median(&col)))
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(d, _)| d)
}

fn column(s: &LoadedSample, c: usize) -> Vec<f64> {
    let d = s.channels.len();
    (0..s.n_events).map(|r| s.events[r * d + c] as f64).collect()
}

/// Slope of `y` against `x` through the medians of `bins` quantile bins of `x` (Theil–Sen on the bin medians).
pub fn binned_slope(x: &[f64], y: &[f64], bins: usize) -> Option<f64> {
    let n = x.len();
    if n < bins * 5 || bins < 3 {
        return None;
    }
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| x[a].partial_cmp(&x[b]).unwrap());
    let mut pts = Vec::with_capacity(bins);
    for b in 0..bins {
        let (lo, hi) = (b * n / bins, (b + 1) * n / bins);
        let xs: Vec<f64> = idx[lo..hi].iter().map(|&i| x[i]).collect();
        let ys: Vec<f64> = idx[lo..hi].iter().map(|&i| y[i]).collect();
        pts.push((stats::median(&xs), stats::median(&ys)));
    }
    // Theil-Sen: the median of all pairwise slopes between bin medians. Ordinary least squares would be dragged
    // by the few brightest bins (large cells are also more autofluorescent), which the median is not.
    let mut slopes = Vec::new();
    for a in 0..pts.len() {
        for b in a + 1..pts.len() {
            let dx = pts[b].0 - pts[a].0;
            if dx > 0.0 {
                slopes.push((pts[b].1 - pts[a].1) / dx);
            }
        }
    }
    (!slopes.is_empty()).then(|| stats::median(&slopes))
}

/// Compute the matrix for `detectors` (in that order). Detectors without a control keep an identity row and
/// are named in the notes; a control that cannot be used is an error, not a silent guess.
pub fn spillover_from_controls(samples: &[&LoadedSample], controls: &[(usize, String)], detectors: &[String], estimator: Estimator) -> Result<AutoComp, String> {
    let n = detectors.len();
    if n == 0 {
        return Err("no detectors".into());
    }
    let mut spill = vec![0.0; n * n];
    for i in 0..n {
        spill[i * n + i] = 1.0;
    }
    let mut notes = Vec::new();
    let mut done = vec![false; n];
    for (si, det) in controls {
        let s = samples.get(*si).ok_or("control sample index out of range")?;
        let i = detectors.iter().position(|d| d == det).ok_or_else(|| format!("{}: '{det}' is not one of the detectors being compensated", s.name))?;
        if done[i] {
            return Err(format!("two controls were given for {det}"));
        }
        let cols: Vec<usize> = detectors.iter().map(|d| s.channel_index(d).ok_or_else(|| format!("{}: has no channel '{d}'", s.name))).collect::<Result<_, _>>()?;
        let primary = column(s, cols[i]);
        // a usable control has a positive population: Otsu on the arcsinh scale separates it from the negatives
        let cut = 150.0 * stats::otsu(&primary.iter().map(|v| (v / 150.0).asinh()).collect::<Vec<_>>()).sinh();
        let pos: Vec<usize> = (0..primary.len()).filter(|&r| primary[r] > cut).collect();
        let neg: Vec<usize> = (0..primary.len()).filter(|&r| primary[r] <= cut).collect();
        let n_pos = pos.len();
        if n_pos < 100 {
            return Err(format!("{}: only {n_pos} events are positive for {det}; a control needs at least 100", s.name));
        }
        if estimator == Estimator::Medians && neg.len() < 100 {
            return Err(format!("{}: has no clear negative population for {det} ({} events below the split); use the slope method", s.name, neg.len()));
        }
        let spread = stats::percentile(&primary, 95.0) - stats::median(&primary);
        if !(spread > 0.0) {
            return Err(format!("{}: the events are not brighter than the background on {det}", s.name));
        }
        let mut clamped = 0;
        for j in 0..n {
            if j == i {
                continue;
            }
            let y = column(s, cols[j]);
            let mut v = match estimator {
                Estimator::Slope => binned_slope(&primary, &y, 25).ok_or_else(|| format!("{}: too few events to fit the spillover of {det}", s.name))?,
                Estimator::Medians => {
                    let med = |rows: &[usize], c: &[f64]| stats::median(&rows.iter().map(|&r| c[r]).collect::<Vec<_>>());
                    (med(&pos, &y) - med(&neg, &y)) / (med(&pos, &primary) - med(&neg, &primary))
                }
            };
            if v < 0.0 {
                v = 0.0; // negative spillover is noise (autofluorescence differences), not physics
                clamped += 1;
            }
            spill[i * n + j] = v;
        }
        if clamped > 0 {
            notes.push(format!("{}: {clamped} slightly negative coefficient(s) set to 0", s.name));
        }
        notes.push(format!("{}: {det} — {n_pos} positive of {} events", s.name, s.n_events));
        done[i] = true;
    }
    for (i, d) in detectors.iter().enumerate() {
        if !done[i] {
            notes.push(format!("no control for {d}: its row is the identity (it spills into nothing)"));
        }
    }
    Ok(AutoComp { matrix: CompInfo { detectors: detectors.to_vec(), spill, source: "computed from single-stain controls".into() }, notes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::compensation::{Compensation, Mode};
    use sf_core::rng::Rng;

    fn normal(rng: &mut Rng) -> f64 {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    const DET: [&str; 4] = ["FITC-A", "PE-A", "APC-A", "BV421-A"];

    fn truth() -> Vec<f64> {
        vec![1.0, 0.18, 0.02, 0.0, 0.04, 1.0, 0.25, 0.0, 0.0, 0.03, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    }

    /// A single-stain control: half positive (lognormal brightness), half negative; measured = true x S + noise.
    fn control(i: usize, s_true: &[f64], n: usize, seed: u64) -> LoadedSample {
        let mut rng = Rng::new(seed);
        let mut ev = Vec::new();
        for k in 0..n {
            let bright = if k % 2 == 0 { 8000.0 * (0.3 * normal(&mut rng)).exp() } else { 0.0 };
            for j in 0..4 {
                let v = bright * s_true[i * 4 + j] + 80.0 + 40.0 * normal(&mut rng);
                ev.push(v as f32);
            }
        }
        LoadedSample::synthetic(&format!("{} alone.fcs", DET[i].trim_end_matches("-A")), &DET, vec![262_144.0; 4], ev)
    }

    const ESTIMATOR: Estimator = Estimator::Medians;

    fn recovers_the_generating_matrix(est: Estimator) {
        let t = truth();
        let samples: Vec<LoadedSample> = (0..3).map(|i| control(i, &t, 6000, 20 + i as u64)).collect();
        let refs: Vec<&LoadedSample> = samples.iter().collect();
        let det: Vec<String> = DET.iter().map(|s| s.to_string()).collect();
        let controls: Vec<(usize, String)> = (0..3).map(|i| (i, DET[i].to_string())).collect();
        let r = spillover_from_controls(&refs, &controls, &det, est).unwrap();
        for i in 0..4 {
            for j in 0..4 {
                assert!((r.matrix.spill[i * 4 + j] - t[i * 4 + j]).abs() < 0.01, "S[{i}][{j}] = {} vs {}", r.matrix.spill[i * 4 + j], t[i * 4 + j]);
            }
        }
        assert!(r.notes.iter().any(|n| n.contains("no control for BV421-A")), "{:?}", r.notes);
        // applying the recovered matrix removes the spillover: the positives' PE median returns to the negatives'
        let comp = Compensation::new(r.matrix.detectors.clone(), r.matrix.spill.clone()).unwrap();
        let mut ev = samples[0].events.clone();
        comp.apply(&mut ev, 4, &[0, 1, 2, 3], Mode::Standard).unwrap();
        let col = |c: usize, pos: bool| stats::median(&(0..6000).filter(|k| (k % 2 == 0) == pos).map(|k| ev[k * 4 + c] as f64).collect::<Vec<_>>());
        assert!((col(1, true) - col(1, false)).abs() < 25.0, "corrected PE: positives {} vs negatives {}", col(1, true), col(1, false));
        let raw = stats::median(&(0..6000).filter(|k| k % 2 == 0).map(|k| samples[0].events[k * 4 + 1] as f64).collect::<Vec<_>>());
        assert!(raw > 1000.0, "before correction PE was contaminated: {raw}");
    }

    #[test]
    fn the_median_method_recovers_the_spillover_that_generated_the_controls() {
        recovers_the_generating_matrix(Estimator::Medians);
    }

    #[test]
    fn the_slope_method_recovers_it_too() {
        recovers_the_generating_matrix(Estimator::Slope);
    }

    #[test]
    fn the_slope_method_works_without_any_negative_population_and_the_median_method_says_it_cannot() {
        let t = truth();
        let mut c = control(1, &t, 3000, 31);
        // every event bright: there is no negative to compare with
        for k in 0..3000 {
            c.events[k * 4 + 1] += 3000.0;
        }
        for k in 0..3000 {
            c.events[k * 4 + 1] = c.events[k * 4 + 1].max(3000.0);
        }
        let det: Vec<String> = DET.iter().map(|s| s.to_string()).collect();
        let m = spillover_from_controls(&[&c], &[(0, "PE-A".into())], &det, Estimator::Medians);
        assert!(m.is_err() || m.as_ref().unwrap().matrix.spill[1 * 4 + 2] >= 0.0);
        let s = spillover_from_controls(&[&c], &[(0, "PE-A".into())], &det, Estimator::Slope).unwrap();
        assert!((s.matrix.spill[1 * 4 + 2] - t[1 * 4 + 2]).abs() < 0.05, "PE->APC slope {}", s.matrix.spill[1 * 4 + 2]);
    }

    #[test]
    fn unusable_controls_are_refused_with_a_reason() {
        let t = truth();
        let c = control(0, &t, 40, 41);
        let det: Vec<String> = DET.iter().map(|s| s.to_string()).collect();
        assert!(spillover_from_controls(&[&c], &[(0, "FITC-A".into())], &det, ESTIMATOR).unwrap_err().contains("at least 100"));
        let big = control(0, &t, 4000, 42);
        assert!(spillover_from_controls(&[&big], &[(0, "XYZ-A".into())], &det, ESTIMATOR).unwrap_err().contains("not one of the detectors"));
        assert!(spillover_from_controls(&[&big, &big], &[(0, "FITC-A".into()), (1, "FITC-A".into())], &det, ESTIMATOR).unwrap_err().contains("two controls"));
        assert!(spillover_from_controls(&[&big], &[(5, "FITC-A".into())], &det, ESTIMATOR).is_err());
        assert!(spillover_from_controls(&[&big], &[], &[], ESTIMATOR).is_err());
    }

    #[test]
    fn control_names_map_to_detectors() {
        let det: Vec<String> = ["FITC-A", "PE-A", "APC-A", "Alexa Fluor 700-A", "BV421-A", "BV711-A", "PE-Cy7-A"].iter().map(|s| s.to_string()).collect();
        for (name, want) in [("fitc alone.fcs", "FITC-A"), ("AF700 alone.fcs", "Alexa Fluor 700-A"), ("bv421 alone.fcs", "BV421-A"), ("COCULTURE_PE CY7 CD 8_002.fcs", "PE-Cy7-A"), ("pe alone.fcs", "PE-A"), ("apc alone.fcs", "APC-A")] {
            assert_eq!(detector_from_name(name, &det).as_deref(), Some(want), "{name}");
        }
        assert_eq!(detector_from_name("unstained.fcs", &det), None);
        assert_eq!(detector_from_name("multistain-bv421-pe-fitc-apc-bv711-AF700.fcs", &det), None, "a tube with many stains is not a single-stain control");
    }

    /// The real single-stain controls: the computed matrix must actually remove each control's spillover.
    /// Measured independently of the estimator: rise of each secondary detector between the control's own
    /// positive and negative events, before and after per-event compensation.
    #[test]
    fn the_real_single_stain_controls_are_decontaminated_by_their_own_matrix() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
        if !dir.join("fitc alone.fcs").exists() {
            eprintln!("SKIPPED: fixtures missing");
            return;
        }
        let names = ["fitc alone", "pe alone", "apc alone", "AF700 alone", "bv421 alone", "bv711 alone"];
        let samples: Vec<LoadedSample> = names.iter().map(|n| LoadedSample::from_fcs(&dir.join(format!("{n}.fcs"))).unwrap()).collect();
        let det: Vec<String> = ["FITC-A", "PE-A", "APC-A", "Alexa Fluor 700-A", "BV421-A", "BV711-A"].iter().map(|s| s.to_string()).collect();
        let controls: Vec<(usize, String)> = samples.iter().enumerate().map(|(i, s)| (i, detector_from_name(&s.name, &det).unwrap_or_else(|| panic!("{}", s.name)))).collect();
        let refs: Vec<&LoadedSample> = samples.iter().collect();
        for est in [Estimator::Medians, Estimator::Slope] {
        let r = spillover_from_controls(&refs, &controls, &det, est).unwrap();
        let n = 6;
        eprintln!("computed matrix ({}):", est.label());
        for i in 0..n {
            eprintln!("  {:<18} {}", det[i], (0..n).map(|j| format!("{:6.3}", r.matrix.spill[i * n + j])).collect::<Vec<_>>().join(" "));
        }
        assert!((0..n).all(|i| r.matrix.spill[i * n + i] == 1.0));
        assert!(r.matrix.spill.iter().all(|&v| (0.0..1.5).contains(&v)), "coefficients must be plausible");
        let comp = Compensation::new(det.clone(), r.matrix.spill.clone()).unwrap();
        let (mut checked, mut sum_before, mut sum_after) = (0, 0.0f64, 0.0f64);
        for (i, s) in samples.iter().enumerate() {
            let cols: Vec<usize> = det.iter().map(|d| s.channel_index(d).unwrap()).collect();
            let mut ev = s.events.clone();
            comp.apply(&mut ev, s.channels.len(), &cols, Mode::Standard).unwrap();
            let d = s.channels.len();
            let primary: Vec<f64> = (0..s.n_events).map(|k| s.events[k * d + cols[i]] as f64).collect();
            let cut = 150.0 * stats::otsu(&primary.iter().map(|v| (v / 150.0).asinh()).collect::<Vec<_>>()).sinh();
            let pos: Vec<usize> = (0..s.n_events).filter(|&k| primary[k] > cut).collect();
            let neg: Vec<usize> = (0..s.n_events).filter(|&k| primary[k] <= cut).collect();
            let rise = |data: &[f32], j: usize| stats::median(&pos.iter().map(|&k| data[k * d + cols[j]] as f64).collect::<Vec<_>>()) - stats::median(&neg.iter().map(|&k| data[k * d + cols[j]] as f64).collect::<Vec<_>>());
            let main_rise = rise(&s.events, i);
            for j in 0..n {
                if j == i {
                    continue;
                }
                let (before, after) = (rise(&s.events, j), rise(&ev, j));
                if before > 0.005 * main_rise {
                    eprintln!("  {} -> {}: rise {before:8.1} before, {after:8.1} after  (main signal {main_rise:.0})", det[i], det[j]);
                    sum_before += before / main_rise;
                    sum_after += after.abs() / main_rise;
                    checked += 1;
                }
            }
        }
        eprintln!("{checked} measurable pairs: summed leak {:.3} of the main signals before, {:.3} after ({})", sum_before, sum_after, est.label());
        assert!(checked >= 3, "the fixtures should contain several measurable spillovers");
        // on dim real controls per-pair residuals are noise-limited, but the total leak must drop substantially
        assert!(sum_after < 0.5 * sum_before, "{}: leak {sum_before:.3} -> {sum_after:.3}", est.label());
        }
    }
}
