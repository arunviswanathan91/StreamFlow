//! Cell cycle, proliferation (dye dilution) and apoptosis (Annexin V / PI) analyses.
//!
//! Ports of the engine's `run_cell_cycle`, `run_proliferation` and `run_apoptosis`: the same histogram
//! binning, seed detection, parameter bounds, trims and thresholds, so results agree with the Python engine
//! on the same data (checked in `tests/analysis_reference.rs`).
//!
//! * Cell cycle: Watson pragmatic (G2 sigma = 2 x G1 sigma) or Dean-Jett-Fox (free G2 sigma); G0/G1 and G2/M
//!   are Gaussians at `mu` and `2 mu`, S phase is a rectangle broadened by the G1 sigma (error functions).
//! * Proliferation: equal-sigma Gaussians one halving (`log10 2`) apart; generation sizes are corrected for
//!   the precursors that produced them (`P_g = N_g / 2^g`) before the Division and Proliferation Indices.
//! * Apoptosis: thresholds at the density valley of each axis' histogram (Savitzky-Golay smoothed), then
//!   quadrant counts.

use crate::fitting::{curve_fit_bounded, erf};
use crate::signal::{convolve_same, find_peaks, histogram, savgol_filter, PeakOptions};
use crate::stats;

fn gauss(x: f64, amp: f64, mu: f64, sig: f64) -> f64 {
    amp * (-0.5 * ((x - mu) / sig).powi(2)).exp()
}

fn sbox(x: f64, amp: f64, m1: f64, m2: f64, sig: f64) -> f64 {
    0.5 * amp * (erf((m2 - x) / (sig * std::f64::consts::SQRT_2)) - erf((m1 - x) / (sig * std::f64::consts::SQRT_2)))
}

/// Python's `round` (half to even) for non-negative values.
fn round_half_even(v: f64) -> f64 {
    let r = v.round();
    if (v - v.trunc() - 0.5).abs() < 1e-12 && r % 2.0 != 0.0 {
        r - 1.0
    } else {
        r
    }
}

/// Make every lower bound strictly less than its upper bound and clamp the seed into range.
fn sanitize(lb: &mut [f64], ub: &mut [f64], p0: &mut [f64]) {
    for k in 0..lb.len() {
        if !(ub[k] > lb[k]) {
            ub[k] = lb[k] + (lb[k].abs() * 1e-3).max(1e-6);
        }
        p0[k] = p0[k].clamp(lb[k], ub[k]);
    }
}

// ---- cell cycle -------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleModel {
    Watson,
    DeanJettFox,
}

#[derive(Debug, Clone, Default)]
pub struct CycleOptions {
    pub bins: Option<usize>,
    /// Seed the G0/G1 peak mean from a user-placed anchor.
    pub mu_g1_hint: Option<f64>,
    /// Seed it from the G2/M anchor (`mu = hint / 2`).
    pub mu_g2_hint: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct CycleResult {
    pub g1_pct: f64,
    pub s_pct: f64,
    pub g2_pct: f64,
    pub cv_g1: f64,
    pub cv_g2: f64,
    pub mu_g1: f64,
    pub mu_g2: f64,
    pub r2: f64,
    pub rmse: f64,
    pub n: usize,
    /// `[lo, g1_hi, g2_lo, hi]` phase boundaries in data units.
    pub boundaries: [f64; 4],
    pub x: Vec<f64>,
    pub hist: Vec<f64>,
    pub g1: Vec<f64>,
    pub s: Vec<f64>,
    pub g2: Vec<f64>,
    pub total: Vec<f64>,
}

pub fn cell_cycle(values: &[f64], model: CycleModel, opt: &CycleOptions) -> Result<CycleResult, String> {
    let mut vals: Vec<f64> = values.iter().copied().filter(|v| v.is_finite() && *v > 0.0).collect(); // DNA stains are linear and positive
    if vals.len() < 100 {
        return Err("Too few events with positive DNA signal for cell-cycle fitting.".into());
    }
    let hi = stats::percentile(&vals, 99.5); // trim the doublet / debris tail
    vals.retain(|&v| v <= hi);
    if !(stats::max(&vals) > stats::min(&vals)) {
        return Err("The DNA signal is constant, so there is no histogram to fit.".into());
    }
    let nbins = opt.bins.unwrap_or(256);
    let (y, edges) = histogram(&vals, nbins, None);
    let x: Vec<f64> = (0..nbins).map(|i| 0.5 * (edges[i] + edges[i + 1])).collect();
    let dx = x[1] - x[0];
    let ys = convolve_same(&y, &[0.2; 5]);
    let ymax = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let pk = find_peaks(&ys, &PeakOptions { prominence: Some(ymax * 0.05), distance: Some((nbins / 40).max(1)), height: None });
    if pk.indices.is_empty() {
        return Err("No clear peak found in the DNA histogram.".into());
    }
    let mut best = 0;
    for (i, &p) in pk.prominences.iter().enumerate() {
        if p > pk.prominences[best] {
            best = i; // first maximum, like numpy.argmax
        }
    }
    let g1_idx = pk.indices[best];
    let mut mu1_seed = x[g1_idx];
    if let Some(h) = opt.mu_g1_hint {
        mu1_seed = h;
    } else if let Some(h) = opt.mu_g2_hint {
        mu1_seed = h / 2.0;
    }
    let mut g2_idx = None;
    if pk.indices.len() > 1 {
        let mut cand = pk.indices[0];
        for &p in &pk.indices {
            if (x[p] - 2.0 * mu1_seed).abs() < (x[cand] - 2.0 * mu1_seed).abs() {
                cand = p;
            }
        }
        if x[cand] >= mu1_seed * 1.4 {
            g2_idx = Some(cand);
        }
    }
    let a1_seed = y[g1_idx];
    let a2_seed = g2_idx.map_or(a1_seed * 0.15, |i| y[i]);
    let s1_seed = (mu1_seed * 0.05).max(dx);
    let as_seed = (a1_seed * 0.05).max(1.0);
    let hint = opt.mu_g1_hint.is_some() || opt.mu_g2_hint.is_some();
    let (lo_frac, hi_frac) = if hint { (0.90, 1.10) } else { (0.6, 1.4) };

    let watson = model == CycleModel::Watson;
    let (mut p0, mut lb, mut ub): (Vec<f64>, Vec<f64>, Vec<f64>) = if watson {
        (vec![a1_seed, mu1_seed, s1_seed, a2_seed, as_seed], vec![0.0, mu1_seed * lo_frac, dx, 0.0, 0.0], vec![a1_seed * 3.0, mu1_seed * hi_frac, mu1_seed * 0.2, a1_seed * 3.0, a1_seed])
    } else {
        (vec![a1_seed, mu1_seed, s1_seed, a2_seed, s1_seed * 1.2, as_seed], vec![0.0, mu1_seed * lo_frac, dx, 0.0, dx, 0.0], vec![a1_seed * 3.0, mu1_seed * hi_frac, mu1_seed * 0.2, a1_seed * 3.0, mu1_seed * 0.3, a1_seed])
    };
    sanitize(&mut lb, &mut ub, &mut p0);
    let f = move |p: &[f64], xs: &[f64], out: &mut [f64]| {
        for (o, &xx) in out.iter_mut().zip(xs) {
            *o = if watson {
                gauss(xx, p[0], p[1], p[2]) + gauss(xx, p[3], 2.0 * p[1], 2.0 * p[2]) + sbox(xx, p[4], p[1], 2.0 * p[1], p[2])
            } else {
                gauss(xx, p[0], p[1], p[2]) + gauss(xx, p[3], 2.0 * p[1], p[4]) + sbox(xx, p[5], p[1], 2.0 * p[1], p[2])
            };
        }
    };
    let fit = curve_fit_bounded(&f, &x, &y, &p0, &lb, &ub, 400).map_err(|e| format!("Cell-cycle fit did not converge: {e}"))?;
    let p = &fit.params;
    let (a1, mu1, sig1, a2) = (p[0], p[1], p[2], p[3]);
    let (sig2, a_s) = if watson { (2.0 * sig1, p[4]) } else { (p[4], p[5]) };
    let mu2 = 2.0 * mu1;
    let g1: Vec<f64> = x.iter().map(|&v| gauss(v, a1, mu1, sig1)).collect();
    let g2: Vec<f64> = x.iter().map(|&v| gauss(v, a2, mu2, sig2)).collect();
    let s: Vec<f64> = x.iter().map(|&v| sbox(v, a_s, mu1, mu2, sig1)).collect();
    let total: Vec<f64> = (0..nbins).map(|i| g1[i] + g2[i] + s[i]).collect();
    let (ag1, ag2, as_) = (g1.iter().sum::<f64>() * dx, g2.iter().sum::<f64>() * dx, s.iter().sum::<f64>() * dx);
    let tot = ag1 + ag2 + as_;
    if !(tot > 0.0) {
        return Err("Cell-cycle fit produced no area — check the channel selection.".into());
    }
    let ymean = stats::mean(&y);
    let ss_res: f64 = y.iter().zip(&total).map(|(a, b)| (a - b) * (a - b)).sum();
    let ss_tot: f64 = y.iter().map(|a| (a - ymean) * (a - ymean)).sum();
    let r2 = if ss_tot > 0.0 { 1.0 - ss_res / ss_tot } else { 0.0 };
    let rmse = (ss_res / nbins as f64).sqrt();
    let (vmin, vmax) = (stats::min(&vals), stats::max(&vals));
    let mut g1_hi = mu1 + 1.5 * sig1;
    let mut g2_lo = mu2 - 1.5 * sig2;
    if !(g1_hi < g2_lo) {
        let mid = 0.5 * (mu1 + mu2);
        g1_hi = mid - 1e-6;
        g2_lo = mid + 1e-6;
    }
    let g1_hi = g1_hi.clamp(vmin, vmax);
    let g2_lo = g2_lo.clamp(vmin, vmax);
    Ok(CycleResult {
        g1_pct: 100.0 * ag1 / tot,
        s_pct: 100.0 * as_ / tot,
        g2_pct: 100.0 * ag2 / tot,
        cv_g1: 100.0 * sig1 / mu1,
        cv_g2: 100.0 * sig2 / mu2,
        mu_g1: mu1,
        mu_g2: mu2,
        r2,
        rmse,
        n: vals.len(),
        boundaries: [vmin, g1_hi, g2_lo, vmax],
        x,
        hist: y,
        g1,
        s,
        g2,
        total,
    })
}

// ---- proliferation ------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Generation {
    pub gen: usize,
    pub count: f64,
    pub pct: f64,
}

#[derive(Debug, Clone)]
pub struct ProliferationResult {
    pub pi: f64,
    pub di: f64,
    pub pct_divided: f64,
    pub generations: Vec<Generation>,
    pub n: usize,
    pub x: Vec<f64>,
    pub hist: Vec<f64>,
    /// Fitted curve of each reported generation.
    pub curves: Vec<Vec<f64>>,
    pub total: Vec<f64>,
}

pub fn proliferation(values: &[f64], max_generations: usize, bins: Option<usize>) -> Result<ProliferationResult, String> {
    let mut vals: Vec<f64> = values.iter().copied().filter(|v| v.is_finite() && *v > 0.0).collect(); // log space needs positives
    if vals.len() < 100 {
        return Err("Too few positive events for proliferation fitting.".into());
    }
    let (lo, hi) = (stats::percentile(&vals, 0.5), stats::percentile(&vals, 99.7));
    vals.retain(|&v| v >= lo && v <= hi);
    let xv: Vec<f64> = vals.iter().map(|v| v.log10()).collect(); // generations are evenly spaced in log10
    let nbins = bins.unwrap_or(256);
    let (y, edges) = histogram(&xv, nbins, None);
    let x: Vec<f64> = (0..nbins).map(|i| 0.5 * (edges[i] + edges[i + 1])).collect();
    let dx = x[1] - x[0];
    let log2 = 2f64.log10(); // one division = -log10(2) in log10 space
    let ys = convolve_same(&y, &[0.2; 5]);
    let ymax = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let pk = find_peaks(&ys, &PeakOptions { prominence: Some(ymax * 0.03), distance: Some(((log2 / dx * 0.5) as usize).max(1)), height: None });
    if pk.indices.is_empty() {
        return Err("No peaks found in the histogram.".into());
    }
    let mu0_seed = pk.indices.iter().map(|&i| x[i]).fold(f64::NEG_INFINITY, f64::max); // gen 0 = brightest peak
    let sig_seed = (log2 * 0.18).max(dx);
    // cap generations to those whose centre stays inside the data (a dimmer one would overfit the edge tail)
    let max_feasible = ((mu0_seed - x[0]) / log2) as i64 + 1;
    let n_peaks = (max_generations as i64).min(max_feasible).max(2) as usize;
    let ymax_raw = y.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let amp_seed: Vec<f64> = (0..n_peaks)
        .map(|g| {
            let c = mu0_seed - g as f64 * log2;
            let j = round_half_even((c - x[0]) / dx).clamp(0.0, (nbins - 1) as f64) as usize;
            y[j].max(1.0)
        })
        .collect();
    let mut p0 = vec![mu0_seed, log2, sig_seed];
    p0.extend(&amp_seed);
    let mut lb = vec![mu0_seed - log2, log2 * 0.7, dx];
    lb.extend(vec![0.0; n_peaks]);
    let mut ub = vec![mu0_seed + log2, log2 * 1.3, log2 * 0.6];
    ub.extend(vec![ymax_raw * 1.5; n_peaks]);
    sanitize(&mut lb, &mut ub, &mut p0);
    let model = |p: &[f64], xs: &[f64], out: &mut [f64]| {
        for (o, &xx) in out.iter_mut().zip(xs) {
            *o = (0..p.len() - 3).map(|g| gauss(xx, p[3 + g], p[0] - g as f64 * p[1], p[2])).sum();
        }
    };
    let fit = curve_fit_bounded(&model, &x, &y, &p0, &lb, &ub, 800).map_err(|e| format!("Proliferation fit did not converge: {e}"))?;
    let (mu0, delta, sig) = (fit.params[0], fit.params[1], fit.params[2]);
    let amps = &fit.params[3..];
    let areas: Vec<f64> = amps.iter().map(|a| a * sig * (2.0 * std::f64::consts::PI).sqrt()).collect();
    let thr = areas.iter().cloned().fold(f64::NEG_INFINITY, f64::max) * 0.02;
    let mut gens = Vec::new();
    for g in 0..n_peaks {
        if areas[g] >= thr {
            gens.push(g); // generations are contiguous from 0: a gap ends the series
        } else {
            break;
        }
    }
    if gens.is_empty() {
        return Err("No generation peaks above threshold.".into());
    }
    let p_of = |g: usize| areas[g] / 2f64.powi(g as i32); // precursor cohort
    let sum_p: f64 = gens.iter().map(|&g| p_of(g)).sum();
    let sum_p_div: f64 = gens.iter().filter(|&&g| g >= 1).map(|&g| p_of(g)).sum();
    let di = if sum_p > 0.0 { gens.iter().map(|&g| g as f64 * p_of(g)).sum::<f64>() / sum_p } else { 0.0 };
    let pi = if sum_p_div > 0.0 { gens.iter().filter(|&&g| g >= 1).map(|&g| g as f64 * p_of(g)).sum::<f64>() / sum_p_div } else { 0.0 };
    let pct_divided = if sum_p > 0.0 { 100.0 * sum_p_div / sum_p } else { 0.0 };
    let total_cells: f64 = gens.iter().map(|&g| areas[g]).sum();
    let generations = gens.iter().map(|&g| Generation { gen: g, count: (areas[g] / dx).round(), pct: 100.0 * areas[g] / total_cells }).collect();
    let mut total = vec![0.0; nbins];
    let curves: Vec<Vec<f64>> = gens
        .iter()
        .map(|&g| {
            let c: Vec<f64> = x.iter().map(|&xx| gauss(xx, amps[g], mu0 - g as f64 * delta, sig)).collect();
            for (t, v) in total.iter_mut().zip(&c) {
                *t += v;
            }
            c
        })
        .collect();
    Ok(ProliferationResult { pi, di, pct_divided, generations, n: vals.len(), x, hist: y, curves, total })
}

// ---- apoptosis ------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quadrants {
    pub live: f64,
    pub early: f64,
    pub late: f64,
    pub necrotic: f64,
}

#[derive(Debug, Clone)]
pub struct ApoptosisResult {
    pub quadrants: Quadrants,
    pub counts: [usize; 4],
    pub annexin_threshold: f64,
    pub pi_threshold: f64,
    pub n: usize,
}

/// Valley between the two dominant populations of a 1-D distribution (in the space the values are given in);
/// falls back to the 95th percentile when fewer than two populations are found.
pub fn valley_threshold(vals: &[f64], bins: usize) -> f64 {
    let finite: Vec<f64> = vals.iter().copied().filter(|v| v.is_finite()).collect();
    let (lo, hi) = (stats::percentile(&finite, 0.5), stats::percentile(&finite, 99.5));
    let (counts, edges) = histogram(&finite, bins, Some((lo, hi)));
    let mids: Vec<f64> = (0..bins).map(|i| (edges[i] + edges[i + 1]) / 2.0).collect();
    let win = ((bins / 10) * 2 + 1).max(5);
    let smooth: Vec<f64> = match savgol_filter(&counts, win, 2) {
        Ok(s) => s.into_iter().map(|v| v.max(0.0)).collect(),
        Err(_) => return stats::percentile(&finite, 95.0),
    };
    let smax = smooth.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let pk = find_peaks(&smooth, &PeakOptions { height: Some(smax * 0.05), distance: Some(bins / 8), prominence: None });
    if pk.indices.len() >= 2 {
        let (p1, p2) = (pk.indices[0], pk.indices[1]);
        let mut v = p1;
        for i in p1..=p2 {
            if smooth[i] < smooth[v] {
                v = i; // first minimum, like numpy.argmin
            }
        }
        return mids[v];
    }
    stats::percentile(&finite, 95.0)
}

/// Quadrant analysis. Thresholds are found on the arcsinh scale (`cofactor`; 0 = raw) and mapped back to raw
/// units, so the quadrants are counted on the raw values. Events with a non-finite value are excluded, not
/// silently filed under "live".
pub fn apoptosis(annexin: &[f64], pi: &[f64], cofactor: f64) -> Result<ApoptosisResult, String> {
    if annexin.len() != pi.len() {
        return Err("annexin and PI must have the same events".into());
    }
    let (a, p): (Vec<f64>, Vec<f64>) = annexin.iter().zip(pi).filter(|(a, p)| a.is_finite() && p.is_finite()).map(|(a, p)| (*a, *p)).unzip();
    let n = a.len();
    if n < 100 {
        return Err("Too few finite events for apoptosis analysis.".into());
    }
    let (fwd, inv): (Box<dyn Fn(f64) -> f64>, Box<dyn Fn(f64) -> f64>) = if cofactor > 0.0 { (Box::new(move |v| (v / cofactor).asinh()), Box::new(move |t: f64| cofactor * t.sinh())) } else { (Box::new(|v| v), Box::new(|t| t)) };
    let at: Vec<f64> = a.iter().map(|&v| fwd(v)).collect();
    let pt: Vec<f64> = p.iter().map(|&v| fwd(v)).collect();
    let ann_thr = inv(valley_threshold(&at, 256));
    let pi_thr = inv(valley_threshold(&pt, 256));
    let mut c = [0usize; 4]; // live, early, late, necrotic
    for (x, y) in a.iter().zip(&p) {
        let (ap, pp) = (*x >= ann_thr, *y >= pi_thr);
        c[match (ap, pp) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        }] += 1;
    }
    let pc = |k: usize| 100.0 * c[k] as f64 / n as f64;
    Ok(ApoptosisResult { quadrants: Quadrants { live: pc(0), early: pc(1), late: pc(2), necrotic: pc(3) }, counts: c, annexin_threshold: ann_thr, pi_threshold: pi_thr, n })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn normal(rng: &mut Rng) -> f64 {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    /// A synthetic DNA histogram population: 55% G1 at 200, 15% G2/M at 400, 30% S uniform between.
    fn dna(n: usize, seed: u64) -> Vec<f64> {
        let mut rng = Rng::new(seed);
        (0..n)
            .map(|_| {
                let u = rng.next_f64();
                if u < 0.55 {
                    200.0 + 10.0 * normal(&mut rng)
                } else if u < 0.70 {
                    400.0 + 20.0 * normal(&mut rng)
                } else {
                    let s = 200.0 + 200.0 * rng.next_f64();
                    s + 10.0 * normal(&mut rng)
                }
            })
            .collect()
    }

    #[test]
    fn cell_cycle_recovers_the_phase_fractions_peaks_and_cvs() {
        let r = cell_cycle(&dna(60_000, 1), CycleModel::DeanJettFox, &CycleOptions::default()).unwrap();
        assert!((r.g1_pct - 55.0).abs() < 4.0 && (r.g2_pct - 15.0).abs() < 3.0 && (r.s_pct - 30.0).abs() < 5.0, "G1 {:.1} S {:.1} G2 {:.1}", r.g1_pct, r.s_pct, r.g2_pct);
        assert!((r.g1_pct + r.s_pct + r.g2_pct - 100.0).abs() < 1e-9);
        assert!((r.mu_g1 - 200.0).abs() < 3.0 && (r.mu_g2 - 2.0 * r.mu_g1).abs() < 1e-9);
        assert!((r.cv_g1 - 5.0).abs() < 1.0, "CV G1 {}", r.cv_g1);
        assert!(r.r2 > 0.97, "R2 {}", r.r2);
        assert!(r.boundaries[0] < r.boundaries[1] && r.boundaries[1] < r.boundaries[2] && r.boundaries[2] < r.boundaries[3]);
        // Watson ties the G2 width to the G1 width
        let w = cell_cycle(&dna(60_000, 1), CycleModel::Watson, &CycleOptions::default()).unwrap();
        assert!((w.cv_g2 - w.cv_g1).abs() < 1e-9, "equal CV by construction");
        // a hint pins the G1 seed (G2 hint is halved)
        let h = cell_cycle(&dna(60_000, 1), CycleModel::Watson, &CycleOptions { mu_g2_hint: Some(400.0), ..Default::default() }).unwrap();
        assert!((h.mu_g1 - 200.0).abs() < 25.0);
    }

    #[test]
    fn cell_cycle_refuses_unusable_input() {
        assert!(cell_cycle(&[1.0; 50], CycleModel::Watson, &CycleOptions::default()).unwrap_err().contains("Too few"));
        let flat = vec![5.0; 500];
        assert!(cell_cycle(&flat, CycleModel::Watson, &CycleOptions::default()).is_err(), "a constant signal has no cell-cycle peak");
        let junk: Vec<f64> = (0..500).map(|i| if i % 2 == 0 { f64::NAN } else { -1.0 }).collect();
        assert!(cell_cycle(&junk, CycleModel::Watson, &CycleOptions::default()).is_err());
    }

    /// Dye dilution: generation g holds 2^g x the cells of the previous after division, spaced log10(2) apart.
    fn dye(seed: u64, n: usize) -> Vec<f64> {
        let mut rng = Rng::new(seed);
        // precursor-corrected cohorts: P_g = 1000 for g=0..3 equal => N_g = P_g * 2^g
        let weights = [1.0, 2.0, 4.0, 8.0];
        let tot: f64 = weights.iter().sum();
        (0..n)
            .map(|_| {
                let mut u = rng.next_f64() * tot;
                let mut g = 0;
                while g < 3 && u > weights[g] {
                    u -= weights[g];
                    g += 1;
                }
                10f64.powf(4.2 - g as f64 * 2f64.log10() + 0.05 * normal(&mut rng))
            })
            .collect()
    }

    #[test]
    fn proliferation_finds_the_generations_and_the_precursor_corrected_indices() {
        let r = proliferation(&dye(3, 80_000), 8, None).unwrap();
        let gens: Vec<usize> = r.generations.iter().map(|g| g.gen).collect();
        assert_eq!(gens, vec![0, 1, 2, 3], "four contiguous generations");
        // observed fractions 1/15, 2/15, 4/15, 8/15
        for (g, want) in r.generations.iter().zip([1.0 / 15.0, 2.0 / 15.0, 4.0 / 15.0, 8.0 / 15.0]) {
            assert!((g.pct / 100.0 - want).abs() < 0.03, "gen {} {:.3} vs {want:.3}", g.gen, g.pct / 100.0);
        }
        // equal precursor cohorts for g = 0..3: DI = (0+1+2+3)/4 = 1.5, PI = (1+2+3)/3 = 2, divided = 3/4
        assert!((r.di - 1.5).abs() < 0.12 && (r.pi - 2.0).abs() < 0.12 && (r.pct_divided - 75.0).abs() < 4.0, "DI {} PI {} divided {}", r.di, r.pi, r.pct_divided);
        assert_eq!(r.curves.len(), 4);
        assert!(proliferation(&[1.0; 20], 8, None).is_err());
    }

    #[test]
    fn savgol_valley_threshold_sits_between_two_populations() {
        let mut rng = Rng::new(5);
        let v: Vec<f64> = (0..20_000).map(|i| if i % 4 == 0 { 8.0 + 0.6 * normal(&mut rng) } else { 2.0 + 0.6 * normal(&mut rng) }).collect();
        let t = valley_threshold(&v, 256);
        assert!(t > 3.5 && t < 6.5, "{t}");
        // A unimodal sample: scipy (find_peaks + savgol_filter on the same data) finds two noise peaks, at bins
        // 26 and 114, in the smoothed histogram and the engine's rule returns the valley between them,
        // 0.9872782914767502. The port matches scipy exactly, weakness of the rule included; it is not
        // corrected silently (the fallback to the 95th percentile only applies when fewer than two peaks exist).
        let one: Vec<f64> = (0..5000).map(|_| 3.0 + normal(&mut rng)).collect();
        assert!((valley_threshold(&one, 256) - 0.9872782914767502).abs() < 1e-9);
        // the 95th-percentile fallback: used when the histogram is too short to smooth (or has <2 peaks)
        let few: Vec<f64> = (0..400).map(|i| i as f64).collect();
        assert!((valley_threshold(&few, 4) - stats::percentile(&few, 95.0)).abs() < 1e-12);
    }

    #[test]
    fn apoptosis_quadrants_partition_the_events_and_exclude_non_finite() {
        let mut rng = Rng::new(7);
        let n = 20_000;
        let (mut a, mut p) = (Vec::new(), Vec::new());
        for i in 0..n {
            // 60% live, 15% early, 15% late, 10% necrotic; raw units on a log-ish scale
            let (an, pn) = match i % 20 {
                0..=11 => (false, false),
                12..=14 => (true, false),
                15..=17 => (true, true),
                _ => (false, true),
            };
            a.push(if an { 5000.0 * (0.25 * normal(&mut rng)).exp() } else { 150.0 * (0.4 * normal(&mut rng)).exp() });
            p.push(if pn { 4000.0 * (0.25 * normal(&mut rng)).exp() } else { 120.0 * (0.4 * normal(&mut rng)).exp() });
        }
        a.push(f64::NAN);
        p.push(10.0);
        a.push(10.0);
        p.push(f64::INFINITY);
        let r = apoptosis(&a, &p, 150.0).unwrap();
        assert_eq!(r.n, n, "the two corrupt events are excluded, not counted as live");
        assert_eq!(r.counts.iter().sum::<usize>(), n);
        let q = r.quadrants;
        for (got, want) in [(q.live, 60.0), (q.early, 15.0), (q.late, 15.0), (q.necrotic, 10.0)] {
            assert!((got - want).abs() < 2.0, "{q:?}");
        }
        assert!((q.live + q.early + q.late + q.necrotic - 100.0).abs() < 1e-9);
        assert!(r.annexin_threshold > 200.0 && r.annexin_threshold < 4000.0, "{}", r.annexin_threshold);
        assert!(apoptosis(&a[..50], &p[..50], 150.0).is_err() && apoptosis(&a, &p[..10], 150.0).is_err());
    }
}
