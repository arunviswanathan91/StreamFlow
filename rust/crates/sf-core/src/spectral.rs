//! Spectral unmixing. Port of `engine/spectral.py`, and checked against it (see `tests/reference.rs`).
//!
//! Clean-room from published methods (StreamFLOW is Apache-2.0):
//! * OLS / WLS unmixing: Novo, Gregori & Rajwa, Cytometry A 2013. `X = E Wt`,
//!   `W = (M Omega^-1 M^T)^-1 M Omega^-1`, Omega = diag(per-detector mean) (Poisson noise); OLS is
//!   Omega = I.
//! * Per-cell autofluorescence: a library of AF spectra from an unstained sample; every event is
//!   unmixed against each candidate and keeps the one with the lowest residual.
//! * Signature QC: cosine similarity and a collinearity index from the inverse Gram matrix.
//!
//! Layout: events are `n x d` row-major (`d` detectors); the signature matrix `M` is `f x d` (one
//! fluorophore per row, each scaled so its peak detector is 1).

use crate::kmeans::kmeans;
use crate::linalg::{self, LinAlgError};
use crate::stats;
use std::fmt;

const EPS: f64 = 1e-12;

#[derive(Debug)]
pub enum SpectralError {
    LinAlg(LinAlgError),
    Msg(String),
}

impl fmt::Display for SpectralError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpectralError::LinAlg(e) => write!(f, "unmixing failed: {e}"),
            SpectralError::Msg(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for SpectralError {}

impl From<LinAlgError> for SpectralError {
    fn from(e: LinAlgError) -> Self {
        SpectralError::LinAlg(e)
    }
}

fn msg<T>(m: impl Into<String>) -> Result<T, SpectralError> {
    Err(SpectralError::Msg(m.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Ols,
    Wls,
}

impl Method {
    pub fn parse(s: &str) -> Result<Method, SpectralError> {
        match s.to_ascii_lowercase().as_str() {
            "ols" => Ok(Method::Ols),
            "wls" => Ok(Method::Wls),
            other => msg(format!("Unknown unmixing method {other:?} (use 'ols' or 'wls').")),
        }
    }
}

fn column(events: &[f64], n: usize, d: usize, j: usize) -> Vec<f64> {
    (0..n).map(|i| events[i * d + j]).collect()
}

// ---- numpy-faithful Otsu (identical binning to np.histogram with equal bins) ----------------------

/// `np.histogram(v, bins)` for equal-width bins over `[min, max]`, replicating numpy's index
/// computation *and* its edge-correction steps so boundary values land in the same bin.
fn np_histogram(v: &[f64], bins: usize) -> (Vec<f64>, Vec<f64>) {
    let (lo, hi) = (stats::min(v), stats::max(v));
    let step = (hi - lo) / bins as f64;
    let mut edges: Vec<f64> = (0..=bins).map(|i| i as f64 * step + lo).collect();
    edges[bins] = hi;
    let norm = bins as f64 / (hi - lo);
    let mut hist = vec![0.0; bins];
    for &x in v {
        let mut i = ((x - lo) * norm) as usize;
        if i == bins {
            i -= 1;
        }
        if x < edges[i] {
            i -= 1;
        } else if x >= edges[i + 1] && i != bins - 1 {
            i += 1;
        }
        hist[i] += 1.0;
    }
    (hist, edges)
}

fn otsu_np(v: &[f64]) -> Option<f64> {
    let v: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if v.len() < 2 || stats::max(&v) - stats::min(&v) <= 0.0 {
        return None;
    }
    let bins = 256;
    let (hist, edges) = np_histogram(&v, bins);
    let total: f64 = hist.iter().sum();
    let p: Vec<f64> = hist.iter().map(|h| h / total).collect();
    let centres: Vec<f64> = (0..bins).map(|i| (edges[i] + edges[i + 1]) / 2.0).collect();
    let (mut w0, mut m) = (vec![0.0; bins], vec![0.0; bins]);
    let (mut cw, mut cm) = (0.0, 0.0);
    for i in 0..bins {
        cw += p[i];
        cm += p[i] * centres[i];
        w0[i] = cw;
        m[i] = cm;
    }
    let mt = m[bins - 1];
    let mut best: Option<(usize, f64)> = None;
    for i in 0..bins {
        let denom = w0[i] * (1.0 - w0[i]);
        if denom <= 0.0 {
            continue;
        }
        let between = (mt * w0[i] - m[i]).powi(2) / denom;
        if between.is_nan() {
            continue;
        }
        if best.map_or(true, |(_, b)| between > b) {
            best = Some((i, between));
        }
    }
    best.map(|(i, _)| centres[i])
}

// ---- signatures --------------------------------------------------------------------------------

/// True for events NOT pinned at the top of the detector range in any detector.
/// `ceiling` is per-detector (`None` = the observed column maximum); `frac` is typically 0.999.
pub fn saturation_mask(events: &[f64], n: usize, d: usize, ceiling: Option<&[f64]>, frac: f64) -> Vec<bool> {
    let top: Vec<f64> = match ceiling {
        Some(c) => c.to_vec(),
        None => (0..d).map(|j| stats::max(&column(events, n, d, j))).collect(),
    };
    (0..n).map(|i| !(0..d).any(|j| events[i * d + j] >= frac * top[j])).collect()
}

/// The detector that best characterises a single-stain control: argmax |loading| of the first
/// principal component of the median-centred events. Robust to a dim control whose brightest
/// *median* detector is dominated by autofluorescence.
pub fn peak_detector(events: &[f64], n: usize, d: usize) -> usize {
    let med: Vec<f64> = (0..d).map(|j| stats::median(&column(events, n, d, j))).collect();
    if n < 2 {
        return med.iter().enumerate().fold((0, f64::MIN), |b, (j, &v)| if v > b.1 { (j, v) } else { b }).0;
    }
    let mut cov = vec![0.0; d * d];
    for i in 0..n {
        for a in 0..d {
            let xa = events[i * d + a] - med[a];
            for b in a..d {
                cov[a * d + b] += xa * (events[i * d + b] - med[b]);
            }
        }
    }
    for a in 0..d {
        for b in 0..a {
            cov[a * d + b] = cov[b * d + a];
        }
    }
    let (_, vecs) = linalg::sym_eig(&cov, d).expect("square by construction");
    (0..d).fold((0, -1.0f64), |best, j| if vecs[j * d].abs() > best.1 { (j, vecs[j * d].abs()) } else { best }).0
}

#[derive(Debug, Clone, Default)]
pub struct Signature {
    pub ok: bool,
    pub reason: String,
    pub peak: Option<usize>,
    pub n_pos: usize,
    pub n_neg: usize,
    pub n_saturated: usize,
    pub threshold: Option<f64>,
    /// Peak-scaled signature (`d` values) when `ok`.
    pub signature: Option<Vec<f64>>,
}

/// Signature of one single-stain control: `median(positive) - negative baseline`, peak-scaled to 1.
/// `negative` is a per-detector baseline (e.g. the unstained median); `None` uses the control's own
/// negative population. `saturation` is a per-detector ceiling for excluding pinned events.
pub fn build_signature(events: &[f64], n: usize, d: usize, negative: Option<&[f64]>, min_pos: usize, saturation: Option<&[f64]>) -> Signature {
    let mut out = Signature::default();
    if n < min_pos * 2 {
        out.reason = format!("too few events ({n})");
        return out;
    }
    let (mut ev, mut n) = (events.to_vec(), n);
    if let Some(ceil) = saturation {
        let keep = saturation_mask(&ev, n, d, Some(ceil), 0.999);
        out.n_saturated = keep.iter().filter(|k| !**k).count();
        let kept = n - out.n_saturated;
        if kept >= min_pos * 2 {
            let mut e2 = Vec::with_capacity(kept * d);
            for i in 0..n {
                if keep[i] {
                    e2.extend_from_slice(&ev[i * d..(i + 1) * d]);
                }
            }
            ev = e2;
            n = kept;
        }
    }
    let pk = peak_detector(&ev, n, d);
    out.peak = Some(pk);
    let pkcol = column(&ev, n, d, pk);
    let scaled: Vec<f64> = pkcol.iter().map(|x| (x / 150.0).asinh()).collect();
    let Some(thr) = otsu_np(&scaled) else {
        out.reason = "no bimodal split on the peak detector".into();
        return out;
    };
    let thr_raw = thr.sinh() * 150.0;
    out.threshold = Some(thr_raw);
    let pos: Vec<bool> = pkcol.iter().map(|&x| x > thr_raw).collect();
    out.n_pos = pos.iter().filter(|p| **p).count();
    out.n_neg = n - out.n_pos;
    if out.n_pos < min_pos {
        out.reason = format!("only {} positive events", out.n_pos);
        return out;
    }
    let median_of = |select: bool| -> Vec<f64> {
        (0..d).map(|j| stats::median(&(0..n).filter(|&i| pos[i] == select).map(|i| ev[i * d + j]).collect::<Vec<_>>())).collect()
    };
    let base: Vec<f64> = match negative {
        Some(b) => b.to_vec(),
        None if out.n_neg < min_pos => vec![0.0; d],
        None => median_of(false),
    };
    let pos_med = median_of(true);
    let sig: Vec<f64> = (0..d).map(|j| (pos_med[j] - base[j]).max(0.0)).collect();
    if sig[pk] <= 0.0 {
        out.reason = "positive population is not brighter than the negative".into();
        return out;
    }
    out.signature = Some(sig.iter().map(|v| v / sig[pk]).collect());
    out.ok = true;
    out
}

#[derive(Debug, Clone)]
pub struct Qc {
    /// `f x f` cosine similarity of the signatures.
    pub cosine: Vec<f64>,
    /// Per-fluorophore collinearity index: 1.0 for an orthogonal set, growing without bound as a
    /// fluorophore becomes explainable by the others (unmixing then amplifies noise by that factor).
    pub collinearity: Vec<f64>,
    /// 2-norm condition number of `M`.
    pub condition: f64,
}

/// Norm of `v` after removing its projection onto span(`rows`), by modified Gram-Schmidt with
/// re-orthogonalisation. Rank-tolerant: a row already in the span adds nothing to the basis, so a
/// *dependent* set is handled rather than producing rounding garbage.
fn span_residual(rows: &[&[f64]], v: &[f64], rel_tol: f64) -> f64 {
    let norm = |x: &[f64]| x.iter().map(|a| a * a).sum::<f64>().sqrt();
    let project_out = |w: &mut Vec<f64>, basis: &[Vec<f64>]| {
        for _ in 0..2 {
            for q in basis {
                let dot: f64 = q.iter().zip(w.iter()).map(|(a, b)| a * b).sum();
                for (wi, qi) in w.iter_mut().zip(q) {
                    *wi -= dot * qi;
                }
            }
        }
    };
    let mut basis: Vec<Vec<f64>> = Vec::new();
    for r in rows {
        let mut w = r.to_vec();
        let n0 = norm(&w);
        project_out(&mut w, &basis);
        let n1 = norm(&w);
        if n0 > 0.0 && n1 > rel_tol * n0 {
            basis.push(w.iter().map(|x| x / n1).collect());
        }
    }
    let mut w = v.to_vec();
    project_out(&mut w, &basis);
    norm(&w)
}

/// Cosine similarity, collinearity index and condition number of a signature set `m (f x d)`.
///
/// Collinearity of row `i` is `||m_i|| / dist(m_i, span of the other rows)` = `sqrt((G^-1)_ii) ||m_i||`
/// for the Gram matrix `G = M M^T`, computed without inverting `G`. It is `inf` when a fluorophore lies
/// in the span of the others (to a relative 1e-9): such a set cannot be unmixed. (Inverting `G` instead
/// gives rounding garbage for a dependent set, which an earlier Python version clipped to 0.0.)
pub fn signature_qc(m: &[f64], f: usize, d: usize) -> Qc {
    const REL_TOL: f64 = 1e-9;
    let row = |i: usize| &m[i * d..(i + 1) * d];
    let norms: Vec<f64> = (0..f).map(|i| row(i).iter().map(|x| x * x).sum::<f64>().sqrt()).collect();
    let mut cosine = vec![0.0; f * f];
    for a in 0..f {
        for b in 0..f {
            let dot: f64 = (0..d).map(|j| m[a * d + j] * m[b * d + j]).sum();
            cosine[a * f + b] = dot / (norms[a].max(EPS) * norms[b].max(EPS));
        }
    }
    let collinearity = (0..f)
        .map(|i| {
            let others: Vec<&[f64]> = (0..f).filter(|&k| k != i).map(row).collect();
            let res = span_residual(&others, row(i), REL_TOL);
            if norms[i] == 0.0 || res <= REL_TOL * norms[i] { f64::INFINITY } else { norms[i] / res }
        })
        .collect();
    // 2-norm condition number from the singular values of M (not of the Gram matrix, which squares it)
    let condition = match linalg::singular_values(m, f, d) {
        Ok(sv) if !sv.is_empty() => {
            let (hi, lo) = (sv[0], sv[sv.len() - 1]);
            if lo <= 0.0 { f64::INFINITY } else { hi / lo }
        }
        _ => f64::NAN,
    };
    Qc { cosine, collinearity, condition }
}

// ---- unmixing -----------------------------------------------------------------------------------

/// `W (f x d)` with `abundances = events @ W^T`. `weights = None` is OLS; `Some(w)` is WLS with
/// `Omega = diag(w)`, floored so a silent detector cannot dominate.
pub fn unmixing_matrix(m: &[f64], f: usize, d: usize, weights: Option<&[f64]>) -> Result<Vec<f64>, SpectralError> {
    let mt = linalg::transpose(m, f, d);
    match weights {
        None => Ok(linalg::solve(&linalg::matmul(m, &mt, f, d, f), f, m, d)?),
        Some(w) => {
            if w.len() != d {
                return msg(format!("{} weights for {d} detectors", w.len()));
            }
            let mw: Vec<f64> = (0..f * d).map(|idx| m[idx] / w[idx % d].max(EPS)).collect();
            Ok(linalg::solve(&linalg::matmul(&mw, &mt, f, d, f), f, &mw, d)?)
        }
    }
}

/// Per-detector mean of the (non-negative part of the) events, floored: the Poisson variance estimate.
pub fn poisson_weights(events: &[f64], n: usize, d: usize, floor: f64) -> Vec<f64> {
    (0..d).map(|j| ((0..n).map(|i| events[i * d + j].max(0.0)).sum::<f64>() / n.max(1) as f64).max(floor)).collect()
}

/// Unmix `events (n x d)` against `m (f x d)`. Returns `(abundances n x f, W f x d)`.
pub fn unmix(events: &[f64], n: usize, d: usize, m: &[f64], f: usize, method: Method, weights: Option<&[f64]>) -> Result<(Vec<f64>, Vec<f64>), SpectralError> {
    let w = match method {
        Method::Ols => unmixing_matrix(m, f, d, None)?,
        Method::Wls => {
            let own;
            let wt = match weights {
                Some(w) => w,
                None => {
                    own = poisson_weights(events, n, d, 1.0);
                    &own
                }
            };
            unmixing_matrix(m, f, d, Some(wt))?
        }
    };
    let ab = linalg::matmul(events, &linalg::transpose(&w, f, d), n, d, f);
    Ok((ab, w))
}

/// Root-sum-square residual per event: how much the model failed to explain.
pub fn residuals(events: &[f64], n: usize, d: usize, m: &[f64], f: usize, abundances: &[f64]) -> Vec<f64> {
    (0..n)
        .map(|i| {
            (0..d)
                .map(|j| {
                    let model: f64 = (0..f).map(|k| abundances[i * f + k] * m[k * d + j]).sum();
                    (events[i * d + j] - model).powi(2)
                })
                .sum::<f64>()
                .sqrt()
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct AfResult {
    /// `n x (f + 1)`; the last column is the autofluorescence abundance.
    pub abundances: Vec<f64>,
    /// Chosen library entry per event.
    pub af_index: Vec<usize>,
    /// **Squared** residual of the chosen candidate (matches the Python reference).
    pub residual2: Vec<f64>,
}

/// Per-cell autofluorescence: unmix against `M` plus each library signature; keep the lowest residual
/// (the first candidate wins ties).
pub fn unmix_with_af(events: &[f64], n: usize, d: usize, m: &[f64], f: usize, lib: &[f64], k: usize, method: Method, weights: Option<&[f64]>) -> Result<AfResult, SpectralError> {
    let own;
    let w: Option<&[f64]> = match (method, weights) {
        (Method::Ols, _) => None,
        (Method::Wls, Some(w)) => Some(w),
        (Method::Wls, None) => {
            own = poisson_weights(events, n, d, 1.0);
            Some(&own)
        }
    };
    let (f1, mut abundances) = (f + 1, vec![0.0; n * (f + 1)]);
    let (mut af_index, mut best) = (vec![0usize; n], vec![f64::INFINITY; n]);
    for c in 0..k {
        let mut mk = m.to_vec();
        mk.extend_from_slice(&lib[c * d..(c + 1) * d]);
        let wk = unmixing_matrix(&mk, f1, d, w)?;
        for i in 0..n {
            let ev = &events[i * d..(i + 1) * d];
            let ab: Vec<f64> = (0..f1).map(|a| (0..d).map(|j| wk[a * d + j] * ev[j]).sum()).collect();
            let res: f64 = (0..d).map(|j| (ev[j] - (0..f1).map(|a| ab[a] * mk[a * d + j]).sum::<f64>()).powi(2)).sum();
            if res < best[i] {
                best[i] = res;
                af_index[i] = c;
                abundances[i * f1..(i + 1) * f1].copy_from_slice(&ab);
            }
        }
    }
    Ok(AfResult { abundances, af_index, residual2: best })
}

/// Cluster an unstained sample into up to `k` autofluorescence signatures (shape-based: spectra are
/// L2-normalised before clustering). Seeded and reproducible. Returns `(library k' x d, counts)`,
/// largest cluster first; `k'` may be smaller than `k` for a small sample.
pub fn extract_af_library(unstained: &[f64], n: usize, d: usize, k: usize, seed: u64, min_events: usize) -> Result<(Vec<f64>, Vec<usize>), SpectralError> {
    let rows: Vec<usize> = (0..n).filter(|&i| unstained[i * d..(i + 1) * d].iter().map(|x| x.max(0.0).powi(2)).sum::<f64>() > 0.0).collect();
    if rows.len() < min_events {
        return msg(format!("Unstained sample has only {} usable events (need >= {min_events}) for an autofluorescence library.", rows.len()));
    }
    let nn = rows.len();
    let e: Vec<f64> = rows.iter().flat_map(|&i| unstained[i * d..(i + 1) * d].iter().map(|x| x.max(0.0))).collect();
    let unit: Vec<f64> = (0..nn)
        .flat_map(|i| {
            let norm = e[i * d..(i + 1) * d].iter().map(|x| x * x).sum::<f64>().sqrt();
            e[i * d..(i + 1) * d].iter().map(move |x| x / norm)
        })
        .collect();
    let k = k.clamp(1, (nn / 50).max(1));
    let km = kmeans(&unit, nn, d, k, 10, 300, seed).map_err(SpectralError::Msg)?;
    let mut lib = Vec::new();
    let mut counts = Vec::new();
    for c in 0..k {
        let members: Vec<usize> = (0..nn).filter(|&i| km.labels[i] == c).collect();
        let sig: Vec<f64> = (0..d).map(|j| stats::median(&members.iter().map(|&i| e[i * d + j]).collect::<Vec<_>>())).collect();
        let mx = sig.iter().cloned().fold(f64::MIN, f64::max).max(EPS);
        lib.push(sig.iter().map(|v| v / mx).collect::<Vec<f64>>());
        counts.push(members.len());
    }
    let mut order: Vec<usize> = (0..k).collect();
    order.sort_by_key(|&i| counts[i]); // stable ascending ...
    order.reverse(); // ... then largest first
    Ok((order.iter().flat_map(|&i| lib[i].clone()).collect(), order.iter().map(|&i| counts[i]).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    const D: usize = 12;

    fn sigs() -> Vec<f64> {
        let mut m = Vec::new();
        for pk in [1.0, 4.0, 7.0, 10.0] {
            m.extend((0..D).map(|x| (-0.5 * ((x as f64 - pk) / 1.4).powi(2)).exp()));
        }
        m
    }

    #[test]
    fn ols_is_wls_with_unit_weights_and_w_mt_is_identity() {
        let m = sigs();
        let (ols, wls) = (unmixing_matrix(&m, 4, D, None).unwrap(), unmixing_matrix(&m, 4, D, Some(&[1.0; D])).unwrap());
        for (a, b) in ols.iter().zip(&wls) {
            assert!((a - b).abs() < 1e-12);
        }
        let wmt = linalg::matmul(&ols, &linalg::transpose(&m, 4, D), 4, D, 4);
        for i in 0..4 {
            for j in 0..4 {
                assert!((wmt[i * 4 + j] - if i == j { 1.0 } else { 0.0 }).abs() < 1e-9, "W M^T != I");
            }
        }
    }

    #[test]
    fn unknown_method_and_bad_weights_are_errors() {
        assert!(Method::parse("nope").unwrap_err().to_string().contains("nope"));
        assert!(unmixing_matrix(&sigs(), 4, D, Some(&[1.0; 3])).is_err());
    }

    #[test]
    fn near_collinear_is_large_but_finite_and_a_dependent_set_is_infinite_not_zero() {
        let m = sigs();
        let mut rng = Rng::new(0);
        let noise: Vec<f64> = (0..D).map(|_| rng.next_f64()).collect();
        let mut near = m.clone();
        near.extend((0..D).map(|j| 0.97 * m[D + j] + 0.03 * noise[j]));
        let q = signature_qc(&near, 5, D);
        assert!(q.cosine[1 * 5 + 4] > 0.99);
        assert!(q.collinearity.iter().all(|c| c.is_finite()), "{:?}", q.collinearity);
        assert!(q.collinearity[1] > 5.0 * q.collinearity[0] && q.collinearity[4] > 5.0 * q.collinearity[0], "{:?}", q.collinearity);

        // a signature that is an exact combination of two others: rows 1, 2 and 4 are mutually
        // dependent -> infinite; the independent rows 0 and 3 stay small
        let mut dep = m.clone();
        dep.extend((0..D).map(|j| 0.97 * m[D + j] + 0.03 * m[2 * D + j]));
        let c = signature_qc(&dep, 5, D).collinearity;
        assert!(c[1].is_infinite() && c[2].is_infinite() && c[4].is_infinite(), "{c:?}");
        assert!(c[0].is_finite() && c[3].is_finite() && c[0] < 2.0 && c[3] < 2.0, "{c:?}");
        assert!(signature_qc(&dep, 5, D).condition > 1e12);
    }

    #[test]
    fn af_library_is_deterministic_and_finds_two_spectral_shapes() {
        let mut rng = Rng::new(2);
        let shapes = [(0..D).map(|j| if j < 6 { 1.0 } else { 0.1 }).collect::<Vec<f64>>(), (0..D).map(|j| if j >= 6 { 1.0 } else { 0.1 }).collect()];
        let mut ev = Vec::new();
        for i in 0..2000 {
            let s = &shapes[i % 2];
            let bright = 200.0 + 100.0 * rng.next_f64();
            ev.extend(s.iter().map(|x| bright * x * (1.0 + 0.05 * (rng.next_f64() - 0.5))));
        }
        let (a, ca) = extract_af_library(&ev, 2000, D, 2, 7, 200).unwrap();
        let (b, cb) = extract_af_library(&ev, 2000, D, 2, 7, 200).unwrap();
        assert_eq!((a.clone(), ca.clone()), (b, cb));
        assert_eq!(ca.iter().sum::<usize>(), 2000);
        // one entry is "left-heavy", the other "right-heavy"
        let left = |s: &[f64]| s[..6].iter().sum::<f64>() > s[6..].iter().sum::<f64>();
        assert_ne!(left(&a[..D]), left(&a[D..]));
        assert!(extract_af_library(&ev[..10 * D], 10, D, 2, 7, 200).is_err());
    }

    #[test]
    fn unmixing_recovers_truth_and_per_cell_af_beats_ignoring_af() {
        let m = sigs();
        let mut rng = Rng::new(3);
        let afs: Vec<f64> = (0..2 * D).map(|i| if (i / D == 0) == (i % D < 6) { 1.0 } else { 0.15 }).collect();
        let n = 3000;
        let (mut ev, mut truth) = (Vec::new(), Vec::new());
        for i in 0..n {
            let t: Vec<f64> = (0..4).map(|_| 2000.0 * rng.next_f64()).collect();
            let c = i % 2;
            ev.extend((0..D).map(|j| (0..4).map(|k| t[k] * m[k * D + j]).sum::<f64>() + 400.0 * afs[c * D + j]));
            truth.extend(t);
        }
        let (plain, _) = unmix(&ev, n, D, &m, 4, Method::Wls, None).unwrap();
        let af = unmix_with_af(&ev, n, D, &m, 4, &afs, 2, Method::Wls, None).unwrap();
        let err = |ab: &[f64], stride: usize| (0..n).map(|i| (0..4).map(|k| (ab[i * stride + k] - truth[i * 4 + k]).powi(2)).sum::<f64>()).sum::<f64>() / n as f64;
        let (e_plain, e_af) = (err(&plain, 4), err(&af.abundances, 5));
        assert!(e_af < e_plain * 0.01, "AF-aware error {e_af} vs ignoring AF {e_plain}");
        assert!((0..n).all(|i| af.af_index[i] == i % 2), "AF candidate chosen wrongly");
    }
}
