//! Group comparison and kinetics: ports of the engine's `run_stats_comparison` and `run_kinetic`.
//!
//! The test choice and every p-value follow what the engine's SciPy calls produce (SciPy 1.18, defaults):
//!
//! * 2 groups, unpaired: Mann-Whitney U, two-sided. `method="auto"`: exact when either sample has 8 or fewer
//!   values and there are no ties, otherwise the normal approximation with tie and continuity corrections.
//! * 2 groups, paired: Wilcoxon signed-rank, two-sided, zero differences dropped. `method="auto"`: more than 50
//!   pairs: normal approximation (no continuity correction, as in SciPy's default); otherwise exact when there are no ties or zeros, a deterministic sign-flip
//!   permutation test when there are and at most 13 pairs, else the normal approximation.
//! * 3+ groups: Kruskal-Wallis (tie-corrected chi-square), then Dunn's test with Bonferroni correction when the
//!   omnibus p < 0.05.
//!
//! Non-finite values are excluded and reported, never silently treated as "not significant".

use crate::fitting::erfc;
use crate::stats;

// ---- distributions ---------------------------------------------------------------------------------------

/// Survival function of the standard normal.
pub fn normal_sf(z: f64) -> f64 {
    0.5 * erfc(z / std::f64::consts::SQRT_2)
}

/// `ln Gamma(x)` for `x > 0` (Lanczos, g = 7).
pub fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [0.999_999_999_999_809_9, 676.520_368_121_885_1, -1_259.139_216_722_402_8, 771.323_428_777_653_1, -176.615_029_162_140_6, 12.507_343_278_686_905, -0.138_571_095_265_720_12, 9.984_369_578_019_572e-6, 1.505_632_735_149_311_6e-7];
    if x < 0.5 {
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + 7.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularised upper incomplete gamma `Q(a, x)`.
fn gammq(a: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }
    let ln_pref = -x + a * x.ln() - ln_gamma(a);
    if x < a + 1.0 {
        let (mut ap, mut sum) = (a, 1.0 / a);
        let mut term = sum;
        for _ in 0..10_000 {
            ap += 1.0;
            term *= x / ap;
            sum += term;
            if term.abs() < sum.abs() * 1e-17 {
                break;
            }
        }
        1.0 - sum * ln_pref.exp()
    } else {
        let tiny = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / tiny;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..10_000 {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < tiny {
                d = tiny;
            }
            c = b + an / c;
            if c.abs() < tiny {
                c = tiny;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < 1e-16 {
                break;
            }
        }
        ln_pref.exp() * h
    }
}

/// Survival function of the chi-square distribution with `df` degrees of freedom.
pub fn chi2_sf(x: f64, df: f64) -> f64 {
    gammq(df / 2.0, x / 2.0)
}

// ---- ranks -------------------------------------------------------------------------------------------------

/// Average ranks (1-based) and the size of every tie group (singletons included).
pub fn rank_average(v: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut ranks = vec![0.0; v.len()];
    let mut ties = Vec::new();
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && v[idx[j + 1]] == v[idx[i]] {
            j += 1;
        }
        let r = (i + j) as f64 / 2.0 + 1.0;
        for &k in &idx[i..=j] {
            ranks[k] = r;
        }
        ties.push((j - i + 1) as f64);
        i = j + 1;
    }
    (ranks, ties)
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestResult {
    pub statistic: f64,
    pub p: f64,
    /// Which p-value computation was used: "exact", "permutation" or "asymptotic".
    pub method: &'static str,
}

fn all_finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}

pub fn stars(p: f64) -> &'static str {
    if p < 0.001 {
        "***"
    } else if p < 0.01 {
        "**"
    } else if p < 0.05 {
        "*"
    } else {
        "ns"
    }
}

// ---- Mann-Whitney U ------------------------------------------------------------------------------------------

/// Counts of the Mann-Whitney U statistic over all arrangements of samples of size `m <= n`:
/// the coefficients of the Gaussian binomial `prod_{i=1..m} (1 - q^(n+i)) / (1 - q^i)`.
fn mwu_counts(m: usize, n: usize) -> Vec<f64> {
    let deg = m * n;
    let mut p = vec![0.0; deg + 1];
    p[0] = 1.0;
    for i in 1..=m {
        let s = n + i;
        for k in (s..=deg).rev() {
            p[k] -= p[k - s];
        }
        for k in i..=deg {
            p[k] += p[k - i];
        }
    }
    p
}

/// Two-sided Mann-Whitney U test (`scipy.stats.mannwhitneyu(x, y, alternative="two-sided")`, method auto).
pub fn mann_whitney_u(x: &[f64], y: &[f64]) -> Result<TestResult, String> {
    if x.is_empty() || y.is_empty() {
        return Err("Mann-Whitney needs at least one value in each group.".into());
    }
    if !all_finite(x) || !all_finite(y) {
        return Err("Mann-Whitney received non-finite values.".into());
    }
    let (n1, n2) = (x.len() as f64, y.len() as f64);
    let xy: Vec<f64> = x.iter().chain(y).copied().collect();
    let (ranks, ties) = rank_average(&xy);
    let r1: f64 = ranks[..x.len()].iter().sum();
    let u1 = r1 - n1 * (n1 + 1.0) / 2.0;
    let u2 = n1 * n2 - u1;
    let has_ties = ties.iter().any(|&t| t > 1.0);
    let exact = !(x.len() > 8 && y.len() > 8) && !has_ties;
    if exact {
        let (m, n) = (x.len().min(y.len()), x.len().max(y.len()));
        let counts = mwu_counts(m, n);
        let total: f64 = counts.iter().sum();
        let u = u1.max(u2).round() as usize;
        let sf: f64 = counts[u..].iter().sum::<f64>() / total; // P(U >= u), inclusive
        return Ok(TestResult { statistic: u1, p: (2.0 * sf).min(1.0), method: "exact" });
    }
    let n = n1 + n2;
    let tie_term: f64 = ties.iter().map(|t| t * t * t - t).sum();
    let s = (n1 * n2 / 12.0 * ((n + 1.0) - tie_term / (n * (n - 1.0)))).sqrt();
    if !(s > 0.0) {
        return Err("All values are identical, so Mann-Whitney is undefined.".into());
    }
    let mut num = u1 - n1 * n2 / 2.0;
    let sign = if num > 0.0 { 1.0 } else if num < 0.0 { -1.0 } else { 0.0 };
    num -= 0.5 * sign; // continuity correction, always towards the null
    let z = num / s;
    Ok(TestResult { statistic: u1, p: (2.0 * normal_sf(z.abs())).min(1.0), method: "asymptotic" })
}

// ---- Wilcoxon signed-rank -------------------------------------------------------------------------------------

/// Number of ways each signed-rank sum `0..=n(n+1)/2` arises from the subsets of `{1..n}`.
fn wilcoxon_counts(n: usize) -> Vec<f64> {
    let max = n * (n + 1) / 2;
    let mut c = vec![0.0; max + 1];
    c[0] = 1.0;
    for r in 1..=n {
        for s in (r..=max).rev() {
            c[s] += c[s - r];
        }
    }
    c
}

/// Two-sided Wilcoxon signed-rank test on the differences `d` (`scipy.stats.wilcoxon`, zero_method "wilcox").
pub fn wilcoxon_signed_rank(d: &[f64]) -> Result<TestResult, String> {
    if !all_finite(d) {
        return Err("Wilcoxon received non-finite differences.".into());
    }
    let n_total = d.len();
    let nz: Vec<f64> = d.iter().copied().filter(|v| *v != 0.0).collect();
    let n_zero = n_total - nz.len();
    let count = nz.len();
    if count == 0 {
        return Err("All paired differences are zero, so Wilcoxon is undefined.".into());
    }
    let abs: Vec<f64> = nz.iter().map(|v| v.abs()).collect();
    let (ranks, ties) = rank_average(&abs);
    let r_plus: f64 = nz.iter().zip(&ranks).filter(|(v, _)| **v > 0.0).map(|(_, r)| r).sum();
    let r_minus: f64 = nz.iter().zip(&ranks).filter(|(v, _)| **v < 0.0).map(|(_, r)| r).sum();
    let statistic = r_plus.min(r_minus);
    let has_ties = ties.iter().any(|&t| t > 1.0);
    let nf = count as f64;

    let asymptotic = || -> Result<TestResult, String> {
        let mn = nf * (nf + 1.0) * 0.25;
        let tie_correct: f64 = ties.iter().map(|t| t * t * t - t).sum();
        let se = ((nf * (nf + 1.0) * (2.0 * nf + 1.0) - tie_correct / 2.0) / 24.0).sqrt();
        if !(se > 0.0) {
            return Err("The differences carry no spread, so the Wilcoxon normal approximation is undefined.".into());
        }
        let z = (r_plus - mn) / se; // scipy.stats.wilcoxon defaults to correction=False: no continuity correction
        Ok(TestResult { statistic, p: (2.0 * normal_sf(z.abs())).min(1.0), method: "asymptotic" })
    };

    if n_total > 50 {
        return asymptotic();
    }
    if !has_ties && n_zero == 0 {
        let pmf = wilcoxon_counts(count);
        let total = 2f64.powi(count as i32);
        let hi = pmf.len() - 1;
        let lo_k = (r_plus.ceil() as usize).min(hi); // P(R <= ceil r+)
        let hi_k = (r_plus.floor() as usize).min(hi); // P(R >= floor r+)
        let cdf: f64 = pmf[..=lo_k].iter().sum::<f64>() / total;
        let sf: f64 = pmf[hi_k..].iter().sum::<f64>() / total;
        return Ok(TestResult { statistic, p: (2.0 * cdf.min(sf)).clamp(0.0, 1.0), method: "exact" });
    }
    if n_total <= 13 {
        // all 2^n sign assignments; zeros keep zero rank weight, ranks of the non-zero |d| stay fixed
        let mut rank_of = vec![0.0; n_total];
        let mut k = 0;
        for (i, v) in d.iter().enumerate() {
            if *v != 0.0 {
                rank_of[i] = ranks[k];
                k += 1;
            }
        }
        let total = 1usize << n_total;
        let positive: Vec<bool> = d.iter().map(|v| *v > 0.0).collect();
        let null: Vec<f64> = (0..total).map(|mask| (0..n_total).filter(|&i| d[i] != 0.0 && (positive[i] ^ (mask >> i & 1 == 1))).map(|i| rank_of[i]).sum()).collect();
        let gamma = (f64::EPSILON * 100.0 * r_plus).abs();
        let less = null.iter().filter(|&&v| v <= r_plus + gamma).count() as f64 / total as f64;
        let greater = null.iter().filter(|&&v| v >= r_plus - gamma).count() as f64 / total as f64;
        return Ok(TestResult { statistic, p: (2.0 * less.min(greater)).clamp(0.0, 1.0), method: "permutation" });
    }
    asymptotic()
}

// ---- Kruskal-Wallis and Dunn -----------------------------------------------------------------------------------

/// Kruskal-Wallis H test with tie correction (`scipy.stats.kruskal`).
pub fn kruskal_wallis(groups: &[&[f64]]) -> Result<TestResult, String> {
    if groups.len() < 2 {
        return Err("Kruskal-Wallis needs at least two groups.".into());
    }
    if groups.iter().any(|g| g.is_empty() || !all_finite(g)) {
        return Err("Kruskal-Wallis received an empty group or non-finite values.".into());
    }
    let all: Vec<f64> = groups.iter().flat_map(|g| g.iter().copied()).collect();
    let n = all.len() as f64;
    let (ranks, ties) = rank_average(&all);
    let mut h = 0.0;
    let mut off = 0;
    for g in groups {
        let r: f64 = ranks[off..off + g.len()].iter().sum();
        h += r * r / g.len() as f64;
        off += g.len();
    }
    h = 12.0 / (n * (n + 1.0)) * h - 3.0 * (n + 1.0);
    let corr = 1.0 - ties.iter().map(|t| t * t * t - t).sum::<f64>() / (n * n * n - n);
    if corr == 0.0 {
        return Err("All values are identical, so Kruskal-Wallis is undefined.".into());
    }
    h /= corr;
    Ok(TestResult { statistic: h, p: chi2_sf(h, groups.len() as f64 - 1.0), method: "asymptotic" })
}

#[derive(Debug, Clone, PartialEq)]
pub struct PostHoc {
    pub pair: String,
    pub p: f64,
    pub stars: &'static str,
}

/// Dunn's test with Bonferroni correction (the engine's own implementation: joint average ranks, tie-corrected
/// variance, two-sided normal p, multiplied by the number of pairs and capped at 1).
pub fn dunn_bonferroni(groups: &[(String, Vec<f64>)]) -> Vec<PostHoc> {
    let all: Vec<f64> = groups.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    let n_total = all.len() as f64;
    let (ranks, ties) = rank_average(&all);
    let tie_term: f64 = ties.iter().map(|t| t * t * t - t).sum();
    let mut mean_ranks = Vec::new();
    let mut off = 0;
    for (_, v) in groups {
        mean_ranks.push(ranks[off..off + v.len()].iter().sum::<f64>() / v.len() as f64);
        off += v.len();
    }
    let k = groups.len();
    let n_pairs = (k * (k - 1) / 2) as f64;
    let mut out = Vec::new();
    for a in 0..k {
        for b in a + 1..k {
            let (na, nb) = (groups[a].1.len() as f64, groups[b].1.len() as f64);
            let sigma = ((n_total * (n_total + 1.0) / 12.0 - tie_term / (12.0 * (n_total - 1.0))) * (1.0 / na + 1.0 / nb)).sqrt();
            let p = if sigma == 0.0 { 1.0 } else { 2.0 * normal_sf((mean_ranks[a] - mean_ranks[b]).abs() / sigma) };
            let p_adj = (p * n_pairs).min(1.0);
            out.push(PostHoc { pair: format!("{} vs {}", groups[a].0, groups[b].0), p: p_adj, stars: stars(p_adj) });
        }
    }
    out
}

// ---- group comparison (engine: run_stats_comparison) ------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GroupSummary {
    pub name: String,
    pub n: usize,
    pub median: f64,
    pub values: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub test: String,
    pub method: &'static str,
    pub statistic: f64,
    pub p: f64,
    pub stars: &'static str,
    pub posthoc: Vec<PostHoc>,
    pub groups: Vec<GroupSummary>,
    /// `(group, how many non-finite values were left out)`.
    pub dropped: Vec<(String, usize)>,
}

/// Compare per-sample values between groups, choosing the test as the engine does. Non-finite values are removed
/// (for a paired comparison a pair is removed as a whole so the pairing is never shifted) and reported in `dropped`.
pub fn compare_groups(raw: &[(String, Vec<f64>)], paired: bool) -> Result<Comparison, String> {
    let mut dropped = Vec::new();
    let mut groups: Vec<(String, Vec<f64>)> = Vec::new();
    if paired {
        if raw.len() != 2 {
            return Err(format!("A paired comparison needs exactly two groups (got {}).", raw.len()));
        }
        let (a, b) = (&raw[0].1, &raw[1].1);
        if a.len() != b.len() {
            return Err(format!("Paired test requires equal group sizes ({} vs {}).", a.len(), b.len()));
        }
        let keep: Vec<usize> = (0..a.len()).filter(|&i| a[i].is_finite() && b[i].is_finite()).collect();
        if keep.len() < a.len() {
            dropped.push((raw[0].0.clone(), a.len() - keep.len()));
            dropped.push((raw[1].0.clone(), a.len() - keep.len()));
        }
        if keep.len() < 2 {
            return Err("Need at least 2 paired values with both sides finite.".into());
        }
        groups.push((raw[0].0.clone(), keep.iter().map(|&i| a[i]).collect()));
        groups.push((raw[1].0.clone(), keep.iter().map(|&i| b[i]).collect()));
    } else {
        for (name, vals) in raw {
            let fin: Vec<f64> = vals.iter().copied().filter(|v| v.is_finite()).collect();
            if fin.len() < vals.len() {
                dropped.push((name.clone(), vals.len() - fin.len()));
            }
            if fin.len() >= 2 {
                groups.push((name.clone(), fin));
            }
        }
        if groups.len() < 2 {
            return Err("Need at least 2 groups with at least 2 finite values each. Assign samples to groups and make sure their values are computed.".into());
        }
    }
    let (test, result, posthoc) = if groups.len() == 2 {
        if paired {
            let d: Vec<f64> = groups[0].1.iter().zip(&groups[1].1).map(|(a, b)| a - b).collect();
            ("Wilcoxon signed-rank (paired)".to_string(), wilcoxon_signed_rank(&d)?, vec![])
        } else {
            ("Mann-Whitney U".to_string(), mann_whitney_u(&groups[0].1, &groups[1].1)?, vec![])
        }
    } else {
        let refs: Vec<&[f64]> = groups.iter().map(|(_, v)| v.as_slice()).collect();
        let r = kruskal_wallis(&refs)?;
        let ph = if r.p < 0.05 { dunn_bonferroni(&groups) } else { vec![] };
        ("Kruskal-Wallis + Dunn (Bonferroni)".to_string(), r, ph)
    };
    let summaries = groups.iter().map(|(name, v)| GroupSummary { name: name.clone(), n: v.len(), median: stats::percentile(v, 50.0), values: v.clone() }).collect();
    Ok(Comparison { test, method: result.method, statistic: result.statistic, p: result.p, stars: stars(result.p), posthoc, groups: summaries, dropped })
}

// ---- kinetics (engine: run_kinetic) --------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct KineticInput {
    pub sample: String,
    pub group: String,
    pub time: Vec<f64>,
    pub value: Vec<f64>,
}

#[derive(Debug, Clone)]
pub struct KineticSeries {
    pub group: String,
    pub n_samples: usize,
    pub times: Vec<f64>,
    /// Mean over the group's samples of the per-bin median; NaN where no sample has events in the bin.
    pub mfi: Vec<f64>,
    /// Population standard deviation (ddof 0) over the group's samples.
    pub sd: Vec<f64>,
}

/// Bin events by time on a shared grid (`bins` equal bins from the earliest to the latest event of any sample),
/// take the median of each sample's bin, then average the samples of each group. Events whose time or value is
/// not finite are left out. A bin without events is NaN, not zero.
pub fn kinetics(inputs: &[KineticInput], bins: usize) -> Result<Vec<KineticSeries>, String> {
    if bins == 0 {
        return Err("bins must be at least 1.".into());
    }
    let (mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in inputs {
        for (t, v) in s.time.iter().zip(&s.value) {
            if t.is_finite() && v.is_finite() {
                t0 = t0.min(*t);
                t1 = t1.max(*t);
            }
        }
    }
    if !(t0.is_finite() && t1.is_finite()) {
        return Err("No events with a finite time and value.".into());
    }
    let edges: Vec<f64> = (0..=bins).map(|i| t0 + (t1 - t0) * i as f64 / bins as f64).collect();
    let mids: Vec<f64> = (0..bins).map(|i| (edges[i] + edges[i + 1]) / 2.0).collect();
    let mut order: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<Vec<f64>>> = Vec::new();
    for s in inputs {
        let mut per_bin: Vec<Vec<f64>> = vec![Vec::new(); bins];
        for (t, v) in s.time.iter().zip(&s.value) {
            if !(t.is_finite() && v.is_finite()) {
                continue;
            }
            // numpy.digitize(t, edges) - 1, clipped: the last edge belongs to the last bin
            let b = edges.partition_point(|e| *e <= *t).saturating_sub(1).min(bins - 1);
            per_bin[b].push(*v);
        }
        let row: Vec<f64> = per_bin.iter().map(|v| if v.is_empty() { f64::NAN } else { stats::percentile(v, 50.0) }).collect();
        let gi = match order.iter().position(|g| *g == s.group) {
            Some(i) => i,
            None => {
                order.push(s.group.clone());
                rows.push(Vec::new());
                order.len() - 1
            }
        };
        rows[gi].push(row);
    }
    Ok(order
        .into_iter()
        .zip(rows)
        .map(|(group, samples)| {
            let (mut mfi, mut sd) = (vec![f64::NAN; bins], vec![f64::NAN; bins]);
            for b in 0..bins {
                let col: Vec<f64> = samples.iter().map(|r| r[b]).filter(|v| !v.is_nan()).collect();
                if !col.is_empty() {
                    let m = col.iter().sum::<f64>() / col.len() as f64;
                    mfi[b] = m;
                    sd[b] = (col.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / col.len() as f64).sqrt();
                }
            }
            KineticSeries { group, n_samples: samples.len(), times: mids.clone(), mfi, sd }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol * b.abs().max(1e-300) + 1e-300
    }

    #[test]
    fn distribution_functions_match_known_values() {
        // values from scipy.stats.norm.sf / chi2.sf / math.lgamma
        assert!(close(normal_sf(1.96), 0.024997895148220435, 1e-12));
        assert!(close(normal_sf(0.0), 0.5, 1e-15));
        assert!(close(normal_sf(8.0), 6.220960574271785e-16, 1e-9), "{}", normal_sf(8.0));
        assert!(close(chi2_sf(3.84, 1.0), 0.05004352124173, 1e-9), "{}", chi2_sf(3.84, 1.0));
        assert!(close(chi2_sf(10.0, 4.0), 0.04042768199451294, 1e-10));
        assert!(close(chi2_sf(1.0, 2.0), 0.6065306597126334, 1e-12));
        assert!(close(ln_gamma(5.0), 24f64.ln(), 1e-13) && close(ln_gamma(0.5), 0.5723649429247001, 1e-13));
    }

    #[test]
    fn ranks_average_ties() {
        let (r, t) = rank_average(&[3.0, 1.0, 3.0, 2.0]);
        assert_eq!(r, vec![3.5, 1.0, 3.5, 2.0]);
        assert_eq!(t, vec![1.0, 1.0, 2.0]);
    }

    #[test]
    fn mann_whitney_small_exact_hand_example() {
        // completely separated samples of 3 and 3: U = 0, exact p = 2 / C(6,3) = 0.1
        let r = mann_whitney_u(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]).unwrap();
        assert_eq!(r.method, "exact");
        assert!(close(r.p, 0.1, 1e-12) && r.statistic == 0.0, "{r:?}");
        // identical groups: the exact p is capped at 1
        let r = mann_whitney_u(&[1.0, 3.0, 5.0], &[2.0, 4.0, 6.0]).unwrap();
        assert!(r.p <= 1.0 && r.p > 0.5);
    }

    #[test]
    fn wilcoxon_small_exact_hand_example() {
        // 5 positive differences of increasing size: r+ = 15, two-sided exact p = 2 / 32
        let r = wilcoxon_signed_rank(&[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap();
        assert_eq!(r.method, "exact");
        assert!(close(r.p, 0.0625, 1e-12) && r.statistic == 0.0, "{r:?}");
        assert!(wilcoxon_signed_rank(&[0.0, 0.0]).is_err());
        assert!(wilcoxon_signed_rank(&[1.0, f64::NAN]).is_err());
    }

    #[test]
    fn a_bad_group_is_reported_not_called_not_significant() {
        let raw = vec![("A".to_string(), vec![1.0, 2.0, 3.0, f64::NAN]), ("B".to_string(), vec![7.0, 8.0, 9.0, 10.0]), ("C".to_string(), vec![f64::NAN])];
        let c = compare_groups(&raw, false).unwrap();
        assert_eq!(c.test, "Mann-Whitney U");
        assert_eq!(c.groups.iter().map(|g| g.n).collect::<Vec<_>>(), vec![3, 4]);
        assert_eq!(c.dropped, vec![("A".to_string(), 1), ("C".to_string(), 1)]);
        // paired: the pair with a NaN on either side goes as a pair
        let paired = vec![("pre".to_string(), vec![1.0, 2.0, f64::NAN, 4.0, 5.0]), ("post".to_string(), vec![2.0, 4.0, 6.0, f64::NAN, 9.0])];
        let c = compare_groups(&paired, true).unwrap();
        assert_eq!(c.groups[0].values, vec![1.0, 2.0, 5.0]);
        assert_eq!(c.groups[1].values, vec![2.0, 4.0, 9.0]);
        assert!(compare_groups(&[("A".to_string(), vec![1.0, 2.0]), ("B".to_string(), vec![1.0])], true).unwrap_err().contains("equal group sizes"));
        assert!(compare_groups(&[("A".to_string(), vec![1.0])], false).is_err());
    }

    #[test]
    fn kinetics_bins_by_time_and_averages_samples_per_group() {
        // time 0..=9, value = 10 * time; one sample per group; 5 bins of width 1.8
        let s = |name: &str, group: &str, scale: f64| KineticInput { sample: name.into(), group: group.into(), time: (0..10).map(|i| i as f64).collect(), value: (0..10).map(|i| i as f64 * scale).collect() };
        let out = kinetics(&[s("a", "G", 10.0), s("b", "G", 20.0), s("c", "H", 1.0)], 5).unwrap();
        assert_eq!(out.len(), 2);
        let g = &out[0];
        assert_eq!((g.group.as_str(), g.n_samples), ("G", 2));
        // bins [0,1.8) = {0,1}, [1.8,3.6) = {2,3}, [3.6,5.4) = {4,5}, [5.4,7.2) = {6,7}, [7.2,9] = {8,9}
        // sample a medians 5, 25, 45, 65, 85; sample b 10, 50, 90, 130, 170 -> means 7.5, 37.5, ...
        for (got, want) in g.mfi.iter().zip([7.5, 37.5, 67.5, 97.5, 127.5]) {
            assert!(close(*got, want, 1e-12), "{:?}", g.mfi);
        }
        for (got, want) in g.sd.iter().zip([2.5, 12.5, 22.5, 32.5, 42.5]) {
            assert!(close(*got, want, 1e-12), "population SD of two values is half their difference: {:?}", g.sd);
        }
        assert!(close(g.times[0], 0.9, 1e-12) && close(g.times[4], 8.1, 1e-12));
        // the last event sits exactly on the last edge and lands in the last bin
        assert!(close(out[1].mfi[4], 8.5, 1e-12));
    }

    #[test]
    fn kinetics_empty_bins_are_nan_and_bad_events_are_left_out() {
        let s = KineticInput { sample: "a".into(), group: "g".into(), time: vec![0.0, 0.1, 9.9, 10.0, f64::NAN, 5.0], value: vec![1.0, 3.0, 5.0, 7.0, 100.0, f64::INFINITY] };
        let out = kinetics(&[s], 5).unwrap();
        assert!(close(out[0].mfi[0], 2.0, 1e-12) && close(out[0].mfi[4], 6.0, 1e-12));
        assert!(out[0].mfi[1].is_nan() && out[0].mfi[2].is_nan() && out[0].sd[3].is_nan(), "no events means NaN, never zero");
        assert!(kinetics(&[], 5).is_err());
    }
}
