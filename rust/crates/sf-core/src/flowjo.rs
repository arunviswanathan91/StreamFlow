//! FlowJo-compatible transforms, needed to read gates stored in FlowJo workspaces (`.wsp`).
//!
//! FlowJo's biexponential display scale is defined by a 4097-point lookup table built from a
//! biexponential `x(t) = minimum * (e^{P t} - S e^{-N t} - c)` and then linearly interpolated, so the
//! table construction *is* the definition. The construction below follows the one FlowKit documents
//! and ships (BSD-3-Clause) for FlowJo 10 compatibility; it is checked against FlowKit's own table to
//! 1e-9 in `tests/reference.rs`. Output is in FlowJo's 0..4096 "channel" units.

use crate::transforms::Transform;

pub const BIEX_CHANNELS: usize = 4096;

/// Newton/bisection hybrid for the root `f(d) = 2 ln d + w d + fb` (the "negative range" of the biex).
fn log_root(b: f64, w: f64) -> f64 {
    if w == 0.0 {
        return b;
    }
    let (mut x_lo, mut x_hi) = (0.0f64, b);
    let mut d = (x_lo + x_hi) / 2.0;
    let mut dx = (x_lo - x_hi).trunc().abs();
    let mut dx_last = dx;
    let fb = -2.0 * b.ln() + w * b;
    let mut f = 2.0 * d.ln() + w * b + fb;
    let mut df = 2.0 / d + w;
    for _ in 0..100 {
        if (((d - x_hi) * df - f) - ((d - x_lo) * df - f)) > 0.0 || (2.0 * f).abs() > (dx_last * df).abs() {
            dx = (x_hi - x_lo) / 2.0;
            d = x_lo + dx;
            if d == x_lo {
                return d;
            }
        } else {
            dx = f / df;
            let t = d;
            d -= dx;
            if d == t {
                return d;
            }
        }
        if dx.abs() < 1.0e-12 {
            return d;
        }
        dx_last = dx;
        f = 2.0 * d.ln() + w * d + fb;
        df = 2.0 / d + w;
        if f < 0.0 {
            x_lo = d;
        } else {
            x_hi = d;
        }
    }
    d
}

/// FlowJo's biexponential scale (`<transforms:biex>` in a workspace).
#[derive(Debug, Clone)]
pub struct Biex {
    /// Raw data values, strictly increasing (length 4097).
    xs: Vec<f64>,
    /// Channel positions 0..=4096.
    ys: Vec<f64>,
}

impl Biex {
    /// `negative` = extra negative decades, `width` = FlowJo's width basis (negative, e.g. -10),
    /// `positive` = positive decades, `max_value` = top of the scale.
    pub fn new(negative: f64, width: f64, positive: f64, max_value: f64) -> Result<Biex, String> {
        if !(width < 0.0) || !width.is_finite() {
            return Err(format!("biex width basis must be a negative number (got {width})"));
        }
        if !(positive.is_finite() && positive > 0.0 && negative.is_finite() && max_value.is_finite() && max_value > 0.0) {
            return Err(format!("invalid biex parameters neg={negative} pos={positive} max={max_value}"));
        }
        let range = BIEX_CHANNELS as f64;
        let ln10 = 10f64.ln();
        let mut w = (-width).log10();
        let mut decades = positive - w / 2.0;
        let extra = negative.max(0.0) + w / 2.0;
        let mut zero_point = ((extra * range) / (extra + decades)).trunc() as i64;
        zero_point = zero_point.min(BIEX_CHANNELS as i64 / 2);
        if decades <= 0.0 {
            return Err(format!("biex positive decades ({positive}) too small for width basis {width}"));
        }
        if zero_point > 0 {
            decades = extra * range / zero_point as f64;
        }
        w /= 2.0 * decades;
        let positive_range = ln10 * decades;
        let minimum = max_value / positive_range.exp();
        let negative_range = log_root(positive_range, w);

        let n = BIEX_CHANNELS + 1;
        let nf = n as f64;
        let mut pos: Vec<f64> = (0..n).map(|i| (i as f64 / nf * positive_range).exp()).collect();
        let s = ((positive_range + negative_range) * (w + extra / decades)).exp();
        let neg: Vec<f64> = (0..n).map(|i| (i as f64 / nf * -negative_range).exp() * s).collect();
        let zp = zero_point as usize;
        let s0 = pos[zp] - neg[zp];
        for i in zp..n {
            pos[i] = minimum * ((pos[i] - neg[i]) - s0);
        }
        for k in 0..zp {
            let m = 2 * zp - k;
            pos[k] = -pos[m];
        }
        let ys: Vec<f64> = (0..n).map(|i| i as f64).collect();
        if pos.windows(2).any(|p| !(p[1] > p[0])) || pos.iter().any(|v| !v.is_finite()) {
            return Err("biex parameters give a non-monotonic table".into());
        }
        Ok(Biex { xs: pos, ys })
    }

    /// FlowJo's defaults: `neg 0`, `width -10`, `pos 4.41854`, `max 262144.000029`.
    pub fn flowjo_default() -> Biex {
        Biex::new(0.0, -10.0, 4.41854, 262144.000029).expect("default parameters are valid")
    }
}

/// `numpy.interp`-style linear interpolation clamped to the end values; NaN stays NaN.
fn interp(xs: &[f64], ys: &[f64], v: f64) -> f64 {
    if v.is_nan() {
        return f64::NAN;
    }
    if v <= xs[0] {
        return ys[0];
    }
    let last = xs.len() - 1;
    if v >= xs[last] {
        return ys[last];
    }
    let hi = xs.partition_point(|&x| x <= v); // first index with xs[hi] > v, in 1..=last
    let lo = hi - 1;
    let t = (v - xs[lo]) / (xs[hi] - xs[lo]);
    ys[lo] + t * (ys[hi] - ys[lo])
}

impl Transform for Biex {
    fn forward(&self, x: f64) -> f64 {
        interp(&self.xs, &self.ys, x)
    }
    fn inverse(&self, y: f64) -> f64 {
        interp(&self.ys, &self.xs, y)
    }
    fn name(&self) -> &'static str {
        "biex"
    }
}

/// FlowJo's log scale: `(log10(max(x, offset)) - log10(offset)) / decades`.
#[derive(Debug, Clone, Copy)]
pub struct WspLog {
    pub offset: f64,
    pub decades: f64,
}

impl WspLog {
    pub fn new(offset: f64, decades: f64) -> Result<WspLog, String> {
        if !(offset > 0.0 && decades > 0.0) || !offset.is_finite() || !decades.is_finite() {
            return Err(format!("invalid FlowJo log parameters offset={offset} decades={decades}"));
        }
        Ok(WspLog { offset, decades })
    }
}

impl Transform for WspLog {
    fn forward(&self, x: f64) -> f64 {
        let v = if x < self.offset { self.offset } else { x };
        (1.0 / self.decades) * (v.log10() - self.offset.log10())
    }
    fn inverse(&self, y: f64) -> f64 {
        self.offset * 10f64.powf(y * self.decades)
    }
    fn name(&self) -> &'static str {
        "wsplog"
    }
}
