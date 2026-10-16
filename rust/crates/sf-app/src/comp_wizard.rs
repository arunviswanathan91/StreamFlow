//! The compensation wizard's mathematics: from single-stain controls (and an optional unstained tube) to a spillover
//! matrix, with the checks a user needs to trust it.
//!
//! Orientation matches [`crate::comp_auto`]: row `i` is the spillover of control `i`'s fluorochrome into every
//! detector, and corrected = measured x inverse(S).
//!
//! What is done, in order, for every control:
//! 1. **Scatter clean-up.** The dominant FSC/SSC population is kept (debris and aggregates are dropped): a smoothed
//!    2-D density, its highest peak, and the connected region above a fraction of that peak.
//! 2. **Positive / negative split** on the stained detector, automatic (a two-component Gaussian mixture on the
//!    arcsinh scale, Otsu as the fallback) or set by hand per control. The negative can be the control's own negative
//!    events, an unstained tube, or a "true zero" background.
//! 3. **The coefficients**, by one of three methods ([`Method`]).
//! 4. **Quality**: a warning badge per control, the spillover-spreading matrix (how much noise each fluorochrome adds
//!    to every other detector once compensated), the cosine similarity of the signatures and the matrix condition.
//!
//! The methods are described in the literature (Bagwell & Adams 1993 for medians; Roca et al., Nat Commun 2021 for
//! the robust-regression-with-refinement idea; Nguyen et al. 2013 for spillover spreading). The code here is written
//! from those descriptions, not copied from any implementation. Nothing is random: every subsample is a fixed stride.

use crate::sample::{CompInfo, LoadedSample};
use sf_core::{linalg, stats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Median of the positive minus median of the negative, per detector, divided by the same on the primary.
    Medians,
    /// Robust slope of each detector against the primary, through bin medians; no negative population needed.
    Slope,
    /// Robust (Huber) regression of each detector on the primary over the clean events, then the matrix is refined
    /// until the compensated controls show no remaining slope. No positive / negative split is needed.
    Refined,
}

impl Method {
    pub const ALL: [Method; 3] = [Method::Medians, Method::Slope, Method::Refined];
    pub fn label(self) -> &'static str {
        match self {
            Method::Medians => "Population medians (classic)",
            Method::Slope => "Binned robust slope",
            Method::Refined => "Robust regression + refinement (AutoSpill-style)",
        }
    }
    pub fn needs_split(self) -> bool {
        self == Method::Medians
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitMethod {
    /// Mixture model, Otsu if the mixture does not separate.
    Auto,
    Otsu,
    Gmm,
}

impl SplitMethod {
    pub const ALL: [SplitMethod; 3] = [SplitMethod::Auto, SplitMethod::Gmm, SplitMethod::Otsu];
    pub fn label(self) -> &'static str {
        match self {
            SplitMethod::Auto => "Automatic",
            SplitMethod::Gmm => "Two-component mixture",
            SplitMethod::Otsu => "Otsu threshold",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CleanMode {
    Off,
    /// Keep the dominant scatter population: cells of the smoothed density above this fraction of its peak that
    /// connect to the peak.
    Dominant(f64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegSource {
    /// Each control's own negative events.
    Own,
    /// One unstained sample (a sample index) is the negative for every control.
    Unstained(usize),
    /// A fictitious all-zero background.
    TrueZero,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlSetup {
    pub sample: usize,
    pub detector: String,
    /// `(negative up to, positive from)` in raw units, when set by hand; `None` = automatic.
    pub manual: Option<(f64, f64)>,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub method: Method,
    pub split: SplitMethod,
    pub clean: CleanMode,
    pub negative: NegSource,
}

impl Default for Options {
    fn default() -> Self {
        Options { method: Method::Refined, split: SplitMethod::Auto, clean: CleanMode::Dominant(0.12), negative: NegSource::Own }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    MissingNegative,
    MissingPositive,
    TooFewEvents,
    PositiveTooDim,
    NoScatterCleanup,
}

impl Badge {
    pub fn label(self) -> &'static str {
        match self {
            Badge::MissingNegative => "Missing negative population",
            Badge::MissingPositive => "Missing positive population",
            Badge::TooFewEvents => "Too few events after clean-up",
            Badge::PositiveTooDim => "Positive is barely brighter than the negative: check the split",
            Badge::NoScatterCleanup => "No scatter clean-up: debris can bias the slope",
        }
    }
    /// A badge that makes the control unusable for the chosen method (others are advice).
    pub fn blocks(self) -> bool {
        matches!(self, Badge::MissingPositive | Badge::TooFewEvents)
    }
}

#[derive(Debug, Clone)]
pub struct ControlReport {
    pub sample: usize,
    pub detector: String,
    pub n_events: usize,
    pub n_clean: usize,
    pub n_pos: usize,
    pub n_neg: usize,
    /// Raw-unit limits actually used: negative up to, positive from.
    pub neg_max: f64,
    pub pos_min: f64,
    pub median_pos: f64,
    pub median_neg: f64,
    /// (median positive - median negative) / (2 x robust SD of the negative).
    pub stain_index: f64,
    pub badges: Vec<Badge>,
}

#[derive(Debug, Clone)]
pub struct WizardResult {
    pub matrix: CompInfo,
    pub reports: Vec<ControlReport>,
    /// `d x d`: row = the stained control's detector, column = the detector receiving noise; 0 where unmeasured.
    pub spread: Vec<f64>,
    /// `d x d` cosine similarity of the spillover rows (1 = indistinguishable signatures).
    pub cosine: Vec<f64>,
    /// Largest over smallest singular value of the matrix; large = unstable compensation.
    pub condition: f64,
    /// How many refinement rounds ran (0 for the other methods) and the largest slope left on a compensated control.
    pub rounds: usize,
    pub residual_slope: f64,
    pub notes: Vec<String>,
}

const COFACTOR: f64 = 150.0;
/// Events per control used for the regression and refinement (a fixed stride keeps the result reproducible).
const MAX_EVENTS: usize = 20_000;
const REFINE_EVENTS: usize = 4_000;

fn column(s: &LoadedSample, c: usize) -> Vec<f64> {
    let d = s.channels.len();
    (0..s.n_events).map(|r| s.events[r * d + c] as f64).collect()
}

fn scatter_channel(s: &LoadedSample, prefix: &str) -> Option<usize> {
    let up = |c: &String| c.to_ascii_uppercase();
    s.channels.iter().position(|c| up(c) == format!("{prefix}-A")).or_else(|| s.channels.iter().position(|c| up(c).starts_with(prefix) && up(c).ends_with("-A")))
}

/// Which events of a sample are kept by the scatter clean-up.
pub fn clean_mask(s: &LoadedSample, mode: CleanMode) -> Vec<bool> {
    clean_mask_for(s, mode, None)
}

/// The clean-up for a single-stain control whose stained detector is column `primary`. The population of interest
/// in a control is the one that is stained, so when the brightest 5 percent of events do not mostly fall inside the
/// dominant scatter region (stained cells or beads often sit apart from the debris-rich bulk), the region is built
/// around where those bright events are instead.
pub fn clean_mask_for(s: &LoadedSample, mode: CleanMode, primary: Option<usize>) -> Vec<bool> {
    let all = vec![true; s.n_events];
    let CleanMode::Dominant(frac) = mode else { return all };
    let (Some(fi), Some(si)) = (scatter_channel(s, "FSC"), scatter_channel(s, "SSC")) else { return all };
    let (x, y) = (column(s, fi), column(s, si));
    let mut mask = dominant_region(&x, &y, frac, None);
    if let Some(pc) = primary {
        let v = column(s, pc);
        let (cut, med) = (stats::percentile(&v, 95.0), stats::percentile(&v, 50.0));
        let bright: Vec<bool> = v.iter().map(|&q| q >= cut && q > med).collect();
        let nb = bright.iter().filter(|&&b| b).count();
        if nb >= 100 {
            let inside = (0..s.n_events).filter(|&i| bright[i] && mask[i]).count();
            if (inside as f64) < 0.5 * nb as f64 {
                mask = dominant_region(&x, &y, frac, Some(&bright));
            }
        }
    }
    if mask.iter().filter(|&&m| m).count() < 200 { all } else { mask }
}

const G: usize = 64;

/// The events inside the connected high-density region around the highest 2-D density peak.
pub fn dominant_region(x: &[f64], y: &[f64], frac: f64, basis: Option<&[bool]>) -> Vec<bool> {
    let n = x.len();
    let (xl, xh) = (stats::percentile(x, 1.0), stats::percentile(x, 99.0));
    let (yl, yh) = (stats::percentile(y, 1.0), stats::percentile(y, 99.0));
    if !(xh > xl) || !(yh > yl) {
        return vec![true; n];
    }
    let cell = |v: f64, lo: f64, hi: f64| (((v - lo) / (hi - lo)) * G as f64).floor();
    let idx = |i: usize| -> Option<(usize, usize)> {
        let (cx, cy) = (cell(x[i], xl, xh), cell(y[i], yl, yh));
        (cx >= 0.0 && cx < G as f64 && cy >= 0.0 && cy < G as f64).then(|| (cx as usize, cy as usize))
    };
    let mut h = vec![0.0f64; G * G];
    for i in 0..n {
        if basis.map_or(false, |b| !b[i]) {
            continue;
        }
        if let Some((cx, cy)) = idx(i) {
            h[cy * G + cx] += 1.0;
        }
    }
    // smooth: three passes of a 3 x 3 box
    for _ in 0..3 {
        let mut o = vec![0.0; G * G];
        for cy in 0..G {
            for cx in 0..G {
                let (mut sum, mut cnt) = (0.0, 0.0);
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (px, py) = (cx as i32 + dx, cy as i32 + dy);
                        if px >= 0 && py >= 0 && (px as usize) < G && (py as usize) < G {
                            sum += h[py as usize * G + px as usize];
                            cnt += 1.0;
                        }
                    }
                }
                o[cy * G + cx] = sum / cnt;
            }
        }
        h = o;
    }
    // 1. separate the populations: connected regions above a low floor; the one holding the most events is the
    //    population of interest (a dense debris spot has the highest peak but fewer events)
    let gmax = h.iter().cloned().fold(0.0, f64::max);
    if !(gmax > 0.0) {
        return vec![true; n];
    }
    let flood = |start: usize, ok: &dyn Fn(usize) -> bool, seen: &mut Vec<bool>| -> Vec<usize> {
        let mut out = vec![start];
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(c) = stack.pop() {
            let (cx, cy) = (c % G, c / G);
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let (px, py) = (cx as i32 + dx, cy as i32 + dy);
                if px < 0 || py < 0 || px as usize >= G || py as usize >= G {
                    continue;
                }
                let q = py as usize * G + px as usize;
                if !seen[q] && ok(q) {
                    seen[q] = true;
                    out.push(q);
                    stack.push(q);
                }
            }
        }
        out
    };
    let low = 0.02 * gmax;
    let mut seen = vec![false; G * G];
    let mut best: Option<(f64, Vec<usize>)> = None;
    for c in 0..G * G {
        if !seen[c] && h[c] >= low {
            let comp = flood(c, &|q| h[q] >= low, &mut seen);
            let mass: f64 = comp.iter().map(|&q| h[q]).sum();
            if best.as_ref().map_or(true, |b| mass > b.0) {
                best = Some((mass, comp));
            }
        }
    }
    let Some((_, comp)) = best else { return vec![true; n] };
    // 2. inside it, the region above a fraction of its own peak, connected to that peak
    let in_comp: std::collections::HashSet<usize> = comp.iter().cloned().collect();
    let peak = comp.iter().cloned().max_by(|&a, &b| h[a].partial_cmp(&h[b]).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
    let floor = frac.clamp(0.01, 0.9) * h[peak];
    let mut inside = vec![false; G * G];
    let mut seen2 = vec![false; G * G];
    for q in flood(peak, &|q| in_comp.contains(&q) && h[q] >= floor, &mut seen2) {
        inside[q] = true;
    }
    (0..n).map(|i| idx(i).map_or(false, |(cx, cy)| inside[cy * G + cx])).collect()
}

fn asinh_c(v: f64) -> f64 {
    (v / COFACTOR).asinh()
}

fn stride<T: Copy>(v: &[T], max: usize) -> Vec<T> {
    if v.len() <= max {
        return v.to_vec();
    }
    let step = v.len() as f64 / max as f64;
    (0..max).map(|k| v[(k as f64 * step) as usize]).collect()
}

/// The threshold between a dim and a bright population on the arcsinh scale, by a two-component mixture.
fn gmm_cut(z: &[f64]) -> Option<f64> {
    let z = stride(z, 30_000);
    if z.len() < 200 {
        return None;
    }
    let t0 = stats::otsu(&z);
    let (lo, hi): (Vec<f64>, Vec<f64>) = (z.iter().cloned().filter(|&v| v <= t0).collect(), z.iter().cloned().filter(|&v| v > t0).collect());
    if lo.len() < 20 || hi.len() < 20 {
        return None;
    }
    let all_var = stats::sd(&z).powi(2).max(1e-6);
    let floor = all_var * 1e-4;
    let mut w = [lo.len() as f64 / z.len() as f64, hi.len() as f64 / z.len() as f64];
    let mut m = [stats::mean(&lo), stats::mean(&hi)];
    let mut v = [stats::sd(&lo).powi(2).max(floor), stats::sd(&hi).powi(2).max(floor)];
    let pdf = |x: f64, m: f64, v: f64| (-(x - m).powi(2) / (2.0 * v)).exp() / (2.0 * std::f64::consts::PI * v).sqrt();
    for _ in 0..80 {
        let (mut r0, mut r1, mut s0, mut s1, mut q0, mut q1) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for &x in &z {
            let (a, b) = (w[0] * pdf(x, m[0], v[0]), w[1] * pdf(x, m[1], v[1]));
            let t = a + b;
            let (ga, gb) = if t > 0.0 { (a / t, b / t) } else { (0.5, 0.5) };
            r0 += ga;
            r1 += gb;
            s0 += ga * x;
            s1 += gb * x;
            q0 += ga * x * x;
            q1 += gb * x * x;
        }
        if r0 < 5.0 || r1 < 5.0 {
            return None;
        }
        let nz = z.len() as f64;
        w = [r0 / nz, r1 / nz];
        m = [s0 / r0, s1 / r1];
        v = [(q0 / r0 - m[0] * m[0]).max(floor), (q1 / r1 - m[1] * m[1]).max(floor)];
    }
    let (a, b) = if m[0] <= m[1] { (0, 1) } else { (1, 0) };
    let sep = (m[b] - m[a]) / ((v[a] + v[b]) / 2.0).sqrt();
    if w[a] < 0.03 || w[b] < 0.03 || sep < 2.0 {
        return None;
    }
    // where the two weighted densities cross, between the means
    let mut cut = None;
    let steps = 600;
    let mut prev = w[a] * pdf(m[a], m[a], v[a]) - w[b] * pdf(m[a], m[b], v[b]);
    for k in 1..=steps {
        let x = m[a] + (m[b] - m[a]) * k as f64 / steps as f64;
        let d = w[a] * pdf(x, m[a], v[a]) - w[b] * pdf(x, m[b], v[b]);
        if prev > 0.0 && d <= 0.0 {
            cut = Some(x);
            break;
        }
        prev = d;
    }
    cut
}

/// The right edge of the negative population on the arcsinh scale: find the highest peak, estimate its width from
/// the side away from the positives (the left half, mirrored), and cut a few widths above the peak. Unlike a
/// two-class split this does not need a second peak: a control whose positives are a thin bright tail on a large
/// dim body (common for weakly stained or very autofluorescent cells) is cut above the body, not through it.
fn negative_edge(z: &[f64], widths: f64) -> Option<f64> {
    if z.len() < 200 {
        return None;
    }
    let (lo, hi) = (stats::percentile(z, 0.5), stats::percentile(z, 99.5));
    if !(hi > lo) {
        return None;
    }
    let bins = 256usize;
    let mut h = vec![0.0f64; bins];
    for &v in z {
        let t = (v - lo) / (hi - lo);
        if (0.0..1.0).contains(&t) {
            h[(t * bins as f64) as usize] += 1.0;
        }
    }
    let sm: Vec<f64> = (0..bins).map(|b| (b.saturating_sub(3)..(b + 4).min(bins)).map(|k| h[k]).sum::<f64>()).collect();
    let peak = (0..bins).max_by(|&a, &b| sm[a].partial_cmp(&sm[b]).unwrap_or(std::cmp::Ordering::Equal))?;
    let mode = lo + (hi - lo) * (peak as f64 + 0.5) / bins as f64;
    let left: Vec<f64> = z.iter().cloned().filter(|&v| v <= mode).map(|v| mode - v).collect();
    if left.len() < 50 {
        return None;
    }
    let sd = 1.4826 * stats::median(&left);
    (sd > 0.0).then(|| mode + widths * sd)
}

/// The automatic cut on a primary detector, in raw units. Automatic never cuts inside the negative body (three
/// widths above its peak is the floor), and takes a mixture valley above that when the control has a distinct
/// bright population.
pub fn auto_cut(values: &[f64], how: SplitMethod) -> f64 {
    let z: Vec<f64> = values.iter().filter(|v| v.is_finite()).map(|&v| asinh_c(v)).collect();
    if z.is_empty() {
        return 0.0;
    }
    let cut = match how {
        SplitMethod::Otsu => stats::otsu(&z),
        SplitMethod::Gmm => gmm_cut(&z).unwrap_or_else(|| stats::otsu(&z)),
        // Automatic: never cut inside the negative body. A mixture valley beyond the negative's edge wins (a distinct
        // bright population); otherwise the edge itself is the cut.
        SplitMethod::Auto => match (negative_edge(&z, 3.0), gmm_cut(&z)) {
            (Some(e), Some(g)) => e.max(g),
            (Some(e), None) => e,
            (None, Some(g)) => g,
            (None, None) => stats::otsu(&z),
        },
    };
    COFACTOR * cut.sinh()
}

/// Huber regression of `y` on `x` with an intercept; the slope. `None` when `x` does not vary.
pub fn robust_slope(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len().min(y.len());
    if n < 20 {
        return None;
    }
    let (x, y) = (stride(&x[..n], MAX_EVENTS), stride(&y[..n], MAX_EVENTS));
    let n = x.len();
    let mut w = vec![1.0; n];
    let (mut slope, mut icpt);
    slope = 0.0;
    for it in 0..15 {
        let sw: f64 = w.iter().sum();
        let mx = w.iter().zip(&x).map(|(w, x)| w * x).sum::<f64>() / sw;
        let my = w.iter().zip(&y).map(|(w, y)| w * y).sum::<f64>() / sw;
        let sxx: f64 = (0..n).map(|i| w[i] * (x[i] - mx).powi(2)).sum();
        if !(sxx > 1e-12) {
            return None;
        }
        let sxy: f64 = (0..n).map(|i| w[i] * (x[i] - mx) * (y[i] - my)).sum();
        let s = sxy / sxx;
        let c = my - s * mx;
        let change = (s - slope).abs();
        slope = s;
        icpt = c;
        let res: Vec<f64> = (0..n).map(|i| y[i] - (slope * x[i] + icpt)).collect();
        let scale = (1.4826 * stats::mad(&res)).max(1e-9);
        let k = 1.345 * scale;
        for i in 0..n {
            let a = res[i].abs();
            w[i] = if a <= k { 1.0 } else { k / a };
        }
        if it > 2 && change < 1e-7 {
            break;
        }
    }
    slope.is_finite().then_some(slope)
}

struct Prepared {
    /// Clean events of the control, detectors in the wizard's order, row-major `n x d`.
    x: Vec<f64>,
    n: usize,
    i: usize,
}

fn rows_of(s: &LoadedSample, cols: &[usize], keep: &[bool], max: usize) -> (Vec<f64>, usize) {
    let d = s.channels.len();
    let rows: Vec<usize> = (0..s.n_events).filter(|&r| keep[r]).collect();
    let rows = stride(&rows, max);
    let mut x = Vec::with_capacity(rows.len() * cols.len());
    for &r in &rows {
        for &c in cols {
            x.push(s.events[r * d + c] as f64);
        }
    }
    (x, rows.len())
}

fn medians_of(x: &[f64], d: usize, rows: &[usize]) -> Vec<f64> {
    (0..d).map(|j| stats::median(&rows.iter().map(|&r| x[r * d + j]).collect::<Vec<_>>())).collect()
}

/// Compute the spillover matrix for `detectors` from the controls.
pub fn compute(samples: &[&LoadedSample], setups: &[ControlSetup], detectors: &[String], opt: &Options) -> Result<WizardResult, String> {
    let d = detectors.len();
    if d == 0 {
        return Err("no detectors".into());
    }
    if setups.is_empty() {
        return Err("no controls were assigned".into());
    }
    let mut seen = vec![false; d];
    let mut spill = vec![0.0; d * d];
    for i in 0..d {
        spill[i * d + i] = 1.0;
    }
    let mut reports = Vec::new();
    let mut notes = Vec::new();
    let mut prepared: Vec<Prepared> = Vec::new();

    // the unstained tube's per-detector medians, when it is the negative
    let neg_baseline: Option<Vec<f64>> = match opt.negative {
        NegSource::Own => None,
        NegSource::TrueZero => Some(vec![0.0; d]),
        NegSource::Unstained(u) => {
            let s = samples.get(u).ok_or("the unstained sample does not exist")?;
            let cols: Vec<usize> = detectors.iter().map(|n| s.channel_index(n).ok_or_else(|| format!("{}: has no channel '{n}'", s.name))).collect::<Result<_, _>>()?;
            let keep = clean_mask(s, opt.clean);
            let (x, n) = rows_of(s, &cols, &keep, usize::MAX);
            if n < 50 {
                return Err(format!("{}: too few events for a negative baseline", s.name));
            }
            Some(medians_of(&x, d, &(0..n).collect::<Vec<_>>()))
        }
    };

    for c in setups {
        let s = samples.get(c.sample).ok_or("control sample index out of range")?;
        let i = detectors.iter().position(|n| *n == c.detector).ok_or_else(|| format!("{}: '{}' is not one of the detectors being compensated", s.name, c.detector))?;
        if seen[i] {
            return Err(format!("two controls were given for {}", c.detector));
        }
        let cols: Vec<usize> = detectors.iter().map(|n| s.channel_index(n).ok_or_else(|| format!("{}: has no channel '{n}'", s.name))).collect::<Result<_, _>>()?;
        let keep = clean_mask_for(s, opt.clean, Some(cols[i]));
        let (x, n) = rows_of(s, &cols, &keep, usize::MAX);
        let primary: Vec<f64> = (0..n).map(|r| x[r * d + i]).collect();
        let (neg_max, pos_min) = match c.manual {
            Some((nm, pm)) => (nm, pm),
            None => {
                let cut = auto_cut(&primary, opt.split);
                (cut, cut)
            }
        };
        let pos: Vec<usize> = (0..n).filter(|&r| primary[r] >= pos_min).collect();
        let neg: Vec<usize> = (0..n).filter(|&r| primary[r] <= neg_max).collect();
        let med_pos = if pos.is_empty() { f64::NAN } else { stats::median(&pos.iter().map(|&r| primary[r]).collect::<Vec<_>>()) };
        let med_neg = if neg.is_empty() { f64::NAN } else { stats::median(&neg.iter().map(|&r| primary[r]).collect::<Vec<_>>()) };
        let neg_sd = if neg.len() > 10 { 1.4826 * stats::mad(&neg.iter().map(|&r| primary[r]).collect::<Vec<_>>()) } else { f64::NAN };
        let stain_index = (med_pos - med_neg) / (2.0 * neg_sd);
        let mut badges = Vec::new();
        if n < 1000 {
            badges.push(Badge::TooFewEvents);
        }
        // how far the brightest events stand above the background, in background widths
        let reach = (stats::percentile(&primary, 99.5) - med_neg) / neg_sd.max(1e-9);
        let split_gap = pos.len() < 100 || (pos.len() as f64) < 0.003 * n as f64;
        if (opt.method.needs_split() && split_gap) || (!opt.method.needs_split() && reach.is_finite() && reach < 8.0) {
            badges.push(Badge::MissingPositive);
        }
        if neg.len() < 100 && opt.method.needs_split() && opt.negative == NegSource::Own {
            badges.push(Badge::MissingNegative);
        }
        // a split of pure noise has a stain index near 1: that control has no positive population at all
        if opt.method.needs_split() && stain_index.is_finite() && stain_index < 2.0 && !badges.contains(&Badge::MissingPositive) {
            badges.push(Badge::MissingPositive);
        } else if stain_index.is_finite() && stain_index < 3.0 && opt.method.needs_split() {
            badges.push(Badge::PositiveTooDim);
        }
        if opt.clean == CleanMode::Off && scatter_channel(s, "FSC").is_some() {
            badges.push(Badge::NoScatterCleanup);
        }
        let blocked = badges.iter().any(|b| b.blocks()) || (badges.contains(&Badge::MissingNegative) && opt.method.needs_split());
        reports.push(ControlReport { sample: c.sample, detector: c.detector.clone(), n_events: s.n_events, n_clean: n, n_pos: pos.len(), n_neg: neg.len(), neg_max, pos_min, median_pos: med_pos, median_neg: med_neg, stain_index, badges: badges.clone() });
        if blocked {
            let why: Vec<&str> = badges.iter().filter(|b| b.blocks() || **b == Badge::MissingNegative).map(|b| b.label()).collect();
            return Err(format!("{} ({}): {}", s.name, c.detector, why.join("; ")));
        }
        seen[i] = true;

        // ---- the row
        let primary_all = &primary;
        for j in 0..d {
            if j == i {
                continue;
            }
            let y: Vec<f64> = (0..n).map(|r| x[r * d + j]).collect();
            let v = match opt.method {
                Method::Medians => {
                    let mp_j = stats::median(&pos.iter().map(|&r| y[r]).collect::<Vec<_>>());
                    let (neg_j, neg_i) = match &neg_baseline {
                        Some(b) => (b[j], b[i]),
                        None => (stats::median(&neg.iter().map(|&r| y[r]).collect::<Vec<_>>()), med_neg),
                    };
                    (mp_j - neg_j) / (med_pos - neg_i)
                }
                Method::Slope => crate::comp_auto::binned_slope(primary_all, &y, 25).ok_or_else(|| format!("{}: too few events to fit the spillover of {}", s.name, c.detector))?,
                Method::Refined => robust_slope(primary_all, &y).ok_or_else(|| format!("{}: the events do not vary on {}", s.name, c.detector))?,
            };
            spill[i * d + j] = v;
        }
        let (xs, ns) = {
            let rows: Vec<usize> = (0..n).collect();
            let rows = stride(&rows, REFINE_EVENTS);
            let mut v = Vec::with_capacity(rows.len() * d);
            for &r in &rows {
                v.extend_from_slice(&x[r * d..(r + 1) * d]);
            }
            (v, rows.len())
        };
        prepared.push(Prepared { x: xs, n: ns, i });
    }

    let mut clamped = 0;
    for v in spill.iter_mut().enumerate().filter(|(k, _)| k / d != k % d).map(|(_, v)| v) {
        if *v < 0.0 {
            *v = 0.0; // a negative spillover is background noise, not physics
            clamped += 1;
        }
    }
    if clamped > 0 {
        notes.push(format!("{clamped} slightly negative coefficient(s) set to 0"));
    }

    // ---- refinement
    let (mut rounds, mut residual) = (0usize, f64::NAN);
    if opt.method == Method::Refined {
        let (r, e) = refine(&mut spill, d, &prepared);
        rounds = r;
        residual = e;
        if e > 1e-3 {
            notes.push(format!("refinement stopped with a remaining slope of {e:.4} on a compensated control"));
        }
    }

    for (i, name) in detectors.iter().enumerate() {
        if !seen[i] {
            notes.push(format!("no control for {name}: its row is the identity (it spills into nothing)"));
        }
    }

    // ---- quality
    let spread = spreading(&spill, d, &prepared);
    let cosine = cosine_matrix(&spill, d);
    let condition = match linalg::singular_values(&spill, d, d) {
        Ok(sv) if !sv.is_empty() => {
            let (mx, mn) = (sv.iter().cloned().fold(f64::MIN, f64::max), sv.iter().cloned().fold(f64::MAX, f64::min));
            if mn > 0.0 { mx / mn } else { f64::INFINITY }
        }
        _ => f64::NAN,
    };
    if condition.is_finite() && condition > 30.0 {
        notes.push(format!("the matrix is poorly conditioned ({condition:.0}): compensation will amplify noise"));
    }
    Ok(WizardResult { matrix: CompInfo { detectors: detectors.to_vec(), spill, source: format!("wizard: {}", opt.method.label()) }, reports, spread, cosine, condition, rounds, residual_slope: residual, notes })
}

/// Make the compensated controls show no leftover slope. With `r_i` the compensated row of control `i` (primary
/// normalised to 1) and `E_i` its off-primary slopes, the exact correction is `S_i <- (e_i + E_i) S`; damping guards
/// against oscillation on noisy data. Returns (rounds, largest remaining slope).
fn refine(spill: &mut Vec<f64>, d: usize, ctrls: &[Prepared]) -> (usize, f64) {
    let mut damp = 1.0;
    let mut last = f64::INFINITY;
    let mut best = spill.clone();
    let mut stalled = 0;
    let mut max_e = f64::NAN;
    for round in 0..20 {
        let Ok(inv) = linalg::invert(spill, d) else { return (round, max_e) };
        let mut errs: Vec<(usize, Vec<f64>)> = Vec::new();
        max_e = 0.0;
        for c in ctrls {
            let comp = linalg::matmul(&c.x, &inv, c.n, d, d);
            let xi: Vec<f64> = (0..c.n).map(|r| comp[r * d + c.i]).collect();
            let mut e = vec![0.0; d];
            for j in 0..d {
                if j == c.i {
                    continue;
                }
                let yj: Vec<f64> = (0..c.n).map(|r| comp[r * d + j]).collect();
                e[j] = robust_slope(&xi, &yj).unwrap_or(0.0);
                max_e = max_e.max(e[j].abs());
            }
            errs.push((c.i, e));
        }
        // 5e-4 is below what a robust line through a few thousand noisy events can resolve
        if max_e < 5e-4 {
            return (round, max_e);
        }
        if max_e >= last * 0.95 {
            stalled += 1;
            if stalled >= 2 {
                // no longer improving: the noise floor of the fits, or an oscillation; stop with the best state
                if max_e > last {
                    *spill = best;
                    return (round, last);
                }
                return (round, max_e);
            }
            damp = 0.5;
        } else {
            stalled = 0;
        }
        last = max_e;
        best = spill.clone();
        let old = spill.clone();
        for (i, e) in &errs {
            let mut t = vec![0.0; d];
            for j in 0..d {
                t[j] = damp * e[j];
            }
            t[*i] = 1.0;
            let row = linalg::row_times_matrix(&t, &old, d);
            let p = row[*i];
            if p.abs() > 1e-9 {
                for j in 0..d {
                    spill[i * d + j] = (row[j] / p).max(if j == *i { 1.0 } else { 0.0 });
                }
            }
        }
    }
    (20, max_e)
}

/// Spillover spreading by a linear model: on the compensated control, bin the events by the stained detector's
/// intensity `F`, take the robust variance of every other detector in each bin (84th percentile minus median,
/// squared), and fit variance = a + b F. The spread coefficient is sqrt(b): the noise standard deviation added per
/// square root of intensity. A negative slope means no measurable spreading and gives 0.
fn spreading(spill: &[f64], d: usize, ctrls: &[Prepared]) -> Vec<f64> {
    let mut out = vec![0.0; d * d];
    let Ok(inv) = linalg::invert(spill, d) else { return out };
    for c in ctrls {
        if c.n < 400 {
            continue;
        }
        let comp = linalg::matmul(&c.x, &inv, c.n, d, d);
        let mut order: Vec<usize> = (0..c.n).collect();
        order.sort_by(|&a, &b| comp[a * d + c.i].partial_cmp(&comp[b * d + c.i]).unwrap_or(std::cmp::Ordering::Equal));
        let bins = (c.n / 100).clamp(8, 32);
        for j in 0..d {
            if j == c.i {
                continue;
            }
            let mut pts = Vec::new();
            for b in 0..bins {
                let (lo, hi) = (b * c.n / bins, (b + 1) * c.n / bins);
                if hi - lo < 20 {
                    continue;
                }
                let f = stats::median(&order[lo..hi].iter().map(|&r| comp[r * d + c.i]).collect::<Vec<_>>());
                let ys: Vec<f64> = order[lo..hi].iter().map(|&r| comp[r * d + j]).collect();
                let s = stats::percentile(&ys, 84.0) - stats::median(&ys);
                pts.push((f.max(0.0), s * s));
            }
            if pts.len() < 4 {
                continue;
            }
            let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / pts.len() as f64, pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64);
            let sxx: f64 = pts.iter().map(|p| (p.0 - mx).powi(2)).sum();
            if sxx > 0.0 {
                let b: f64 = pts.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum::<f64>() / sxx;
                out[c.i * d + j] = if b > 0.0 { b.sqrt() } else { 0.0 };
            }
        }
    }
    out
}

/// Cosine similarity between every pair of spillover rows.
pub fn cosine_matrix(spill: &[f64], d: usize) -> Vec<f64> {
    let norm: Vec<f64> = (0..d).map(|i| (0..d).map(|j| spill[i * d + j].powi(2)).sum::<f64>().sqrt()).collect();
    let mut out = vec![0.0; d * d];
    for i in 0..d {
        for k in 0..d {
            let dot: f64 = (0..d).map(|j| spill[i * d + j] * spill[k * d + j]).sum();
            out[i * d + k] = if norm[i] > 0.0 && norm[k] > 0.0 { dot / (norm[i] * norm[k]) } else { 0.0 };
        }
    }
    out
}

/// The control's events after compensation with `m`, for the before/after preview: `(primary, other)` columns.
pub fn compensated_pair(s: &LoadedSample, m: &CompInfo, primary: &str, other: &str) -> Result<(Vec<f64>, Vec<f64>), String> {
    let d = m.detectors.len();
    let cols: Vec<usize> = m.detectors.iter().map(|n| s.channel_index(n).ok_or_else(|| format!("{}: has no channel '{n}'", s.name))).collect::<Result<_, _>>()?;
    let (pi, oi) = (m.detectors.iter().position(|n| n == primary).ok_or("unknown detector")?, m.detectors.iter().position(|n| n == other).ok_or("unknown detector")?);
    let inv = linalg::invert(&m.spill, d).map_err(|e| format!("the matrix cannot be inverted: {e:?}"))?;
    let keep = vec![true; s.n_events];
    let (x, n) = rows_of(s, &cols, &keep, MAX_EVENTS);
    let comp = linalg::matmul(&x, &inv, n, d, d);
    Ok(((0..n).map(|r| comp[r * d + pi]).collect(), (0..n).map(|r| comp[r * d + oi]).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::rng::Rng;

    fn normal(rng: &mut Rng) -> f64 {
        let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    const DET: [&str; 4] = ["FITC-A", "PE-A", "APC-A", "BV421-A"];

    fn truth() -> Vec<f64> {
        vec![1.0, 0.18, 0.02, 0.0, 0.04, 1.0, 0.25, 0.0, 0.0, 0.03, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
    }

    /// FSC/SSC then the four detectors. Debris (low scatter) is mixed in; spreading noise grows with brightness.
    fn control(i: Option<usize>, s_true: &[f64], n: usize, seed: u64, debris: bool) -> LoadedSample {
        let mut rng = Rng::new(seed);
        let mut ev = Vec::new();
        for k in 0..n {
            let junk = debris && k % 5 == 0;
            let (fsc, ssc) = if junk { (15_000.0 + 3000.0 * normal(&mut rng), 6_000.0 + 1500.0 * normal(&mut rng)) } else { (120_000.0 + 12_000.0 * normal(&mut rng), 50_000.0 + 6_000.0 * normal(&mut rng)) };
            ev.push(fsc as f32);
            ev.push(ssc as f32);
            // half positive: brightness lognormal; junk is dim in every colour
            let bright = match i {
                Some(_) if k % 2 == 0 && !junk => 8000.0 * (0.4 * normal(&mut rng)).exp(),
                _ => 0.0,
            };
            for j in 0..4 {
                let sig = i.map_or(0.0, |i| bright * s_true[i * 4 + j]);
                let sd = (40.0f64.powi(2) + 0.6 * sig.max(0.0)).sqrt();
                ev.push((sig + 80.0 + sd * normal(&mut rng)) as f32);
            }
        }
        let name = i.map_or("unstained.fcs".to_string(), |i| format!("{} alone.fcs", DET[i].trim_end_matches("-A")));
        LoadedSample::synthetic(&name, &["FSC-A", "SSC-A", "FITC-A", "PE-A", "APC-A", "BV421-A"], vec![262_144.0; 6], ev)
    }

    fn controls(debris: bool) -> Vec<LoadedSample> {
        let t = truth();
        (0..3).map(|i| control(Some(i), &t, 6000, 40 + i as u64, debris)).collect()
    }

    fn run(samples: &[LoadedSample], opt: Options) -> WizardResult {
        let refs: Vec<&LoadedSample> = samples.iter().collect();
        let det: Vec<String> = DET.iter().map(|s| s.to_string()).collect();
        let setups: Vec<ControlSetup> = (0..3).map(|i| ControlSetup { sample: i, detector: DET[i].to_string(), manual: None }).collect();
        compute(&refs, &setups, &det, &opt).unwrap()
    }

    fn close_to_truth(r: &WizardResult, tol: f64) {
        let t = truth();
        for i in 0..3 {
            for j in 0..4 {
                let v = r.matrix.spill[i * 4 + j];
                assert!((v - t[i * 4 + j]).abs() < tol, "S[{i}][{j}] = {v} vs {} ({:?})", t[i * 4 + j], r.matrix.source);
            }
        }
    }

    #[test]
    fn every_method_recovers_the_matrix_that_generated_the_controls() {
        let s = controls(true);
        for method in Method::ALL {
            let r = run(&s, Options { method, ..Options::default() });
            close_to_truth(&r, 0.015);
            assert!(r.reports.iter().all(|c| c.n_clean < c.n_events), "the scatter clean-up dropped the debris");
        }
    }

    #[test]
    fn the_refinement_converges_and_fixes_a_biased_start() {
        let s = controls(false);
        let r = run(&s, Options { method: Method::Refined, clean: CleanMode::Off, ..Options::default() });
        assert!(r.rounds < 20 && r.residual_slope < 2e-3, "rounds {} residual {}", r.rounds, r.residual_slope);
        close_to_truth(&r, 0.012);
        // a deliberately wrong start is pulled back by refine()
        let t = truth();
        let det = 4;
        let mut bad = t.clone();
        bad[1] = 0.30; // FITC -> PE overstated
        let prepared: Vec<Prepared> = (0..3)
            .map(|i| {
                let s = control(Some(i), &t, 3000, 90 + i as u64, false);
                let (x, n) = rows_of(&s, &[2, 3, 4, 5], &vec![true; s.n_events], usize::MAX);
                Prepared { x, n, i }
            })
            .collect();
        let (rounds, e) = refine(&mut bad, det, &prepared);
        assert!(e < 5e-3 && rounds > 0, "rounds {rounds} residual {e}");
        assert!((bad[1] - 0.18).abs() < 0.015, "FITC->PE refined to {}", bad[1]);
    }

    #[test]
    fn an_unstained_negative_and_true_zero_change_the_median_method_as_they_should() {
        let mut s = controls(false);
        s.push(control(None, &truth(), 6000, 70, false));
        let own = run(&s, Options { method: Method::Medians, clean: CleanMode::Off, ..Options::default() });
        let un = run(&s, Options { method: Method::Medians, clean: CleanMode::Off, negative: NegSource::Unstained(3), ..Options::default() });
        close_to_truth(&un, 0.015);
        for k in 0..16 {
            assert!((own.matrix.spill[k] - un.matrix.spill[k]).abs() < 0.02);
        }
        // a true-zero negative ignores the 80-unit background: the coefficients still land close, not exactly
        let zero = run(&s, Options { method: Method::Medians, clean: CleanMode::Off, negative: NegSource::TrueZero, ..Options::default() });
        close_to_truth(&zero, 0.03);
    }

    #[test]
    fn the_automatic_split_finds_the_valley_and_a_manual_split_is_respected() {
        let s = control(Some(1), &truth(), 8000, 5, false);
        let col = column(&s, 3);
        for how in [SplitMethod::Auto, SplitMethod::Gmm, SplitMethod::Otsu] {
            let cut = auto_cut(&col, how);
            assert!(cut > 150.0 && cut < 5000.0, "{how:?}: {cut}");
        }
        let refs = vec![&s];
        let det: Vec<String> = DET.iter().map(|s| s.to_string()).collect();
        let setups = vec![ControlSetup { sample: 0, detector: "PE-A".into(), manual: Some((400.0, 3000.0)) }];
        let r = compute(&refs, &setups, &det, &Options { method: Method::Medians, clean: CleanMode::Off, ..Options::default() }).unwrap();
        let c = &r.reports[0];
        assert_eq!((c.neg_max, c.pos_min), (400.0, 3000.0));
        assert!(c.n_pos > 2000 && c.n_pos < 4000 && c.n_neg > 3000, "{} pos {} neg", c.n_pos, c.n_neg);
    }

    #[test]
    fn the_scatter_clean_up_keeps_the_main_population_and_not_the_debris() {
        let s = control(Some(0), &truth(), 5000, 8, true);
        let keep = clean_mask(&s, CleanMode::Dominant(0.12));
        let d = s.channels.len();
        let (mut junk_kept, mut junk, mut good_kept, mut good) = (0, 0, 0, 0);
        for r in 0..s.n_events {
            let is_junk = s.events[r * d] < 40_000.0;
            if is_junk {
                junk += 1;
                junk_kept += keep[r] as usize;
            } else {
                good += 1;
                good_kept += keep[r] as usize;
            }
        }
        assert!(junk_kept * 20 < junk, "debris kept: {junk_kept} of {junk}");
        assert!(good_kept * 10 > good * 8, "main population kept: {good_kept} of {good}");
        // a sample without scatter channels is left alone
        let bare = LoadedSample::synthetic("b.fcs", &["FITC-A", "PE-A"], vec![1.0; 2], vec![1.0; 800]);
        assert!(clean_mask(&bare, CleanMode::Dominant(0.12)).iter().all(|&k| k));
    }

    #[test]
    fn badges_warn_about_unusable_and_doubtful_controls() {
        let det: Vec<String> = DET.iter().map(|s| s.to_string()).collect();
        // an unstained tube given as a control: nothing is positive
        let blank = control(None, &truth(), 6000, 3, false);
        let refs = vec![&blank];
        let setups = vec![ControlSetup { sample: 0, detector: "FITC-A".into(), manual: None }];
        let e = compute(&refs, &setups, &det, &Options { clean: CleanMode::Off, ..Options::default() }).unwrap_err();
        assert!(e.contains("positive") || e.contains("barely"), "{e}");
        // too few events
        let few = control(Some(0), &truth(), 300, 4, false);
        let e = compute(&[&few], &setups, &det, &Options::default()).unwrap_err();
        assert!(e.contains("Too few"), "{e}");
        // a good control without clean-up carries the advice badge, not an error
        let good = control(Some(0), &truth(), 6000, 6, false);
        let r = compute(&[&good], &setups, &det, &Options { clean: CleanMode::Off, ..Options::default() }).unwrap();
        assert!(r.reports[0].badges.contains(&Badge::NoScatterCleanup));
        // two controls for one detector, and a detector that does not exist
        let both = vec![ControlSetup { sample: 0, detector: "FITC-A".into(), manual: None }, ControlSetup { sample: 0, detector: "FITC-A".into(), manual: None }];
        assert!(compute(&[&good], &both, &det, &Options::default()).unwrap_err().contains("two controls"));
        let bad = vec![ControlSetup { sample: 0, detector: "XYZ-A".into(), manual: None }];
        assert!(compute(&[&good], &bad, &det, &Options::default()).is_err());
    }

    #[test]
    fn spreading_follows_the_noise_that_was_put_in_and_cosine_follows_the_rows() {
        // FITC adds noise sd = sqrt(0.6 * signal) to PE only through its 18 percent spillover; the spread coefficient
        // measured on the compensated control is of the order of sqrt(0.6 x 0.18) = 0.33 for PE and ~0 for BV421's
        // untouched detector (pure noise, no signal-dependent term)
        let s = controls(false);
        let r = run(&s, Options { method: Method::Refined, clean: CleanMode::Off, ..Options::default() });
        let pe = r.spread[0 * 4 + 1];
        let bv = r.spread[0 * 4 + 3];
        assert!(pe > 0.15 && pe < 0.7, "spread into PE {pe}");
        assert!(bv < pe * 0.5, "spread into BV421 {bv} vs PE {pe}");
        // cosine: identical rows give 1, orthogonal rows 0
        let c = cosine_matrix(&[1.0, 0.0, 0.0, 1.0], 2);
        assert_eq!(c, vec![1.0, 0.0, 0.0, 1.0]);
        let c = cosine_matrix(&[1.0, 1.0, 2.0, 2.0], 2);
        assert!((c[1] - 1.0).abs() < 1e-12);
        // the matrix here is well conditioned
        assert!(r.condition.is_finite() && r.condition < 5.0, "{}", r.condition);
    }

    #[test]
    fn compensated_controls_show_no_spillover_in_the_preview_pair() {
        let s = controls(false);
        let r = run(&s, Options { method: Method::Refined, clean: CleanMode::Off, ..Options::default() });
        let (x, y) = compensated_pair(&s[0], &r.matrix, "FITC-A", "PE-A").unwrap();
        let before = robust_slope(&column(&s[0], 2), &column(&s[0], 3)).unwrap();
        let after = robust_slope(&x, &y).unwrap();
        assert!(before > 0.12 && after.abs() < 0.01, "slope before {before}, after {after}");
    }

    #[test]
    fn the_robust_slope_ignores_outliers_that_would_drag_a_least_squares_line() {
        let mut rng = Rng::new(1);
        let x: Vec<f64> = (0..2000).map(|_| 1000.0 * rng.next_f64()).collect();
        let mut y: Vec<f64> = x.iter().map(|v| 0.2 * v + 5.0 * normal(&mut rng)).collect();
        for k in 0..60 {
            y[k] += 4000.0;
        }
        let s = robust_slope(&x, &y).unwrap();
        assert!((s - 0.2).abs() < 0.01, "{s}");
        assert!(robust_slope(&[1.0; 100], &[2.0; 100]).is_none());
    }
}


