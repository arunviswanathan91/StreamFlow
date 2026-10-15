//! Display/analysis transforms. Each is a strictly increasing map with an exact inverse, so a
//! threshold chosen in transformed space can be reported back in raw units (the engine's
//! `_invert_matrix_transform`, but closed-form instead of numeric root finding).

pub use crate::logicle::Logicle;

pub trait Transform {
    fn forward(&self, x: f64) -> f64;
    fn inverse(&self, y: f64) -> f64;
    fn name(&self) -> &'static str;
}

/// `asinh(x / cofactor)`: the engine's arcsinh (note: no log10 rescaling, unlike GatingML's asinh).
/// The cofactor is floored at 1e-9 exactly as the engine does.
#[derive(Debug, Clone, Copy)]
pub struct Arcsinh {
    pub cofactor: f64,
}

impl Arcsinh {
    pub fn new(cofactor: f64) -> Arcsinh {
        Arcsinh { cofactor: cofactor.max(1e-9) }
    }
}

impl Transform for Arcsinh {
    fn forward(&self, x: f64) -> f64 {
        (x / self.cofactor).asinh()
    }
    fn inverse(&self, y: f64) -> f64 {
        y.sinh() * self.cofactor
    }
    fn name(&self) -> &'static str {
        "arcsinh"
    }
}

/// GatingML log: `log10(x / T) / M + 1`, defined for `x > 0` only (`x == 0` -> -inf, `x < 0` -> NaN,
/// the same as numpy). Maps `T` to 1 and `T * 10^-M` to 0.
#[derive(Debug, Clone, Copy)]
pub struct Log {
    pub t: f64,
    pub m: f64,
}

impl Log {
    pub fn new(t: f64, m: f64) -> Result<Log, String> {
        if !(t > 0.0 && m > 0.0) || !t.is_finite() || !m.is_finite() {
            return Err(format!("invalid log parameters T={t} M={m}"));
        }
        Ok(Log { t, m })
    }
}

impl Transform for Log {
    fn forward(&self, x: f64) -> f64 {
        (x / self.t).log10() / self.m + 1.0
    }
    fn inverse(&self, y: f64) -> f64 {
        self.t * 10f64.powf(self.m * (y - 1.0))
    }
    fn name(&self) -> &'static str {
        "log"
    }
}

/// Apply a transform in place to one column of a row-major `f32` event matrix.
pub fn apply_column(t: &dyn Transform, data: &mut [f32], n_cols: usize, col: usize) {
    for row in data.chunks_exact_mut(n_cols) {
        row[col] = t.forward(row[col] as f64) as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcsinh_and_its_inverse() {
        let t = Arcsinh::new(150.0);
        assert!((t.forward(0.0)).abs() < 1e-15);
        assert!((t.forward(150.0) - 1f64.asinh()).abs() < 1e-15);
        for x in [-1e5, -150.0, 0.0, 3.0, 1e6] {
            assert!((t.inverse(t.forward(x)) - x).abs() <= 1e-9 * x.abs().max(1.0));
        }
        assert_eq!(Arcsinh::new(0.0).cofactor, 1e-9); // the engine's floor
    }

    #[test]
    fn log_defining_points_and_domain() {
        let t = Log::new(262144.0, 4.5).unwrap();
        assert!((t.forward(262144.0) - 1.0).abs() < 1e-15);
        assert!(t.forward(262144.0 * 10f64.powf(-4.5)).abs() < 1e-12);
        assert_eq!(t.forward(0.0), f64::NEG_INFINITY);
        assert!(t.forward(-1.0).is_nan());
        assert!((t.inverse(t.forward(1234.5)) - 1234.5).abs() < 1e-9);
        assert!(Log::new(0.0, 4.5).is_err());
    }

    #[test]
    fn apply_column_touches_only_that_column() {
        let mut d = vec![1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        apply_column(&Arcsinh::new(1.0), &mut d, 3, 1);
        assert_eq!(d[0], 1.0);
        assert_eq!(d[2], 3.0);
        assert_eq!(d[3], 4.0);
        assert!((d[1] as f64 - 2f64.asinh()).abs() < 1e-6);
        assert!((d[4] as f64 - 5f64.asinh()).abs() < 1e-6);
    }
}
