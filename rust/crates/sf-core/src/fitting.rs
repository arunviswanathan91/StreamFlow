//! Numerical fitting: the error function and box-constrained non-linear least squares (the role
//! `scipy.optimize.curve_fit(..., bounds=...)` plays in the cell-cycle and proliferation fits).
//!
//! The fitter is a Levenberg-Marquardt iteration with a numerical Jacobian and projection onto the bounds
//! (variables pinned at a bound whose gradient pushes outward are held for that step). It minimises the same
//! sum of squared residuals as SciPy's trust-region-reflective method, so both reach the same minimum on a
//! well-posed problem; the tests compare the fitted curves, not iteration paths.

use crate::linalg::solve;

/// `erf(x)` to ~1e-15: Maclaurin series for small |x|, continued fraction for the complement otherwise.
pub fn erf(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    let a = x.abs();
    if a > 6.0 {
        return x.signum();
    }
    if a < 2.5 {
        // erf(x) = 2/sqrt(pi) * sum_{n>=0} (-1)^n x^(2n+1) / (n! (2n+1))
        let mut term = a;
        let mut sum = a;
        let x2 = a * a;
        for n in 1..200 {
            term *= -x2 / n as f64;
            let add = term / (2 * n + 1) as f64;
            sum += add;
            if add.abs() < 1e-17 * sum.abs() {
                break;
            }
        }
        return x.signum() * sum * 2.0 / std::f64::consts::PI.sqrt();
    }
    // erfc(a) = exp(-a^2)/(a sqrt(pi)) * 1/(1+ 1/(2a^2)/(1+2/(2a^2)/(1+3/(2a^2)/...))) via Lentz on the CF
    let mut f = 0.0f64;
    for k in (1..=60).rev() {
        f = (k as f64 / 2.0) / (a + f);
    }
    let erfc = (-a * a).exp() / ((a + f) * std::f64::consts::PI.sqrt());
    x.signum() * (1.0 - erfc)
}

#[derive(Debug, Clone)]
pub struct Fit {
    pub params: Vec<f64>,
    /// Sum of squared residuals at the solution.
    pub cost: f64,
    pub iterations: usize,
    pub converged: bool,
}

/// Fit `model(params, xs, out)` (which must fill `out` with the predicted `y` for every `x`) to `ys`, keeping
/// every parameter inside `[lb, ub]`. `p0` is clamped into the box.
pub fn curve_fit_bounded(model: &dyn Fn(&[f64], &[f64], &mut [f64]), xs: &[f64], ys: &[f64], p0: &[f64], lb: &[f64], ub: &[f64], max_iter: usize) -> Result<Fit, String> {
    let (n, m) = (xs.len(), p0.len());
    if ys.len() != n || lb.len() != m || ub.len() != m || n < m {
        return Err("curve_fit: inconsistent sizes".into());
    }
    if (0..m).any(|i| !(ub[i] > lb[i])) {
        return Err("curve_fit: every lower bound must be below its upper bound".into());
    }
    let clamp = |p: &mut [f64]| {
        for i in 0..m {
            p[i] = p[i].clamp(lb[i], ub[i]);
        }
    };
    let mut p = p0.to_vec();
    clamp(&mut p);
    let mut pred = vec![0.0; n];
    let cost_of = |p: &[f64], pred: &mut [f64]| {
        model(p, xs, pred);
        pred.iter().zip(ys).map(|(a, b)| (a - b) * (a - b)).sum::<f64>()
    };
    let mut cost = cost_of(&p, &mut pred);
    if !cost.is_finite() {
        return Err("curve_fit: the model is not finite at the starting point".into());
    }
    let mut lambda = 1e-3;
    let mut converged = false;
    let mut it = 0;
    let (mut p_plus, mut p_minus) = (vec![0.0; n], vec![0.0; n]);
    while it < max_iter {
        it += 1;
        // residuals and numerical Jacobian (central differences, one-sided at a bound)
        let r: Vec<f64> = pred.iter().zip(ys).map(|(a, b)| a - b).collect();
        let mut jac = vec![0.0; n * m];
        for k in 0..m {
            let h = 1e-6 * p[k].abs().max(1e-3 * (ub[k] - lb[k]).min(1.0).max(1e-8));
            let (mut hi, mut lo) = (p.clone(), p.clone());
            hi[k] = (p[k] + h).min(ub[k]);
            lo[k] = (p[k] - h).max(lb[k]);
            let span = hi[k] - lo[k];
            if span <= 0.0 {
                continue;
            }
            model(&hi, xs, &mut p_plus);
            model(&lo, xs, &mut p_minus);
            for i in 0..n {
                jac[i * m + k] = (p_plus[i] - p_minus[i]) / span;
            }
        }
        let g: Vec<f64> = (0..m).map(|k| (0..n).map(|i| jac[i * m + k] * r[i]).sum()).collect();
        let mut h = vec![0.0; m * m];
        for a in 0..m {
            for b in a..m {
                let v: f64 = (0..n).map(|i| jac[i * m + a] * jac[i * m + b]).sum();
                h[a * m + b] = v;
                h[b * m + a] = v;
            }
        }
        // variables held at a bound that the gradient pushes outward stay fixed for this step
        let free: Vec<usize> = (0..m).filter(|&k| !((p[k] <= lb[k] && g[k] > 0.0) || (p[k] >= ub[k] && g[k] < 0.0))).collect();
        if free.is_empty() {
            converged = true;
            break;
        }
        let gnorm = free.iter().map(|&k| g[k].abs()).fold(0.0, f64::max);
        if gnorm < 1e-12 * cost.max(1.0) {
            converged = true;
            break;
        }
        let mut improved = false;
        for _ in 0..40 {
            let f = free.len();
            let mut a = vec![0.0; f * f];
            let mut b = vec![0.0; f];
            for (ia, &ka) in free.iter().enumerate() {
                for (ib, &kb) in free.iter().enumerate() {
                    a[ia * f + ib] = h[ka * m + kb];
                }
                a[ia * f + ia] += lambda * h[ka * m + ka].max(1e-12);
                b[ia] = -g[ka];
            }
            let Ok(step) = solve(&a, f, &b, 1) else {
                lambda *= 10.0;
                continue;
            };
            let mut trial = p.clone();
            for (ia, &ka) in free.iter().enumerate() {
                trial[ka] += step[ia];
            }
            clamp(&mut trial);
            let mut tp = vec![0.0; n];
            let c = cost_of(&trial, &mut tp);
            if c.is_finite() && c < cost {
                let rel = (cost - c) / cost.max(1e-300);
                let moved = trial.iter().zip(&p).map(|(a, b)| (a - b).abs() / (b.abs() + 1e-12)).fold(0.0, f64::max);
                p = trial;
                pred = tp;
                cost = c;
                lambda = (lambda / 3.0).max(1e-12);
                improved = true;
                if rel < 1e-13 || moved < 1e-12 {
                    converged = true;
                }
                break;
            }
            lambda *= 4.0;
            if lambda > 1e14 {
                break;
            }
        }
        if !improved {
            converged = true; // no downhill step exists at any damping: a (local) minimum
            break;
        }
        if converged {
            break;
        }
    }
    Ok(Fit { params: p, cost, iterations: it, converged })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erf_is_accurate_across_its_range() {
        // reference values from Python math.erf (the series and the continued fraction meet at 2.5)
        for (x, want) in [(0.0, 0.0), (0.1, 0.1124629160182849), (0.5, 0.5204998778130465), (1.0, 0.8427007929497149), (2.0, 0.9953222650189527), (2.4999, 0.9995928300996666), (2.5, 0.999593047982555), (3.0, 0.9999779095030014), (4.5, 0.9999999998033839), (-1.0, -0.8427007929497149)] {
            assert!((erf(x) - want).abs() < 2e-14 * want.abs().max(1.0) + 1e-15, "erf({x}) = {} vs {want}", erf(x));
        }
        assert_eq!(erf(7.0), 1.0);
        assert_eq!(erf(-7.0), -1.0);
        assert!(erf(f64::NAN).is_nan());
        // smooth across the switch between the series and the continued fraction at 2.5
        assert!((erf(2.4999999) - erf(2.5000001)).abs() < 1e-8);
    }

    #[test]
    fn a_gaussian_is_recovered_inside_its_bounds() {
        let xs: Vec<f64> = (0..120).map(|i| i as f64 * 0.1).collect();
        let truth = [50.0, 6.0, 0.9];
        let model = |p: &[f64], xs: &[f64], out: &mut [f64]| {
            for (o, &x) in out.iter_mut().zip(xs) {
                *o = p[0] * (-0.5 * ((x - p[1]) / p[2]).powi(2)).exp();
            }
        };
        let mut ys = vec![0.0; xs.len()];
        model(&truth, &xs, &mut ys);
        let fit = curve_fit_bounded(&model, &xs, &ys, &[20.0, 5.0, 2.0], &[0.0, 3.0, 0.1], &[200.0, 9.0, 4.0], 200).unwrap();
        for (a, b) in fit.params.iter().zip(&truth) {
            assert!((a - b).abs() < 1e-6, "{:?}", fit.params);
        }
        assert!(fit.cost < 1e-12 && fit.converged);
    }

    #[test]
    fn a_bound_is_respected_even_when_the_optimum_lies_beyond_it() {
        // true centre 8 but the box stops at 7: the fit must sit exactly on the bound
        let xs: Vec<f64> = (0..100).map(|i| i as f64 * 0.1).collect();
        let model = |p: &[f64], xs: &[f64], out: &mut [f64]| {
            for (o, &x) in out.iter_mut().zip(xs) {
                *o = 10.0 * (-0.5 * ((x - p[0]) / 1.0).powi(2)).exp();
            }
        };
        let mut ys = vec![0.0; xs.len()];
        model(&[8.0], &xs, &mut ys);
        let fit = curve_fit_bounded(&model, &xs, &ys, &[5.0], &[0.0], &[7.0], 100).unwrap();
        assert_eq!(fit.params[0], 7.0);
        assert!(fit.converged);
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let model = |_: &[f64], _: &[f64], out: &mut [f64]| out.fill(0.0);
        assert!(curve_fit_bounded(&model, &[1.0, 2.0], &[1.0], &[1.0], &[0.0], &[2.0], 10).is_err());
        assert!(curve_fit_bounded(&model, &[1.0, 2.0], &[1.0, 2.0], &[1.0], &[2.0], &[2.0], 10).is_err());
        let nan = |_: &[f64], _: &[f64], out: &mut [f64]| out.fill(f64::NAN);
        assert!(curve_fit_bounded(&nan, &[1.0, 2.0], &[1.0, 2.0], &[1.0], &[0.0], &[2.0], 10).is_err());
    }
}
