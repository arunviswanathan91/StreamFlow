//! Logicle (biexponential) data scale: Parks, Moore & Roederer (Cytometry A, 2012).
//!
//! The scale is defined by its **inverse**, a closed-form biexponential
//! `x(s) = a e^{bs} - c e^{-ds} - f`, antisymmetric about the zero point `x1`. The forward map is
//! solved exactly (bracketed Newton on the monotone inverse), **not** read from a lookup table, so it
//! agrees with `flowutils` to ~1e-9 instead of the ~1e-4 a 16k-entry LUT gives. At 500k events the
//! solve costs a few tens of milliseconds, which is cheap enough not to need the approximation.

use crate::transforms::Transform;

const LN10: f64 = std::f64::consts::LN_10;
/// Beyond this scaled coordinate the biexponential overflows `f64` anyway.
const S_MAX: f64 = 50.0;

#[derive(Debug, Clone)]
pub struct Logicle {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    f: f64,
    x1: f64,
}

impl Logicle {
    /// `t` top of scale (> 0), `w` linear width in decades (>= 0, typ. 0.5), `m` total decades
    /// (> 0, typ. 4.5), `a` extra negative decades (>= 0, typ. 0). Requires `2w <= m`.
    pub fn new(t: f64, w: f64, m: f64, a: f64) -> Result<Logicle, String> {
        if !(t > 0.0 && m > 0.0 && w >= 0.0 && a >= 0.0) || ![t, w, m, a].iter().all(|v| v.is_finite()) {
            return Err(format!("invalid logicle parameters T={t} W={w} M={m} A={a}"));
        }
        if 2.0 * w > m {
            return Err(format!("logicle width W={w} too large: needs 2W <= M ({m})"));
        }
        let w_ = w / (m + a);
        let x2 = a / (m + a);
        let x1 = x2 + w_;
        let x0 = x2 + 2.0 * w_;
        let b = (m + a) * LN10;
        let d = solve_d(b, w_);
        let ca = (x0 * (b + d)).exp();
        let fa = (b * x1).exp() - ca / (d * x1).exp();
        let aa = t / ((b.exp() - fa) - ca / d.exp());
        Ok(Logicle { a: aa, b, c: ca * aa, d, f: fa * aa, x1 })
    }

    /// Biexponential for `s >= x1`.
    #[inline]
    fn inv_pos(&self, s: f64) -> f64 {
        self.a * (self.b * s).exp() - self.c * (-self.d * s).exp() - self.f
    }

    /// Scaled coordinate -> raw data (closed form, antisymmetric about the zero point).
    pub fn inverse(&self, x: f64) -> f64 {
        if x < self.x1 {
            -self.inv_pos(2.0 * self.x1 - x)
        } else {
            self.inv_pos(x)
        }
    }

    /// Raw data -> scaled coordinate. NaN stays NaN; infinities pass through.
    pub fn scale(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return value;
        }
        if value < 0.0 {
            2.0 * self.x1 - self.scale_pos(-value)
        } else {
            self.scale_pos(value)
        }
    }

    /// Solve `inv_pos(s) = v` for `s >= x1`, `v >= 0`.
    fn scale_pos(&self, v: f64) -> f64 {
        if v == 0.0 {
            return self.x1;
        }
        let (mut lo, mut hi) = (self.x1, 1.0f64);
        while self.inv_pos(hi) < v {
            lo = hi;
            hi += 1.0;
            if hi >= S_MAX {
                return S_MAX;
            }
        }
        let mut s = 0.5 * (lo + hi);
        for _ in 0..100 {
            let f = self.inv_pos(s) - v;
            if f > 0.0 {
                hi = s;
            } else {
                lo = s;
            }
            let df = self.a * self.b * (self.b * s).exp() + self.c * self.d * (-self.d * s).exp();
            let mut next = s - f / df;
            if !(next > lo && next < hi) {
                next = 0.5 * (lo + hi); // Newton left the bracket: bisect
            }
            if (next - s).abs() <= 1e-15 * s.abs().max(1.0) {
                return next;
            }
            s = next;
        }
        s
    }
}

impl Transform for Logicle {
    fn forward(&self, x: f64) -> f64 {
        self.scale(x)
    }
    fn inverse(&self, y: f64) -> f64 {
        Logicle::inverse(self, y)
    }
    fn name(&self) -> &'static str {
        "logicle"
    }
}

/// Root of g(d) = 2 ln(d/b) + w (b + d); monotonic increasing on (0, b).
fn solve_d(b: f64, w: f64) -> f64 {
    if w == 0.0 {
        return b;
    }
    let (mut lo, mut hi) = (1e-300, b);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let g = 2.0 * (mid / b).ln() + w * (b + mid);
        if g > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    0.5 * (lo + hi)
}

#[cfg(test)]
mod tests {
    use super::*;
    const T: f64 = 262144.0;

    fn lg() -> Logicle {
        Logicle::new(T, 0.5, 4.5, 0.0).unwrap()
    }

    #[test]
    fn matches_the_flowutils_reference_to_the_old_tolerance() {
        let reference = [
            (-1000.0, -0.232115),
            (0.0, 0.111111),
            (100.0, 0.213181),
            (1000.0, 0.454338),
            (10000.0, 0.683833),
            (100000.0, 0.906928),
            (262144.0, 1.0),
        ];
        for (x, want) in reference {
            let got = lg().scale(x);
            assert!((got - want).abs() < 1e-6, "logicle({x}) = {got}, want {want}");
        }
    }

    #[test]
    fn the_defining_points_hold_exactly() {
        let l = lg();
        assert!((l.scale(T) - 1.0).abs() < 1e-12, "T must map to 1");
        assert!(l.inverse(l.scale(0.0)).abs() < 1e-9);
        // antisymmetry: scale(-x) mirrors scale(x) about the zero point
        let z = l.scale(0.0);
        for x in [1.0, 50.0, 1e3, 1e5] {
            assert!(((l.scale(-x) - z) + (l.scale(x) - z)).abs() < 1e-12, "x={x}");
        }
    }

    #[test]
    fn exact_solve_round_trips_to_machine_precision() {
        let l = lg();
        for v in [-5e4, -500.0, -1.0, 0.0, 1e-3, 1.0, 50.0, 5000.0, 200000.0, 262144.0, 5e5] {
            let back = l.inverse(l.scale(v));
            assert!((back - v).abs() <= 1e-9 * v.abs().max(1.0), "round-trip {v} -> {back}");
        }
    }

    #[test]
    fn is_strictly_monotonic() {
        let l = lg();
        let mut prev = f64::NEG_INFINITY;
        let mut v = -20000.0;
        while v <= 4e5 {
            let s = l.scale(v);
            assert!(s > prev, "not increasing at {v}");
            prev = s;
            v += 997.0;
        }
    }

    #[test]
    fn bad_parameters_are_rejected_and_non_finite_input_passes_through() {
        assert!(Logicle::new(0.0, 0.5, 4.5, 0.0).is_err());
        assert!(Logicle::new(T, 3.0, 4.5, 0.0).is_err()); // 2W > M
        assert!(Logicle::new(f64::NAN, 0.5, 4.5, 0.0).is_err());
        assert!(lg().scale(f64::NAN).is_nan());
        assert_eq!(lg().scale(f64::INFINITY), f64::INFINITY);
    }
}
