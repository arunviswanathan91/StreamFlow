//! Fluorescence compensation.
//!
//! Convention (verified against FlowKit on real events): the spillover matrix `S` has one row per
//! fluorophore and one column per detector, and
//!
//! ```text
//! compensated = raw @ inv(S)        (row vector per event, detector columns only)
//! ```
//!
//! Columns that are not detectors in the matrix are left untouched. The optional non-negative mode
//! solves `S^T x = y, x >= 0` per event (the engine's `nnls` mode), so a compensated value can never
//! be physically negative; it is slower and biases dim populations, which is why it is opt-in.

use crate::linalg::{self, LinAlgError};
use std::fmt;

#[derive(Debug)]
pub enum CompError {
    LinAlg(LinAlgError),
    Shape(String),
}

impl fmt::Display for CompError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompError::LinAlg(e) => write!(f, "cannot compensate: {e}"),
            CompError::Shape(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for CompError {}

impl From<LinAlgError> for CompError {
    fn from(e: LinAlgError) -> Self {
        CompError::LinAlg(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Standard,
    NonNegative,
}

#[derive(Debug, Clone)]
pub struct Compensation {
    pub detectors: Vec<String>,
    spill: Vec<f64>,
    inverse: Vec<f64>,
}

impl Compensation {
    /// `spill` is `n x n` row-major with `n == detectors.len()`.
    pub fn new(detectors: Vec<String>, spill: Vec<f64>) -> Result<Compensation, CompError> {
        let n = detectors.len();
        if n == 0 || spill.len() != n * n {
            return Err(CompError::Shape(format!(
                "spillover must be square over the detectors: {} detectors but {} values",
                n,
                spill.len()
            )));
        }
        if spill.iter().any(|v| !v.is_finite()) {
            return Err(CompError::Shape("spillover contains NaN or infinite values".into()));
        }
        let inverse = linalg::invert(&spill, n)?;
        Ok(Compensation { detectors, spill, inverse })
    }

    pub fn n(&self) -> usize {
        self.detectors.len()
    }

    /// Compensate one event's detector values (`y.len() == n`).
    pub fn row(&self, y: &[f64], mode: Mode) -> Result<Vec<f64>, CompError> {
        let n = self.n();
        if y.len() != n {
            return Err(CompError::Shape(format!("expected {n} detector values, got {}", y.len())));
        }
        Ok(match mode {
            Mode::Standard => linalg::row_times_matrix(y, &self.inverse, n),
            // y = x S  <=>  S^T x^T = y^T
            Mode::NonNegative => linalg::nnls(&linalg::transpose(&self.spill, n, n), n, n, y)?,
        })
    }

    /// Compensate a row-major `f32` event matrix in place. `cols[i]` is the column holding
    /// `detectors[i]`; every other column is left exactly as it was.
    pub fn apply(&self, events: &mut [f32], n_cols: usize, cols: &[usize], mode: Mode) -> Result<(), CompError> {
        let n = self.n();
        if cols.len() != n {
            return Err(CompError::Shape(format!("{} column indices for {n} detectors", cols.len())));
        }
        if let Some(&bad) = cols.iter().find(|&&c| c >= n_cols) {
            return Err(CompError::Shape(format!("column index {bad} out of range for {n_cols} columns")));
        }
        let mut y = vec![0.0f64; n];
        for ev in events.chunks_exact_mut(n_cols) {
            for (k, &c) in cols.iter().enumerate() {
                y[k] = ev[c] as f64;
            }
            let x = self.row(&y, mode)?;
            for (k, &c) in cols.iter().enumerate() {
                ev[c] = x[k] as f32;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp() -> Compensation {
        let names = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
        Compensation::new(names, vec![1.0, 0.2, 0.0, 0.1, 1.0, 0.3, 0.0, 0.05, 1.0]).unwrap()
    }

    #[test]
    fn identity_spillover_changes_nothing() {
        let names = vec!["A".to_string(), "B".to_string()];
        let c = Compensation::new(names, vec![1.0, 0.0, 0.0, 1.0]).unwrap();
        assert_eq!(c.row(&[3.0, -4.0], Mode::Standard).unwrap(), vec![3.0, -4.0]);
    }

    #[test]
    fn compensating_a_pure_single_stain_returns_a_pure_signal() {
        // an event that is 100 units of fluorophore B alone looks like 100 * (row B of S)
        let c = comp();
        let raw = [100.0 * 0.1, 100.0 * 1.0, 100.0 * 0.3];
        let x = c.row(&raw, Mode::Standard).unwrap();
        assert!(x[0].abs() < 1e-9 && x[2].abs() < 1e-9 && (x[1] - 100.0).abs() < 1e-9, "{x:?}");
    }

    #[test]
    fn apply_leaves_non_detector_columns_untouched() {
        // columns: [FSC, A, X, B, C]; detectors live in columns 1, 3, 4
        let mut ev = vec![7.0f32, 10.0, 99.0, 100.0, 30.0];
        comp().apply(&mut ev, 5, &[1, 3, 4], Mode::Standard).unwrap();
        assert_eq!(ev[0], 7.0);
        assert_eq!(ev[2], 99.0);
        assert!((ev[3] - 100.0).abs() < 1e-3 && (ev[1] - 0.0).abs() < 1e-3 && (ev[4] - 0.0).abs() < 1e-3);
    }

    #[test]
    fn non_negative_mode_never_returns_a_negative_value() {
        let c = comp();
        let x = c.row(&[-50.0, 200.0, 5.0], Mode::NonNegative).unwrap();
        assert!(x.iter().all(|v| *v >= 0.0), "{x:?}");
        // and agrees with standard mode when the standard answer is already non-negative
        let y = [10.0, 100.0, 40.0];
        let (s, n) = (c.row(&y, Mode::Standard).unwrap(), c.row(&y, Mode::NonNegative).unwrap());
        if s.iter().all(|v| *v >= 0.0) {
            for (a, b) in s.iter().zip(&n) {
                assert!((a - b).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn bad_matrices_are_errors_with_messages() {
        let n2 = || vec!["A".to_string(), "B".to_string()];
        let singular = Compensation::new(n2(), vec![1.0, 2.0, 2.0, 4.0]).unwrap_err();
        assert!(singular.to_string().contains("singular"), "{singular}");
        assert!(Compensation::new(n2(), vec![1.0, 0.0, 0.0]).is_err());
        assert!(Compensation::new(n2(), vec![1.0, f64::NAN, 0.0, 1.0]).is_err());
        let mut ev = vec![1.0f32, 2.0];
        assert!(comp().apply(&mut ev, 2, &[0, 1], Mode::Standard).is_err()); // 2 cols for 3 detectors
    }
}
