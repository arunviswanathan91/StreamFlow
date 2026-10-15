//! The single home of every statistic (port of the Java `Stats`; same settled conventions).
//!
//! * Standard deviation is the **population** SD (divide by n).
//! * Percentiles are **type-7** (linear interpolation between order statistics, h = (n-1)p/100).
//! * The geometric mean drops `x <= 0` and **reports how many**, so a caller can never present a
//!   gMean of 40% of a population as if it were the whole.
//! * Mode is the peak of the smoothed distribution, not the most frequent raw value.
//! * Empty input yields 0 for every statistic (matches the Java contract).

/// Percentiles defining the robust statistics: +/-1 SD of a Gaussian.
pub const P_LOW: f64 = 15.87;
pub const P_HIGH: f64 = 84.13;

const MODE_BINS: usize = 256;
const MODE_SMOOTH_PASSES: usize = 3;
const MODE_SMOOTH_WIDTH: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoMean {
    pub value: f64,
    pub excluded: usize,
    pub total: usize,
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

pub fn median(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        median_sorted(&sorted(v))
    }
}

/// Population standard deviation.
pub fn sd(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt()
}

/// CV = 100 * SD / mean; 0 when the mean is 0.
pub fn cv(v: &[f64]) -> f64 {
    let m = mean(v);
    if m == 0.0 {
        0.0
    } else {
        100.0 * sd(v) / m
    }
}

/// rSD = (P84.13 - P15.87) / 2.
pub fn rsd(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let s = sorted(v);
    (percentile_sorted(&s, P_HIGH) - percentile_sorted(&s, P_LOW)) / 2.0
}

/// rCV = 100 * rSD / **median** (the denominator is the median, not the mean).
pub fn rcv(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let s = sorted(v);
    let med = median_sorted(&s);
    if med == 0.0 {
        return 0.0;
    }
    100.0 * ((percentile_sorted(&s, P_HIGH) - percentile_sorted(&s, P_LOW)) / 2.0) / med
}

/// Median absolute deviation, unscaled.
pub fn mad(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let med = median(v);
    let dev: Vec<f64> = v.iter().map(|x| (x - med).abs()).collect();
    median(&dev)
}

pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        percentile_sorted(&sorted(v), p)
    }
}

pub fn min(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().cloned().fold(f64::MAX, f64::min)
    }
}

pub fn max(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().cloned().fold(f64::MIN, f64::max)
    }
}

/// Geometric mean over strictly positive events, reporting how many were excluded.
pub fn geo_mean(v: &[f64]) -> GeoMean {
    let (mut sum, mut n) = (0.0, 0usize);
    for &x in v {
        if x > 0.0 {
            sum += x.ln();
            n += 1;
        }
    }
    GeoMean { value: if n == 0 { 0.0 } else { (sum / n as f64).exp() }, excluded: v.len() - n, total: v.len() }
}

/// Pearson correlation; 0 if either input is constant or empty.
pub fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let (ma, mb) = (a[..n].iter().sum::<f64>() / n as f64, b[..n].iter().sum::<f64>() / n as f64);
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (da, db) = (a[i] - ma, b[i] - mb);
        sab += da * db;
        saa += da * da;
        sbb += db * db;
    }
    let d = (saa * sbb).sqrt();
    if d == 0.0 {
        0.0
    } else {
        sab / d
    }
}

/// Otsu threshold over a 256-bin histogram; a constant input returns that value.
pub fn otsu(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let (lo, hi) = (min(v), max(v));
    if hi <= lo {
        return lo;
    }
    let bins = 256usize;
    let mut hist = vec![0u64; bins];
    let scale = bins as f64 / (hi - lo);
    for &x in v {
        hist[(((x - lo) * scale) as usize).min(bins - 1)] += 1;
    }
    let total = v.len() as f64;
    let sum: f64 = hist.iter().enumerate().map(|(b, &h)| b as f64 * h as f64).sum();
    let (mut sum_b, mut w_b, mut max_var, mut thresh) = (0.0, 0.0, -1.0, 0usize);
    for (b, &h) in hist.iter().enumerate() {
        w_b += h as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += b as f64 * h as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > max_var {
            max_var = between;
            thresh = b;
        }
    }
    lo + (thresh as f64 + 0.5) / scale
}

/// Peak of the smoothed distribution (256 bins, three box-blur passes of width 5).
pub fn mode(v: &[f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let (lo, hi) = (min(v), max(v));
    if hi <= lo {
        return lo;
    }
    let mut h = vec![0.0f64; MODE_BINS];
    let scale = MODE_BINS as f64 / (hi - lo);
    for &x in v {
        h[(((x - lo) * scale) as usize).min(MODE_BINS - 1)] += 1.0;
    }
    for _ in 0..MODE_SMOOTH_PASSES {
        h = box_blur(&h, MODE_SMOOTH_WIDTH);
    }
    let mut peak = 0;
    for i in 1..h.len() {
        if h[i] > h[peak] {
            peak = i;
        }
    }
    lo + (peak as f64 + 0.5) / scale
}

fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s
}

fn median_sorted(s: &[f64]) -> f64 {
    let n = s.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

fn percentile_sorted(s: &[f64], p: f64) -> f64 {
    let n = s.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return s[0];
    }
    let h = (n - 1) as f64 * p.clamp(0.0, 100.0) / 100.0;
    let lo = h.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    s[lo] + (h - lo as f64) * (s[hi] - s[lo])
}

fn box_blur(h: &[f64], width: usize) -> Vec<f64> {
    let r = (width / 2) as isize;
    (0..h.len() as isize)
        .map(|i| {
            let (mut s, mut c) = (0.0, 0.0);
            for k in -r..=r {
                let j = i + k;
                if j >= 0 && (j as usize) < h.len() {
                    s += h[j as usize];
                    c += 1.0;
                }
            }
            s / c
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: [f64; 10] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
    const EPS: f64 = 1e-6;
    fn near(a: f64, b: f64, eps: f64) {
        assert!((a - b).abs() <= eps, "{a} != {b} (eps {eps})");
    }

    #[test]
    fn mean_and_median() {
        near(mean(&V), 5.5, EPS);
        near(median(&V), 5.5, EPS);
        near(median(&[2.0, 10.0, 6.0]), 6.0, EPS); // odd n
    }

    #[test]
    fn population_sd_and_cv() {
        near(sd(&V), 8.25f64.sqrt(), EPS);
        near(cv(&V), 100.0 * 8.25f64.sqrt() / 5.5, EPS);
        near(cv(&[-1.0, 1.0]), 0.0, EPS); // mean 0 -> undefined -> 0
    }

    #[test]
    fn type7_percentiles() {
        near(percentile(&V, 50.0), 5.5, EPS);
        near(percentile(&V, 0.0), 1.0, EPS);
        near(percentile(&V, 100.0), 10.0, EPS);
        near(percentile(&V, P_LOW), 2.4283, 1e-4);
        near(percentile(&V, P_HIGH), 8.5717, 1e-4);
    }

    #[test]
    fn robust_statistics_use_the_confirmed_formulas() {
        let expected = (8.5717 - 2.4283) / 2.0;
        near(rsd(&V), expected, 1e-4);
        near(rcv(&V), 100.0 * expected / 5.5, 1e-3); // denominator is the MEDIAN
    }

    #[test]
    fn median_absolute_deviation() {
        near(mad(&V), 2.5, EPS);
    }

    #[test]
    fn geometric_mean_reports_the_events_it_could_not_use() {
        let g = geo_mean(&[1.0, 4.0, -3.0]);
        near(g.value, 2.0, EPS); // sqrt(1*4) on 2 of 3
        assert_eq!((g.excluded, g.total), (1, 3));
        assert_eq!(geo_mean(&[-1.0, 0.0]).value, 0.0);
    }

    #[test]
    fn otsu_splits_two_clouds_and_handles_degenerate_input() {
        let mut v: Vec<f64> = (0..500).map(|i| 10.0 + (i % 20) as f64).collect();
        v.extend((0..500).map(|i| 90.0 + (i % 20) as f64));
        let t = otsu(&v);
        assert!(t > 29.0 && t < 90.0, "threshold {t}");
        assert_eq!(otsu(&[]), 0.0);
        assert_eq!(otsu(&[5.0, 5.0, 5.0]), 5.0);
    }

    #[test]
    fn correlation_and_mode() {
        let b: Vec<f64> = V.iter().map(|x| 2.0 * x + 1.0).collect();
        near(correlation(&V, &b), 1.0, 1e-12);
        near(correlation(&V, &[3.0; 10]), 0.0, EPS);
        let mut d: Vec<f64> = vec![50.0; 300];
        d.extend((0..100).map(|i| i as f64));
        assert!((mode(&d) - 50.0).abs() < 1.0, "mode {}", mode(&d));
    }

    #[test]
    fn empty_input_is_zero_everywhere() {
        let e: [f64; 0] = [];
        for x in [mean(&e), median(&e), sd(&e), cv(&e), rsd(&e), rcv(&e), mad(&e), percentile(&e, 50.0), min(&e), max(&e), mode(&e)] {
            assert_eq!(x, 0.0);
        }
    }
}
