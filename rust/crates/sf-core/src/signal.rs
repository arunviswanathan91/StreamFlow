//! 1-D signal helpers that mirror the SciPy / NumPy routines the analysis modules were written with, so the
//! same data gives the same peaks: [`find_peaks`] (`scipy.signal.find_peaks`), [`savgol_filter`]
//! (`scipy.signal.savgol_filter`, polynomial edge handling), [`histogram`] (`numpy.histogram`) and
//! [`convolve_same`] (`numpy.convolve(..., mode="same")`).

use crate::linalg::lstsq;

/// Indices of local maxima, scipy's rule: a flat top counts once (at its middle) if it is strictly higher
/// than both neighbours; the two end points never qualify.
fn local_maxima(x: &[f64]) -> Vec<usize> {
    let n = x.len();
    let mut out = Vec::new();
    if n < 3 {
        return out;
    }
    let mut i = 1;
    let i_max = n - 1;
    while i < i_max {
        if x[i - 1] < x[i] {
            let mut ahead = i + 1;
            while ahead < i_max && x[ahead] == x[i] {
                ahead += 1;
            }
            if x[ahead] < x[i] {
                out.push((i + ahead - 1) / 2);
                i = ahead;
            }
        }
        i += 1;
    }
    out
}

/// Keep the highest peaks, removing any peak closer than `distance` samples to a higher one.
fn select_by_distance(peaks: &[usize], priority: &[f64], distance: usize) -> Vec<usize> {
    let n = peaks.len();
    let mut keep = vec![true; n];
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| priority[a].partial_cmp(&priority[b]).unwrap()); // stable: ties keep position order
    for &i in order.iter().rev() {
        if !keep[i] {
            continue;
        }
        let mut j = i;
        while j > 0 {
            j -= 1;
            if peaks[i] - peaks[j] >= distance {
                break;
            }
            keep[j] = false;
        }
        let mut j = i + 1;
        while j < n && peaks[j] - peaks[i] < distance {
            keep[j] = false;
            j += 1;
        }
    }
    peaks.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect()
}

/// Topographic prominence of each peak (whole signal as the window).
pub fn peak_prominences(x: &[f64], peaks: &[usize]) -> Vec<f64> {
    let n = x.len();
    peaks
        .iter()
        .map(|&p| {
            let h = x[p];
            let mut left_min = h;
            let mut i = p as isize;
            while i >= 0 && x[i as usize] <= h {
                left_min = left_min.min(x[i as usize]);
                i -= 1;
            }
            let mut right_min = h;
            let mut i = p;
            while i < n && x[i] <= h {
                right_min = right_min.min(x[i]);
                i += 1;
            }
            h - left_min.max(right_min)
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
pub struct PeakOptions {
    pub height: Option<f64>,
    pub prominence: Option<f64>,
    pub distance: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Peaks {
    pub indices: Vec<usize>,
    /// Prominence of each returned peak (only meaningful when a prominence option was given, but always filled).
    pub prominences: Vec<f64>,
}

/// `scipy.signal.find_peaks` with the `height`, `distance` and `prominence` conditions, applied in scipy's
/// order: local maxima, height, distance, prominence.
pub fn find_peaks(x: &[f64], opt: &PeakOptions) -> Peaks {
    let mut peaks = local_maxima(x);
    if let Some(h) = opt.height {
        peaks.retain(|&p| x[p] >= h);
    }
    if let Some(d) = opt.distance {
        let pr: Vec<f64> = peaks.iter().map(|&p| x[p]).collect();
        peaks = select_by_distance(&peaks, &pr, d.max(1));
    }
    let mut prom = peak_prominences(x, &peaks);
    if let Some(pmin) = opt.prominence {
        let keep: Vec<bool> = prom.iter().map(|&p| p >= pmin).collect();
        peaks = peaks.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect();
        prom = prom.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect();
    }
    Peaks { indices: peaks, prominences: prom }
}

/// `numpy.convolve(y, kernel, mode="same")` for an odd-length kernel: output has the length of `y`, zero padded.
pub fn convolve_same(y: &[f64], kernel: &[f64]) -> Vec<f64> {
    let (n, m) = (y.len() as isize, kernel.len() as isize);
    let half = (m - 1) / 2;
    (0..n)
        .map(|i| {
            let mut s = 0.0;
            for (k, kv) in kernel.iter().enumerate() {
                // full convolution index c = i + half ; y[j] * kernel[c - j]
                let j = i + half - k as isize;
                if j >= 0 && j < n {
                    s += y[j as usize] * kv;
                }
            }
            s
        })
        .collect()
}

/// `numpy.histogram(values, bins, range)`: equal-width bins, the last bin includes its upper edge. Values
/// outside the range (and NaN) are ignored. Returns `(counts, edges)`; with no `range` it is `(min, max)`.
pub fn histogram(values: &[f64], bins: usize, range: Option<(f64, f64)>) -> (Vec<f64>, Vec<f64>) {
    let finite = || values.iter().copied().filter(|v| v.is_finite());
    let (lo, hi) = range.unwrap_or_else(|| (finite().fold(f64::INFINITY, f64::min), finite().fold(f64::NEG_INFINITY, f64::max)));
    let (lo, hi) = if lo == hi { (lo - 0.5, hi + 0.5) } else { (lo, hi) }; // numpy widens a degenerate range
    let edges: Vec<f64> = (0..=bins).map(|i| lo + (hi - lo) * i as f64 / bins as f64).collect();
    let mut counts = vec![0.0; bins];
    for v in finite() {
        if v < lo || v > hi {
            continue;
        }
        let mut b = (((v - lo) / (hi - lo)) * bins as f64) as usize;
        if b >= bins {
            b = bins - 1;
        }
        // guard the float edge cases numpy also corrects for
        if v < edges[b] && b > 0 {
            b -= 1;
        } else if b + 1 < bins && v >= edges[b + 1] {
            b += 1;
        }
        counts[b] += 1.0;
    }
    (counts, edges)
}

/// Savitzky-Golay smoothing (`deriv = 0`) with polynomial order `order` and odd `window`, edges handled like
/// scipy's default `mode="interp"`: the first/last `window` points are fitted with one polynomial and
/// evaluated at the edge positions.
pub fn savgol_filter(x: &[f64], window: usize, order: usize) -> Result<Vec<f64>, String> {
    let n = x.len();
    if window % 2 == 0 || window == 0 {
        return Err("savgol window must be odd".into());
    }
    if order >= window {
        return Err("savgol order must be below the window length".into());
    }
    if window > n {
        return Err(format!("savgol window {window} is longer than the signal ({n})"));
    }
    let half = (window / 2) as isize;
    // interior: convolution with the centre row of the pseudo-inverse of the Vandermonde matrix
    let cols = order + 1;
    let mut a = vec![0.0; window * cols];
    for r in 0..window {
        let t = r as f64 - half as f64;
        for c in 0..cols {
            a[r * cols + c] = t.powi(c as i32);
        }
    }
    // coefficient vector for the value at the centre: solve A^T A c = A^T e_centre-row ... easiest via lstsq on identity columns
    let mut coef = vec![0.0; window];
    for (j, cj) in coef.iter_mut().enumerate() {
        let mut e = vec![0.0; window];
        e[j] = 1.0;
        let fit = lstsq(&a, window, cols, &e).map_err(|e| e.to_string())?;
        *cj = fit[0]; // constant term = fitted value at the centre
    }
    let mut out = vec![0.0; n];
    for i in half as usize..n - half as usize {
        out[i] = (0..window).map(|k| coef[k] * x[i + k - half as usize]).sum();
    }
    // edges: polynomial fit over the first / last `window` points
    let fit_edge = |seg: &[f64], positions: &mut dyn Iterator<Item = (usize, f64)>, out: &mut [f64]| -> Result<(), String> {
        let mut v = vec![0.0; window * cols];
        for r in 0..window {
            for c in 0..cols {
                v[r * cols + c] = (r as f64).powi(c as i32);
            }
        }
        let p = lstsq(&v, window, cols, seg).map_err(|e| e.to_string())?;
        for (idx, t) in positions {
            out[idx] = (0..cols).map(|c| p[c] * t.powi(c as i32)).sum();
        }
        Ok(())
    };
    fit_edge(&x[..window], &mut (0..half as usize).map(|i| (i, i as f64)), &mut out)?;
    fit_edge(&x[n - window..], &mut (n - half as usize..n).map(|i| (i, (i - (n - window)) as f64)), &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_maxima_handle_plateaus_and_ends() {
        assert_eq!(local_maxima(&[0.0, 1.0, 0.0, 2.0, 2.0, 2.0, 1.0, 3.0]), vec![1, 4], "a flat top counts once, at its middle; the last point never does");
        assert_eq!(local_maxima(&[3.0, 2.0, 1.0]), Vec::<usize>::new(), "monotone: no interior maximum");
        assert_eq!(local_maxima(&[1.0, 2.0, 2.0, 1.0]), vec![1], "plateau of two: the left of the middle pair (integer division)");
        assert!(local_maxima(&[1.0, 2.0]).is_empty());
    }

    #[test]
    fn prominence_is_height_above_the_higher_of_the_two_base_minima() {
        let x = [0.0, 3.0, 1.0, 2.0, 0.5, 4.0, 0.0];
        let p = find_peaks(&x, &PeakOptions::default());
        assert_eq!(p.indices, vec![1, 3, 5]);
        // peak 1 (3): left base 0, right base 0.5 (the scan stops at the taller peak at index 5) -> prominence 3 - 0.5
        // peak 3 (2): left min 1 (stops at the higher peak 1), right min 0.5 -> 2 - max(1, 0.5) = 1
        // peak 5 (4): 4 - max(0, 0) = 4 (nothing higher either side)
        assert_eq!(p.prominences, vec![2.5, 1.0, 4.0]);
        let q = find_peaks(&x, &PeakOptions { prominence: Some(2.0), ..Default::default() });
        assert_eq!(q.indices, vec![1, 5], "prominence 2.5 and 4 pass the 2.0 bar, 1.0 does not");
        let h = find_peaks(&x, &PeakOptions { height: Some(3.5), ..Default::default() });
        assert_eq!(h.indices, vec![5]);
    }

    #[test]
    fn distance_keeps_the_taller_of_two_close_peaks() {
        let x = [0.0, 5.0, 0.0, 6.0, 0.0, 0.0, 0.0, 4.0, 0.0];
        let p = find_peaks(&x, &PeakOptions { distance: Some(3), ..Default::default() });
        assert_eq!(p.indices, vec![3, 7], "the 5 at index 1 is within 3 of the taller 6 at index 3");
        assert_eq!(find_peaks(&x, &PeakOptions { distance: Some(1), ..Default::default() }).indices, vec![1, 3, 7]);
    }

    #[test]
    fn convolve_same_matches_numpy_on_a_hand_example() {
        // np.convolve([1,2,3,4], ones(3)/3, 'same') = [1, 2, 3, 2.3333]
        let r = convolve_same(&[1.0, 2.0, 3.0, 4.0], &[1.0 / 3.0; 3]);
        assert_eq!(r.len(), 4);
        for (a, b) in r.iter().zip([1.0, 2.0, 3.0, 7.0 / 3.0]) {
            assert!((a - b).abs() < 1e-12, "{r:?}");
        }
    }

    #[test]
    fn histogram_matches_numpy_conventions() {
        let (c, e) = histogram(&[0.0, 0.5, 1.0, 1.0, 2.0, f64::NAN, 5.0], 2, Some((0.0, 2.0)));
        assert_eq!(c, vec![2.0, 3.0], "[0,1) holds 0 and 0.5; [1,2] holds 1, 1 and the upper edge 2");
        assert_eq!(e, vec![0.0, 1.0, 2.0]);
        let (c2, _) = histogram(&[3.0, 3.0, 3.0], 4, None);
        assert_eq!(c2.iter().sum::<f64>(), 3.0, "a constant signal still bins (numpy widens the range)");
    }

    #[test]
    fn savgol_reproduces_polynomials_exactly_including_the_edges() {
        let x: Vec<f64> = (0..40).map(|i| 0.5 + 0.3 * i as f64 - 0.02 * (i * i) as f64).collect();
        let s = savgol_filter(&x, 9, 2).unwrap();
        for (a, b) in s.iter().zip(&x) {
            assert!((a - b).abs() < 1e-9, "a quadratic passes through an order-2 filter unchanged");
        }
        // smoothing reduces noise
        let noisy: Vec<f64> = (0..200).map(|i| (i as f64 / 20.0).sin() + if i % 2 == 0 { 0.2 } else { -0.2 }).collect();
        let sm = savgol_filter(&noisy, 11, 2).unwrap();
        let rough = |v: &[f64]| v.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f64>();
        assert!(rough(&sm) < 0.3 * rough(&noisy));
        assert!(savgol_filter(&x, 8, 2).is_err() && savgol_filter(&x, 5, 5).is_err() && savgol_filter(&x[..4], 5, 2).is_err());
    }
}
