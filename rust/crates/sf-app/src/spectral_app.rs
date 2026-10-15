//! Spectral unmixing as the application uses it: assign single-stain controls (and an unstained tube), build
//! the signature matrix with the engine's cleaning (peak detection, saturation exclusion, signature QC), then
//! unmix any sample into `Unmix-<fluor>` channels (plus `Unmix-AF` when per-cell autofluorescence is on, and
//! `Unmix-Residual`, the part of each event the model could not explain). The numerics are
//! `sf_core::spectral`, verified against the Python engine; this module is the plumbing around them.

use crate::sample::LoadedSample;
use crate::session::Session;
use sf_core::spectral::{self, Method, Qc};
use sf_core::stats;

/// One single-stain control: which sample, and the fluorochrome name used for its channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    pub sample: usize,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Single(String),
    Unstained,
    Other,
}

/// Guess what a file is from its name: `fitc alone.fcs` -> `FITC`, `Unstained.fcs` -> unstained.
pub fn guess_role(sample_name: &str) -> Role {
    let base = sample_name.trim_end_matches(".fcs").trim_end_matches(".FCS").trim();
    let lower = base.to_ascii_lowercase();
    if lower.contains("unstained") || lower.contains("unstain") || lower == "blank" {
        return Role::Unstained;
    }
    for suffix in [" alone", " single", " only", " ss", " control", " stain"] {
        if let Some(stem) = lower.strip_suffix(suffix) {
            let n = stem.len();
            return Role::Single(base[..n].trim().to_string());
        }
    }
    Role::Other
}

/// Detectors to unmix over: fluorescence area channels (no scatter, time, or derived channels).
pub fn spectral_detectors(s: &LoadedSample) -> Vec<String> {
    s.base_channels()
        .iter()
        .filter(|c| {
            let u = c.to_ascii_uppercase();
            u.ends_with("-A") && !u.contains("FSC") && !u.contains("SSC") && !u.contains("TIME")
        })
        .cloned()
        .collect()
}

#[derive(Debug, Clone)]
pub struct SigInfo {
    pub fluor: String,
    pub sample: String,
    pub peak_detector: String,
    pub n_pos: usize,
    pub n_neg: usize,
    pub n_saturated: usize,
}

#[derive(Debug, Clone)]
pub struct SpectralModel {
    pub detectors: Vec<String>,
    pub fluors: Vec<String>,
    /// `f x d`, each row peak-scaled to 1.
    pub m: Vec<f64>,
    pub qc: Qc,
    pub infos: Vec<SigInfo>,
    /// Controls that could not be used, with the reason.
    pub rejected: Vec<String>,
    /// Autofluorescence library `k x d` (from the unstained tube) and the size of each cluster.
    pub af_library: Option<(Vec<f64>, Vec<usize>)>,
}

impl SpectralModel {
    pub fn f(&self) -> usize {
        self.fluors.len()
    }
    pub fn d(&self) -> usize {
        self.detectors.len()
    }
    /// Fluorochromes whose signature is nearly explainable by the others (unmixing amplifies their noise).
    pub fn collinear(&self, limit: f64) -> Vec<(String, f64)> {
        self.fluors.iter().zip(&self.qc.collinearity).filter(|(_, &c)| !(c < limit)).map(|(f, &c)| (f.clone(), c)).collect()
    }
}

fn detector_matrix(s: &LoadedSample, detectors: &[String]) -> Result<Vec<f64>, String> {
    let cols: Vec<usize> = detectors.iter().map(|d| s.channel_index(d).ok_or_else(|| format!("{}: has no detector '{d}'", s.name))).collect::<Result<_, _>>()?;
    let d = s.channels.len();
    Ok((0..s.n_events).flat_map(|r| cols.iter().map(move |&c| (r, c))).map(|(r, c)| s.events[r * d + c] as f64).collect())
}

/// Build the signature matrix. Controls that fail (too few positive events, ...) are listed in `rejected`;
/// the model needs at least two good signatures.
pub fn build_model(session: &Session, refs: &[Reference], unstained: Option<usize>, detectors: &[String], af_clusters: usize, seed: u64) -> Result<SpectralModel, String> {
    let d = detectors.len();
    if d < 2 {
        return Err("choose at least two detectors".into());
    }
    let baseline: Option<Vec<f64>> = match unstained {
        Some(u) => {
            let us = session.sample(u);
            let m = detector_matrix(us, detectors)?;
            Some((0..d).map(|j| stats::median(&(0..us.n_events).map(|i| m[i * d + j]).collect::<Vec<_>>())).collect())
        }
        None => None,
    };
    let (mut rows, mut fluors, mut infos, mut rejected) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for r in refs {
        let s = session.sample(r.sample);
        if fluors.contains(&r.label) {
            return Err(format!("two controls are both named '{}'", r.label));
        }
        let ev = detector_matrix(s, detectors)?;
        let ceiling: Vec<f64> = detectors.iter().map(|det| s.range_of(det)).collect();
        let sig = spectral::build_signature(&ev, s.n_events, d, baseline.as_deref(), 50, Some(&ceiling));
        if !sig.ok {
            rejected.push(format!("{} ({}): {}", r.label, s.name, sig.reason));
            continue;
        }
        let v = sig.signature.clone().unwrap();
        infos.push(SigInfo { fluor: r.label.clone(), sample: s.name.clone(), peak_detector: detectors[sig.peak.unwrap_or(0)].clone(), n_pos: sig.n_pos, n_neg: sig.n_neg, n_saturated: sig.n_saturated });
        rows.extend(v);
        fluors.push(r.label.clone());
    }
    if fluors.len() < 2 {
        return Err(format!("need at least two usable single-stain controls (usable: {}; rejected: {})", fluors.len(), rejected.join("; ")));
    }
    let f = fluors.len();
    let qc = spectral::signature_qc(&rows, f, d);
    let af_library = match (unstained, af_clusters) {
        (Some(u), k) if k > 0 => {
            let us = session.sample(u);
            let m = detector_matrix(us, detectors)?;
            match spectral::extract_af_library(&m, us.n_events, d, k, seed, 200) {
                Ok((lib, counts)) => Some((lib, counts)),
                Err(e) => {
                    rejected.push(format!("autofluorescence library: {e}"));
                    None
                }
            }
        }
        _ => None,
    };
    Ok(SpectralModel { detectors: detectors.to_vec(), fluors, m: rows, qc, infos, rejected, af_library })
}

#[derive(Debug, Clone)]
pub struct UnmixReport {
    pub sample: String,
    pub n_events: usize,
    pub median_residual: f64,
    pub used_af: bool,
}

/// Unmix one sample and attach the results as `Unmix-` channels (replacing earlier ones).
pub fn unmix_sample(session: &mut Session, sample: usize, model: &SpectralModel, method: Method, use_af: bool) -> Result<UnmixReport, String> {
    let (d, f) = (model.d(), model.f());
    let (ev, n, name, range) = {
        let s = session.sample(sample);
        let ev = detector_matrix(s, &model.detectors)?;
        let range = model.detectors.iter().map(|x| s.range_of(x)).fold(0.0, f64::max);
        (ev, s.n_events, s.name.clone(), range)
    };
    let af = if use_af { model.af_library.as_ref() } else { None };
    let (ab, width, resid): (Vec<f64>, usize, Vec<f64>) = match af {
        Some((lib, counts)) => {
            let r = spectral::unmix_with_af(&ev, n, d, &model.m, f, lib, counts.len(), method, None).map_err(|e| e.to_string())?;
            (r.abundances, f + 1, r.residual2.iter().map(|x| x.sqrt()).collect())
        }
        None => {
            let (ab, _) = spectral::unmix(&ev, n, d, &model.m, f, method, None).map_err(|e| e.to_string())?;
            let res = spectral::residuals(&ev, n, d, &model.m, f, &ab);
            (ab, f, res)
        }
    };
    let mut names: Vec<String> = model.fluors.clone();
    if width > f {
        names.push("AF".into());
    }
    let mut cols: Vec<Vec<f32>> = (0..width).map(|a| (0..n).map(|i| ab[i * width + a] as f32).collect()).collect();
    names.push("Residual".into());
    cols.push(resid.iter().map(|&v| v as f32).collect());
    let median_residual = stats::median(&resid);
    session.set_unmixed(sample, names, cols, range)?;
    Ok(UnmixReport { sample: name, n_events: n, median_residual, used_af: width > f })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::rng::Rng;

    const D: usize = 12;

    fn spectra() -> Vec<Vec<f64>> {
        [1.0, 4.0, 7.0, 10.0].iter().map(|&pk| (0..D).map(|j| (-0.5 * ((j as f64 - pk) / 1.4).powi(2)).exp()).collect()).collect()
    }

    fn names() -> Vec<String> {
        (0..D).map(|j| format!("D{j}-A")).collect()
    }

    fn poisson_like(rng: &mut Rng, mean: f64) -> f64 {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        mean + mean.max(1.0).sqrt() * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// `n` events: abundance `ab_of(k)` of each fluor spectrum (+ optional AF spectrum) + baseline + noise.
    fn make_sample(name: &str, n: usize, seed: u64, ab_of: impl Fn(usize) -> [f64; 4], af: Option<&[Vec<f64>]>) -> LoadedSample {
        let mut rng = Rng::new(seed);
        let sp = spectra();
        let mut ev = Vec::new();
        for k in 0..n {
            let ab = ab_of(k);
            let af_spec = af.map(|a| &a[k % a.len()]);
            for j in 0..D {
                let mut mean = 30.0 + (0..4).map(|f| ab[f] * sp[f][j]).sum::<f64>();
                if let Some(a) = af_spec {
                    mean += 400.0 * a[j];
                }
                ev.push(poisson_like(&mut rng, mean).max(0.0) as f32);
            }
        }
        let det: Vec<String> = names();
        let refs: Vec<&str> = det.iter().map(String::as_str).collect();
        LoadedSample::synthetic(name, &refs, vec![1e6; D], ev)
    }

    fn session() -> (Session, Vec<Reference>, usize, usize) {
        let mut s = Session::new();
        let mut refs = Vec::new();
        let af = vec![(0..D).map(|j| 0.3 + 0.7 * (j as f64 / D as f64)).collect::<Vec<_>>(), (0..D).map(|j| 1.0 - 0.8 * (j as f64 / D as f64)).collect::<Vec<_>>()];
        for (f, label) in ["A", "B", "C", "D"].iter().enumerate() {
            let smp = make_sample(&format!("{label} alone.fcs"), 4000, 100 + f as u64, move |k| {
                let mut a = [0.0; 4];
                a[f] = if k % 2 == 0 { 5000.0 * (1.0 + 0.2 * ((k % 7) as f64 - 3.0) / 3.0) } else { 0.0 };
                a
            }, Some(&af));
            let i = s.add_sample(smp);
            refs.push(Reference { sample: i, label: label.to_string() });
        }
        let un = s.add_sample(make_sample("unstained.fcs", 3000, 200, |_| [0.0; 4], Some(&af)));
        let mixed = s.add_sample(make_sample("mixed.fcs", 3000, 300, |k| [if k % 4 == 0 { 3000.0 } else { 0.0 }, if k % 4 == 1 { 2000.0 } else { 0.0 }, if k % 4 == 2 { 4000.0 } else { 0.0 }, if k % 4 == 3 { 1500.0 } else { 0.0 }], Some(&af)));
        (s, refs, un, mixed)
    }

    #[test]
    fn roles_are_guessed_from_file_names() {
        assert_eq!(guess_role("fitc alone.fcs"), Role::Single("fitc".into()));
        assert_eq!(guess_role("AF700 alone.fcs"), Role::Single("AF700".into()));
        assert_eq!(guess_role("BV421 single.fcs"), Role::Single("BV421".into()));
        assert_eq!(guess_role("Unstained.fcs"), Role::Unstained);
        assert_eq!(guess_role("multistain-bv421-pe.fcs"), Role::Other);
    }

    #[test]
    fn the_model_recovers_the_true_spectra_and_reports_quality() {
        let (s, refs, un, _) = session();
        let m = build_model(&s, &refs, Some(un), &names(), 2, 1).unwrap();
        assert_eq!((m.f(), m.d()), (4, D));
        assert!(m.rejected.is_empty(), "{:?}", m.rejected);
        let truth = spectra();
        for (f, sig) in truth.iter().enumerate() {
            let est = &m.m[f * D..(f + 1) * D];
            let norm = sig.iter().cloned().fold(0.0, f64::max);
            let err = est.iter().zip(sig).map(|(a, b)| (a - b / norm).abs()).fold(0.0, f64::max);
            assert!(err < 0.05, "signature {f} differs by {err}");
            assert_eq!(m.infos[f].peak_detector, format!("D{}-A", [1, 4, 7, 10][f]), "peak detector of {f}");
        }
        assert!(m.qc.condition.is_finite() && m.qc.condition > 1.0 && m.qc.condition < 100.0, "{}", m.qc.condition);
        assert!(m.collinear(5.0).is_empty(), "well separated spectra are not collinear: {:?}", m.collinear(5.0));
        let (lib, counts) = m.af_library.as_ref().unwrap();
        assert_eq!(lib.len(), 2 * D);
        assert_eq!(counts.iter().sum::<usize>(), 3000);
    }

    #[test]
    fn unmixing_recovers_abundances_and_per_cell_autofluorescence_lowers_the_residual() {
        let (mut s, refs, un, mixed) = session();
        let m = build_model(&s, &refs, Some(un), &names(), 2, 1).unwrap();
        let plain = unmix_sample(&mut s, mixed, &m, Method::Wls, false).unwrap();
        assert!(!plain.used_af);
        let chans: Vec<String> = s.sample(mixed).channels.clone();
        assert!(chans.contains(&"Unmix-A".to_string()) && chans.contains(&"Unmix-Residual".to_string()) && !chans.contains(&"Unmix-AF".to_string()));
        // events k%4==0 carry 3000 of A (the AF adds a little to everything)
        let col = |s: &Session, name: &str| {
            let smp = s.sample(mixed);
            let c = smp.channel_index(name).unwrap();
            let d = smp.channels.len();
            (0..smp.n_events).map(|r| smp.events[r * d + c] as f64).collect::<Vec<_>>()
        };
        let a = col(&s, "Unmix-A");
        let a_pos = stats::median(&a.iter().enumerate().filter(|(k, _)| k % 4 == 0).map(|(_, v)| *v).collect::<Vec<_>>());
        let a_neg = stats::median(&a.iter().enumerate().filter(|(k, _)| k % 4 != 0).map(|(_, v)| *v).collect::<Vec<_>>());
        assert!((a_pos - a_neg - 3000.0).abs() < 400.0, "A positives are {a_pos}, negatives {a_neg}");
        // with per-cell AF the residual is lower and an AF channel exists
        let with_af = unmix_sample(&mut s, mixed, &m, Method::Wls, true).unwrap();
        assert!(with_af.used_af && s.sample(mixed).channel_index("Unmix-AF").is_some());
        assert!(with_af.median_residual < 0.8 * plain.median_residual, "AF: {} vs {}", with_af.median_residual, plain.median_residual);
        assert_eq!(s.sample(mixed).channels.iter().filter(|c| c.starts_with("Unmix-")).count(), 6, "replaced, not stacked: 4 fluors + AF + residual");
        // compensation and unmixing coexist and clear independently
        let info = crate::sample::CompInfo::identity(vec!["D0-A".into(), "D1-A".into()]);
        s.set_compensation(mixed, &info, sf_core::compensation::Mode::Standard).unwrap();
        assert!(s.sample(mixed).channel_index("Comp-D0-A").is_some() && s.sample(mixed).channel_index("Unmix-A").is_some());
        s.clear_compensation(mixed);
        assert!(s.sample(mixed).channel_index("Comp-D0-A").is_none() && s.sample(mixed).channel_index("Unmix-A").is_some());
        s.clear_unmixed(mixed);
        assert_eq!(s.sample(mixed).channels.len(), D);
    }

    #[test]
    fn bad_controls_are_rejected_with_reasons_and_collinear_sets_are_flagged() {
        let (mut s, mut refs, un, _) = session();
        // a control with no positive events
        let blank = s.add_sample(make_sample("E alone.fcs", 2000, 400, |_| [0.0; 4], None));
        refs.push(Reference { sample: blank, label: "E".into() });
        let m = build_model(&s, &refs, Some(un), &names(), 0, 1).unwrap();
        assert_eq!(m.f(), 4);
        assert!(m.rejected.iter().any(|r| r.starts_with("E ")), "{:?}", m.rejected);
        assert!(m.af_library.is_none());
        // two controls with the same name
        let mut dup = refs.clone();
        dup[1].label = "A".into();
        assert!(build_model(&s, &dup, Some(un), &names(), 0, 1).unwrap_err().contains("both named"));
        // identical spectra: the second fluor is explained by the first -> collinear
        let twin = s.add_sample(make_sample("A2 alone.fcs", 4000, 500, |k| [if k % 2 == 0 { 5000.0 } else { 0.0 }, 0.0, 0.0, 0.0], None));
        let r2 = vec![refs[0].clone(), Reference { sample: twin, label: "A2".into() }, refs[2].clone()];
        let m2 = build_model(&s, &r2, Some(un), &names(), 0, 1).unwrap();
        let flagged: Vec<String> = m2.collinear(5.0).into_iter().map(|c| c.0).collect();
        assert!(flagged.contains(&"A".to_string()) && flagged.contains(&"A2".to_string()), "{flagged:?} cosine {:?}", m2.qc.cosine);
        // bad requests
        assert!(build_model(&s, &refs[..1], Some(un), &names(), 0, 1).unwrap_err().contains("at least two"));
        assert!(build_model(&s, &refs, None, &names()[..1], 0, 1).is_err());
    }

    /// The real single-stain tubes (6 detectors): each control unmixes to mostly its own fluorochrome.
    #[test]
    fn the_real_single_stain_controls_unmix_to_their_own_fluorochromes() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../test-assests/fcs/multi stain"));
        if !dir.join("fitc alone.fcs").exists() {
            eprintln!("SKIPPED: fixtures missing");
            return;
        }
        let mut s = Session::new();
        let mut refs = Vec::new();
        for n in ["fitc alone", "pe alone", "apc alone", "AF700 alone", "bv421 alone", "bv711 alone"] {
            let i = s.load_fcs(&dir.join(format!("{n}.fcs"))).unwrap();
            let Role::Single(label) = guess_role(&format!("{n}.fcs")) else { panic!("{n}") };
            refs.push(Reference { sample: i, label: label.to_uppercase() });
        }
        let un = s.load_fcs(&dir.join("unstained.fcs")).unwrap();
        let multi = s.load_fcs(&dir.join("multistain-bv421-pe-fitc-apc-bv711-AF700.fcs")).unwrap();
        let det = spectral_detectors(s.sample(0));
        assert_eq!(det.len(), 6, "{det:?}");
        let m = build_model(&s, &refs, Some(un), &det, 0, 1).unwrap();
        eprintln!("signatures:");
        for (f, name) in m.fluors.iter().enumerate() {
            eprintln!("  {name:<7} peak {:<18} {}", m.infos[f].peak_detector, (0..m.d()).map(|j| format!("{:5.2}", m.m[f * m.d() + j])).collect::<Vec<_>>().join(" "));
        }
        eprintln!("condition number {:.2}, collinearity {:?}", m.qc.condition, m.qc.collinearity.iter().map(|c| (c * 100.0).round() / 100.0).collect::<Vec<_>>());
        assert!(m.rejected.is_empty(), "{:?}", m.rejected);
        assert!(m.qc.condition.is_finite() && m.qc.condition < 50.0);
        let mut ok = 0;
        for (f, r) in refs.iter().enumerate() {
            unmix_sample(&mut s, r.sample, &m, Method::Ols, false).unwrap();
            let smp = s.sample(r.sample);
            let d = smp.channels.len();
            let own = smp.channel_index(&format!("Unmix-{}", r.label)).unwrap();
            let own_vals: Vec<f64> = (0..smp.n_events).map(|k| smp.events[k * d + own] as f64).collect();
            let hi = stats::percentile(&own_vals, 90.0);
            let others: Vec<f64> = m.fluors.iter().enumerate().filter(|(g, _)| *g != f).map(|(_, name)| {
                let c = smp.channel_index(&format!("Unmix-{name}")).unwrap();
                stats::median(&(0..smp.n_events).filter(|&k| smp.events[k * d + own] as f64 >= hi).map(|k| smp.events[k * d + c] as f64).collect::<Vec<_>>()).abs()
            }).collect();
            let own_med = stats::median(&(0..smp.n_events).filter(|&k| smp.events[k * d + own] as f64 >= hi).map(|k| smp.events[k * d + own] as f64).collect::<Vec<_>>());
            let worst = others.iter().cloned().fold(0.0, f64::max);
            eprintln!("  {:<7} own {own_med:9.0}, largest other {worst:8.0} ({:.1}%)", r.label, 100.0 * worst / own_med);
            assert!(worst < 0.25 * own_med, "{}: crosstalk {worst} vs {own_med}", r.label);
            ok += 1;
        }
        assert_eq!(ok, 6);
        // the fully stained tube unmixes too, with a residual
        let rep = unmix_sample(&mut s, multi, &m, Method::Wls, false).unwrap();
        assert!(rep.median_residual.is_finite());
        assert!(s.sample(multi).channel_index("Unmix-Residual").is_some());
    }
}
