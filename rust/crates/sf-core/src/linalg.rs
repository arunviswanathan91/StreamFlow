//! Small dense linear algebra (row-major `Vec<f64>`), enough for compensation and unmixing.
//!
//! Numerical policy: inversion uses Gauss-Jordan with **partial pivoting** and refuses a numerically
//! singular matrix with an error rather than returning garbage; least squares uses **Householder
//! QR** (not normal equations, which square the condition number).

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum LinAlgError {
    /// The matrix has no usable inverse; `pivot` is the largest remaining pivot magnitude and
    /// `scale` the largest entry, so callers can report how close to singular it was.
    Singular { pivot: f64, scale: f64 },
    Shape(String),
}

impl fmt::Display for LinAlgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinAlgError::Singular { pivot, scale } => write!(
                f,
                "matrix is singular or nearly so (largest pivot {pivot:e} against scale {scale:e}); \
                 two rows are (almost) linearly dependent"
            ),
            LinAlgError::Shape(m) => write!(f, "shape error: {m}"),
        }
    }
}

impl std::error::Error for LinAlgError {}

/// Inverse of an `n x n` row-major matrix.
pub fn invert(a: &[f64], n: usize) -> Result<Vec<f64>, LinAlgError> {
    if a.len() != n * n {
        return Err(LinAlgError::Shape(format!("expected {n}x{n} = {} values, got {}", n * n, a.len())));
    }
    let scale = a.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mut m = a.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    // singular when a pivot falls below this (relative to the largest entry)
    let tol = scale * n as f64 * f64::EPSILON;
    for col in 0..n {
        let (mut p, mut best) = (col, m[col * n + col].abs());
        for r in col + 1..n {
            if m[r * n + col].abs() > best {
                best = m[r * n + col].abs();
                p = r;
            }
        }
        if best <= tol || !best.is_finite() {
            return Err(LinAlgError::Singular { pivot: best, scale });
        }
        if p != col {
            for k in 0..n {
                m.swap(col * n + k, p * n + k);
                inv.swap(col * n + k, p * n + k);
            }
        }
        let d = m[col * n + col];
        for k in 0..n {
            m[col * n + k] /= d;
            inv[col * n + k] /= d;
        }
        for r in 0..n {
            if r != col {
                let f = m[r * n + col];
                if f != 0.0 {
                    for k in 0..n {
                        m[r * n + k] -= f * m[col * n + k];
                        inv[r * n + k] -= f * inv[col * n + k];
                    }
                }
            }
        }
    }
    Ok(inv)
}

/// Solve `A X = B` for `A (n x n)` and `B (n x k)` (both row-major) by Gaussian elimination with
/// partial pivoting. Preferred over `invert(A) @ B`: no explicit inverse is formed.
pub fn solve(a: &[f64], n: usize, b: &[f64], k: usize) -> Result<Vec<f64>, LinAlgError> {
    if a.len() != n * n || b.len() != n * k {
        return Err(LinAlgError::Shape(format!("solve needs A {n}x{n} and B {n}x{k}")));
    }
    let scale = a.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let tol = scale * n as f64 * f64::EPSILON;
    let (mut m, mut x) = (a.to_vec(), b.to_vec());
    for col in 0..n {
        let (mut p, mut best) = (col, m[col * n + col].abs());
        for r in col + 1..n {
            if m[r * n + col].abs() > best {
                best = m[r * n + col].abs();
                p = r;
            }
        }
        if best <= tol || !best.is_finite() {
            return Err(LinAlgError::Singular { pivot: best, scale });
        }
        if p != col {
            for j in 0..n {
                m.swap(col * n + j, p * n + j);
            }
            for j in 0..k {
                x.swap(col * k + j, p * k + j);
            }
        }
        for r in col + 1..n {
            let f = m[r * n + col] / m[col * n + col];
            if f != 0.0 {
                for j in col..n {
                    m[r * n + j] -= f * m[col * n + j];
                }
                for j in 0..k {
                    x[r * k + j] -= f * x[col * k + j];
                }
            }
        }
    }
    for col in (0..n).rev() {
        for j in 0..k {
            let mut s = x[col * k + j];
            for c in col + 1..n {
                s -= m[col * n + c] * x[c * k + j];
            }
            x[col * k + j] = s / m[col * n + col];
        }
    }
    Ok(x)
}

/// Eigen-decomposition of a symmetric `n x n` matrix by cyclic Jacobi rotations. Returns
/// `(values, vectors)` with values **descending**; eigenvector `i` is column `i` of the row-major
/// `vectors` matrix. Jacobi is slower than QR iteration but unconditionally accurate for the small
/// matrices used here (signature Gram matrices, detector covariances of a few dozen channels).
pub fn sym_eig(a: &[f64], n: usize) -> Result<(Vec<f64>, Vec<f64>), LinAlgError> {
    if a.len() != n * n {
        return Err(LinAlgError::Shape(format!("sym_eig needs {n}x{n}")));
    }
    let mut m = a.to_vec();
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    for _sweep in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |&j| j != i).map(move |j| (i, j))).map(|(i, j)| m[i * n + j] * m[i * n + j]).sum();
        let total: f64 = m.iter().map(|x| x * x).sum();
        if off <= 1e-30 * total.max(1e-300) {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = m[p * n + q];
                if apq.abs() < 1e-300 {
                    continue;
                }
                let theta = (m[q * n + q] - m[p * n + p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let (kp, kq) = (m[k * n + p], m[k * n + q]);
                    m[k * n + p] = c * kp - s * kq;
                    m[k * n + q] = s * kp + c * kq;
                }
                for k in 0..n {
                    let (pk, qk) = (m[p * n + k], m[q * n + k]);
                    m[p * n + k] = c * pk - s * qk;
                    m[q * n + k] = s * pk + c * qk;
                }
                for k in 0..n {
                    let (kp, kq) = (v[k * n + p], v[k * n + q]);
                    v[k * n + p] = c * kp - s * kq;
                    v[k * n + q] = s * kp + c * kq;
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&i, &j| m[j * n + j].total_cmp(&m[i * n + i]));
    let values = order.iter().map(|&i| m[i * n + i]).collect();
    let mut vectors = vec![0.0; n * n];
    for (new, &old) in order.iter().enumerate() {
        for k in 0..n {
            vectors[k * n + new] = v[k * n + old];
        }
    }
    Ok((values, vectors))
}

/// Singular values (descending) of an `m x n` row-major matrix by one-sided Jacobi (Hestenes).
/// Each singular value is accurate to relative `eps` - unlike eigenvalues of `A^T A`, which square
/// the condition number and cannot tell 1e-9 from exactly singular.
pub fn singular_values(a: &[f64], m: usize, n: usize) -> Result<Vec<f64>, LinAlgError> {
    if a.len() != m * n {
        return Err(LinAlgError::Shape(format!("singular_values needs {m}x{n}")));
    }
    // work on columns: store as n column vectors of length m
    let mut cols: Vec<Vec<f64>> = (0..n).map(|j| (0..m).map(|i| a[i * n + j]).collect()).collect();
    for _sweep in 0..60 {
        let mut rotated = false;
        for p in 0..n {
            for q in p + 1..n {
                let (mut alpha, mut beta, mut gamma) = (0.0, 0.0, 0.0);
                for i in 0..m {
                    alpha += cols[p][i] * cols[p][i];
                    beta += cols[q][i] * cols[q][i];
                    gamma += cols[p][i] * cols[q][i];
                }
                if gamma.abs() <= 1e-15 * (alpha * beta).sqrt() || gamma == 0.0 {
                    continue;
                }
                rotated = true;
                let zeta = (beta - alpha) / (2.0 * gamma);
                let t = zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt());
                let t = if zeta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (1.0 + t * t).sqrt();
                let s = c * t;
                for i in 0..m {
                    let (x, y) = (cols[p][i], cols[q][i]);
                    cols[p][i] = c * x - s * y;
                    cols[q][i] = s * x + c * y;
                }
            }
        }
        if !rotated {
            break;
        }
    }
    let mut sv: Vec<f64> = cols.iter().map(|c| c.iter().map(|x| x * x).sum::<f64>().sqrt()).collect();
    sv.sort_by(|a, b| b.total_cmp(a));
    sv.truncate(m.min(n)); // a wide matrix has only min(m, n) singular values
    Ok(sv)
}

/// `y (1 x n) @ m (n x n)` -> `1 x n`.
pub fn row_times_matrix(y: &[f64], m: &[f64], n: usize) -> Vec<f64> {
    let mut out = vec![0.0; n];
    for (i, yi) in y.iter().enumerate() {
        if *yi != 0.0 {
            for j in 0..n {
                out[j] += yi * m[i * n + j];
            }
        }
    }
    out
}

/// `A (m x n) @ B (n x p)` -> `m x p`.
pub fn matmul(a: &[f64], b: &[f64], m: usize, n: usize, p: usize) -> Vec<f64> {
    let mut out = vec![0.0; m * p];
    for i in 0..m {
        for k in 0..n {
            let aik = a[i * n + k];
            if aik != 0.0 {
                for j in 0..p {
                    out[i * p + j] += aik * b[k * p + j];
                }
            }
        }
    }
    out
}

pub fn transpose(a: &[f64], m: usize, n: usize) -> Vec<f64> {
    let mut t = vec![0.0; m * n];
    for i in 0..m {
        for j in 0..n {
            t[j * m + i] = a[i * n + j];
        }
    }
    t
}

/// Least squares `min ||A x - b||` for `A (m x n)`, `m >= n`, full column rank, via Householder QR.
pub fn lstsq(a: &[f64], m: usize, n: usize, b: &[f64]) -> Result<Vec<f64>, LinAlgError> {
    if a.len() != m * n || b.len() != m || m < n {
        return Err(LinAlgError::Shape(format!("lstsq needs A {m}x{n} with m >= n and b of length {m}")));
    }
    let mut r = a.to_vec();
    let mut qtb = b.to_vec();
    let scale = a.iter().fold(0.0f64, |s, v| s.max(v.abs()));
    for k in 0..n {
        let norm = (k..m).map(|i| r[i * n + k] * r[i * n + k]).sum::<f64>().sqrt();
        if norm <= scale * m as f64 * f64::EPSILON {
            return Err(LinAlgError::Singular { pivot: norm, scale });
        }
        let alpha = if r[k * n + k] > 0.0 { -norm } else { norm };
        // Householder vector v = x - alpha e1, stored in a temp
        let mut v: Vec<f64> = (k..m).map(|i| r[i * n + k]).collect();
        v[0] -= alpha;
        let vnorm2: f64 = v.iter().map(|x| x * x).sum();
        if vnorm2 > 0.0 {
            for j in k..n {
                let dot: f64 = (k..m).map(|i| v[i - k] * r[i * n + j]).sum();
                let f = 2.0 * dot / vnorm2;
                for i in k..m {
                    r[i * n + j] -= f * v[i - k];
                }
            }
            let dot: f64 = (k..m).map(|i| v[i - k] * qtb[i]).sum();
            let f = 2.0 * dot / vnorm2;
            for i in k..m {
                qtb[i] -= f * v[i - k];
            }
        }
    }
    // back substitution on the leading n x n upper triangle
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut s = qtb[i];
        for j in i + 1..n {
            s -= r[i * n + j] * x[j];
        }
        x[i] = s / r[i * n + i];
    }
    Ok(x)
}

/// Non-negative least squares, `min ||A x - b||` subject to `x >= 0` (Lawson-Hanson active set).
/// `A` is `m x n` row-major. Same problem and tolerance convention as `scipy.optimize.nnls`.
pub fn nnls(a: &[f64], m: usize, n: usize, b: &[f64]) -> Result<Vec<f64>, LinAlgError> {
    if a.len() != m * n || b.len() != m {
        return Err(LinAlgError::Shape(format!("nnls needs A {m}x{n} and b of length {m}")));
    }
    let at = transpose(a, m, n);
    let col_norm1 = (0..n).map(|j| (0..m).map(|i| a[i * n + j].abs()).sum::<f64>()).fold(0.0f64, f64::max);
    let tol = 10.0 * f64::EPSILON * m.max(n) as f64 * col_norm1;
    let mut x = vec![0.0; n];
    let mut passive = vec![false; n];
    let gradient = |x: &[f64]| -> Vec<f64> {
        let resid: Vec<f64> = (0..m).map(|i| b[i] - (0..n).map(|j| a[i * n + j] * x[j]).sum::<f64>()).collect();
        (0..n).map(|j| (0..m).map(|i| at[j * m + i] * resid[i]).sum()).collect()
    };
    let solve_passive = |passive: &[bool]| -> Result<Vec<f64>, LinAlgError> {
        let idx: Vec<usize> = (0..n).filter(|&j| passive[j]).collect();
        let sub: Vec<f64> = (0..m).flat_map(|i| idx.iter().map(move |&j| (i, j))).map(|(i, j)| a[i * n + j]).collect();
        let s = lstsq(&sub, m, idx.len(), b)?;
        let mut full = vec![0.0; n];
        for (k, &j) in idx.iter().enumerate() {
            full[j] = s[k];
        }
        Ok(full)
    };
    for _ in 0..(3 * n + 10) {
        let w = gradient(&x);
        let cand = (0..n).filter(|&j| !passive[j]).max_by(|&p, &q| w[p].total_cmp(&w[q]));
        match cand {
            Some(t) if w[t] > tol => passive[t] = true,
            _ => break,
        }
        loop {
            let s = solve_passive(&passive)?;
            if (0..n).filter(|&j| passive[j]).all(|j| s[j] > 0.0) {
                x = s;
                break;
            }
            let mut alpha = f64::INFINITY;
            for j in (0..n).filter(|&j| passive[j] && s[j] <= 0.0) {
                alpha = alpha.min(x[j] / (x[j] - s[j]));
            }
            for j in 0..n {
                x[j] += alpha * (s[j] - x[j]);
            }
            for j in 0..n {
                if passive[j] && x[j].abs() <= tol {
                    passive[j] = false;
                    x[j] = 0.0;
                }
            }
        }
    }
    Ok(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: &[f64], b: &[f64], eps: f64) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() <= eps, "{x} != {y}");
        }
    }

    #[test]
    fn inverse_times_matrix_is_identity() {
        let a = [4.0, 7.0, 2.0, 1.0, 3.0, 5.0, 6.0, 0.5, 9.0];
        let inv = invert(&a, 3).unwrap();
        near(&matmul(&a, &inv, 3, 3, 3), &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], 1e-12);
    }

    #[test]
    fn pivoting_handles_a_zero_leading_entry() {
        let inv = invert(&[0.0, 1.0, 1.0, 0.0], 2).unwrap();
        near(&inv, &[0.0, 1.0, 1.0, 0.0], 1e-15);
    }

    #[test]
    fn singular_and_misshapen_inputs_are_errors_not_garbage() {
        assert!(matches!(invert(&[1.0, 2.0, 2.0, 4.0], 2), Err(LinAlgError::Singular { .. })));
        assert!(matches!(invert(&[1.0, 2.0, 3.0], 2), Err(LinAlgError::Shape(_))));
        assert!(invert(&[f64::NAN, 0.0, 0.0, 1.0], 2).is_err());
    }

    #[test]
    fn solve_matches_invert_and_handles_multiple_right_hand_sides() {
        let a = [4.0, 7.0, 2.0, 1.0, 3.0, 5.0, 6.0, 0.5, 9.0];
        let b = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]; // 3x2
        let x = solve(&a, 3, &b, 2).unwrap();
        near(&matmul(&a, &x, 3, 3, 2), &b, 1e-12);
        near(&x, &matmul(&invert(&a, 3).unwrap(), &b, 3, 3, 2), 1e-12);
        assert!(matches!(solve(&[1.0, 2.0, 2.0, 4.0], 2, &[1.0, 1.0], 1), Err(LinAlgError::Singular { .. })));
    }

    #[test]
    fn jacobi_eigen_satisfies_a_v_equals_lambda_v_and_sorts_descending() {
        let a = [4.0, 1.0, 0.5, 1.0, 3.0, 0.2, 0.5, 0.2, 1.0];
        let (vals, vecs) = sym_eig(&a, 3).unwrap();
        assert!(vals[0] >= vals[1] && vals[1] >= vals[2]);
        for c in 0..3 {
            let v: Vec<f64> = (0..3).map(|r| vecs[r * 3 + c]).collect();
            let av = matmul(&a, &v, 3, 3, 1);
            for r in 0..3 {
                assert!((av[r] - vals[c] * v[r]).abs() < 1e-12, "A v != lambda v for eigenpair {c}");
            }
            assert!((v.iter().map(|x| x * x).sum::<f64>() - 1.0).abs() < 1e-12, "unit norm");
        }
        assert!((vals.iter().sum::<f64>() - 8.0).abs() < 1e-12, "trace preserved");
    }

    #[test]
    fn singular_values_of_known_matrices_including_a_rank_deficient_one() {
        // diag(3, 2, 1) rotated is still {3, 2, 1}; and a rank-1 matrix has exactly one nonzero value
        let sv = singular_values(&[3.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0], 3, 3).unwrap();
        near(&sv, &[3.0, 2.0, 1.0], 1e-14);
        let rank1 = singular_values(&[1.0, 2.0, 3.0, 2.0, 4.0, 6.0], 2, 3).unwrap();
        assert!((rank1[0] - (5.0f64 * 14.0).sqrt()).abs() < 1e-12, "{rank1:?}");
        assert!(rank1[1] < 1e-14 * rank1[0], "second singular value should be ~0, got {:e}", rank1[1]);
        // wide and tall give the same values
        let wide = singular_values(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3).unwrap();
        let tall = singular_values(&transpose(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 2, 3), 3, 2).unwrap();
        near(&wide, &tall, 1e-13);
    }

    #[test]
    fn lstsq_recovers_an_exact_and_an_overdetermined_fit() {
        // exact: x = (2, -1)
        let a = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let x = lstsq(&a, 3, 2, &[0.0, 2.0, 4.0]).unwrap();
        near(&x, &[2.0, -1.0], 1e-12);
        // noisy line through (0,1),(1,2.1),(2,2.9),(3,4.2), hand-derived: Sxy = 5.2, Sxx = 5,
        // slope = 1.04, intercept = mean(y) - slope * mean(x) = 2.55 - 1.56 = 0.99
        let a2 = [1.0, 0.0, 1.0, 1.0, 1.0, 2.0, 1.0, 3.0];
        let x2 = lstsq(&a2, 4, 2, &[1.0, 2.1, 2.9, 4.2]).unwrap();
        near(&x2, &[0.99, 1.04], 1e-12);
    }

    #[test]
    fn nnls_matches_the_unconstrained_answer_when_it_is_positive() {
        let a = [1.0, 0.0, 0.0, 1.0];
        near(&nnls(&a, 2, 2, &[3.0, 4.0]).unwrap(), &[3.0, 4.0], 1e-12);
    }

    #[test]
    fn nnls_clamps_a_negative_component_to_zero() {
        let a = [1.0, 0.0, 0.0, 1.0];
        near(&nnls(&a, 2, 2, &[3.0, -4.0]).unwrap(), &[3.0, 0.0], 1e-12);
        near(&nnls(&a, 2, 2, &[0.0, 0.0]).unwrap(), &[0.0, 0.0], 1e-12);
    }
}
