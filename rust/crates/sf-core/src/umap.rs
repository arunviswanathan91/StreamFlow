//! UMAP (McInnes, Healy & Melville 2018): a fuzzy neighbourhood graph on the data, then a force-directed
//! layout whose attraction/repulsion come from the cross-entropy between that graph and one defined on the
//! embedding by `q(d) = 1 / (1 + a d^(2b))`.
//!
//! 1. [`fuzzy_graph`]: for each point, `rho` = distance to its nearest neighbour and `sigma` solved so the
//!    memberships `exp(-(d - rho) / sigma)` over its neighbours sum to `log2(n_neighbors)`; the directed
//!    graph is symmetrised with the fuzzy union `a + b - a*b`. As in umap-learn, `n_neighbors` counts the
//!    point itself, so `n_neighbors - 1` real neighbours are used.
//! 2. [`find_ab`]: `a, b` from a least-squares fit of `q` to the target curve set by `min_dist` and `spread`.
//! 3. spectral initialisation (leading eigenvectors of the normalised graph by subspace iteration) so the
//!    layout starts with global structure, falling back to a seeded random start if that fails.
//! 4. stochastic gradient descent over edges sampled in proportion to their weight, with negative sampling
//!    for repulsion and a linearly decaying learning rate.
//!
//! The graph is deterministic given the neighbours and is checked against umap-learn exactly; the layout is
//! stochastic and is judged by neighbourhood preservation, not coordinates. densMAP is not implemented.

use crate::graphcluster::knn;
use crate::rng::Rng;

/// Fit `a, b` of `1 / (1 + a x^(2b))` to `1` for `x <= min_dist`, `exp(-(x - min_dist) / spread)` beyond,
/// on 300 points of `[0, 3 * spread]` (Levenberg-Marquardt from `(1, 1)`).
pub fn find_ab(min_dist: f64, spread: f64) -> Result<(f64, f64), String> {
    if !(min_dist >= 0.0 && spread > 0.0 && min_dist <= spread) || !min_dist.is_finite() || !spread.is_finite() {
        return Err(format!("need 0 <= min_dist <= spread (got {min_dist}, {spread})"));
    }
    let m = 300;
    let xs: Vec<f64> = (0..m).map(|i| 3.0 * spread * i as f64 / (m - 1) as f64).collect();
    let ys: Vec<f64> = xs.iter().map(|&x| if x <= min_dist { 1.0 } else { (-(x - min_dist) / spread).exp() }).collect();
    let (mut a, mut b, mut lambda) = (1.0f64, 1.0f64, 1e-3f64);
    let resid = |a: f64, b: f64| -> f64 { xs.iter().zip(&ys).map(|(&x, &y)| (1.0 / (1.0 + a * x.powf(2.0 * b)) - y).powi(2)).sum() };
    let mut cost = resid(a, b);
    for _ in 0..500 {
        let (mut jtj, mut jtr) = ([[0.0f64; 2]; 2], [0.0f64; 2]);
        for (&x, &y) in xs.iter().zip(&ys) {
            if x <= 0.0 {
                continue; // x^(2b) = 0 there: no gradient information (and ln x undefined)
            }
            let p = x.powf(2.0 * b);
            let f = 1.0 / (1.0 + a * p);
            let (da, db) = (-p * f * f, -a * p * 2.0 * x.ln() * f * f);
            let r = f - y;
            jtj[0][0] += da * da;
            jtj[0][1] += da * db;
            jtj[1][1] += db * db;
            jtr[0] += da * r;
            jtr[1] += db * r;
        }
        jtj[1][0] = jtj[0][1];
        let mut improved = false;
        for _ in 0..30 {
            let (h00, h11) = (jtj[0][0] * (1.0 + lambda) + 1e-300, jtj[1][1] * (1.0 + lambda) + 1e-300);
            let det = h00 * h11 - jtj[0][1] * jtj[1][0];
            if det.abs() < 1e-300 {
                lambda *= 10.0;
                continue;
            }
            let (sa, sb) = (-(h11 * jtr[0] - jtj[0][1] * jtr[1]) / det, -(-jtj[1][0] * jtr[0] + h00 * jtr[1]) / det);
            let (na, nb) = (a + sa, b + sb);
            if na > 0.0 && nb > 0.0 {
                let nc = resid(na, nb);
                if nc < cost {
                    let done = (cost - nc) < 1e-14 * cost.max(1e-30);
                    (a, b, cost, lambda) = (na, nb, nc, (lambda / 3.0).max(1e-12));
                    improved = true;
                    if done {
                        return Ok((a, b));
                    }
                    break;
                }
            }
            lambda *= 4.0;
        }
        if !improved {
            break;
        }
    }
    Ok((a, b))
}

#[derive(Debug, Clone)]
pub struct FuzzyGraph {
    pub n: usize,
    /// Symmetric fuzzy-union edges `(i, j, weight)` with `i < j`, weight in `(0, 1]`.
    pub edges: Vec<(u32, u32, f64)>,
    pub sigmas: Vec<f64>,
    pub rhos: Vec<f64>,
}

/// `rho` and `sigma` for one point from its (self-excluded) neighbour distances.
fn smooth_knn(dists: &[f64], target: f64) -> (f64, f64) {
    let rho = dists.iter().cloned().filter(|&d| d > 0.0).fold(f64::INFINITY, f64::min);
    let rho = if rho.is_finite() { rho } else { 0.0 };
    let (mut lo, mut hi, mut mid) = (0.0f64, f64::INFINITY, 1.0f64);
    for _ in 0..64 {
        let psum: f64 = dists
            .iter()
            .map(|&d| {
                let g = d - rho;
                if g > 0.0 {
                    (-g / mid).exp()
                } else {
                    1.0
                }
            })
            .sum();
        if (psum - target).abs() < 1e-5 {
            break;
        }
        if psum > target {
            hi = mid;
            mid = (lo + hi) / 2.0;
        } else {
            lo = mid;
            mid = if hi.is_infinite() { mid * 2.0 } else { (lo + hi) / 2.0 };
        }
    }
    (rho, mid)
}

/// The fuzzy simplicial set of the data for `n_neighbors` (which counts the point itself).
pub fn fuzzy_graph(data: &[f64], n: usize, d: usize, n_neighbors: usize) -> Result<FuzzyGraph, String> {
    if n_neighbors < 2 {
        return Err("n_neighbors must be at least 2".into());
    }
    let k = n_neighbors - 1;
    let nb = knn(data, n, d, k)?;
    let dist = |i: usize, j: usize| (0..d).map(|c| (data[i * d + c] - data[j * d + c]).powi(2)).sum::<f64>().sqrt();
    let target = (n_neighbors as f64).log2();
    let mut sigmas = vec![0.0; n];
    let mut rhos = vec![0.0; n];
    let mut directed: std::collections::HashMap<(u32, u32), f64> = std::collections::HashMap::with_capacity(n * k);
    let mut all_mean = 0.0;
    let mut row_dists: Vec<Vec<f64>> = Vec::with_capacity(n);
    for i in 0..n {
        let ds: Vec<f64> = nb[i * k..(i + 1) * k].iter().map(|&j| dist(i, j as usize)).collect();
        all_mean += ds.iter().sum::<f64>() / k as f64;
        row_dists.push(ds);
    }
    all_mean /= n as f64;
    for i in 0..n {
        let ds = &row_dists[i];
        let (rho, mut sigma) = smooth_knn(ds, target);
        let row_mean = ds.iter().sum::<f64>() / k as f64;
        // guard against a collapsing kernel (umap's MIN_K_DIST_SCALE = 1e-3)
        let floor = 1e-3 * if rho > 0.0 { row_mean } else { all_mean };
        if sigma < floor {
            sigma = floor;
        }
        rhos[i] = rho;
        sigmas[i] = sigma;
        for (&j, &dij) in nb[i * k..(i + 1) * k].iter().zip(ds) {
            let w = if dij - rho <= 0.0 || sigma == 0.0 { 1.0 } else { (-(dij - rho) / sigma).exp() };
            if w > 0.0 {
                directed.insert((i as u32, j), w);
            }
        }
    }
    let mut edges = Vec::new();
    for (&(i, j), &a) in &directed {
        if i < j {
            let b = directed.get(&(j, i)).copied().unwrap_or(0.0);
            edges.push((i, j, a + b - a * b));
        } else if !directed.contains_key(&(j, i)) {
            edges.push((j, i, a)); // one-way edge stored under its (smaller, larger) key
        }
    }
    edges.sort_by_key(|e| (e.0, e.1));
    Ok(FuzzyGraph { n, edges, sigmas, rhos })
}

#[derive(Debug, Clone)]
pub struct UmapParams {
    pub n_neighbors: usize,
    pub min_dist: f64,
    pub spread: f64,
    /// `None` = 500 for up to 10,000 points, else 200 (umap-learn's rule).
    pub n_epochs: Option<usize>,
    pub negative_sample_rate: usize,
    pub learning_rate: f64,
    pub seed: u64,
}

impl Default for UmapParams {
    fn default() -> Self {
        UmapParams { n_neighbors: 15, min_dist: 0.1, spread: 1.0, n_epochs: None, negative_sample_rate: 5, learning_rate: 1.0, seed: 42 }
    }
}

fn clip(v: f64) -> f64 {
    v.clamp(-4.0, 4.0)
}

/// Spectral initialisation: coordinates from the leading non-trivial eigenvectors of `D^-1/2 W D^-1/2`,
/// scaled to a `[0, 10]` box (umap-learn's convention) plus small noise. `None` when it cannot be computed.
fn spectral_init(g: &FuzzyGraph, rng: &mut Rng) -> Option<Vec<f64>> {
    let n = g.n;
    let mut adj: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n];
    for &(i, j, w) in &g.edges {
        adj[i as usize].push((j, w));
        adj[j as usize].push((i, w));
    }
    let deg: Vec<f64> = adj.iter().map(|a| a.iter().map(|e| e.1).sum()).collect();
    if deg.iter().any(|&d| d <= 0.0) || n < 4 {
        return None;
    }
    let isq: Vec<f64> = deg.iter().map(|d| 1.0 / d.sqrt()).collect();
    let dim = 3; // trivial eigenvector + 2 coordinates
    let mut v: Vec<Vec<f64>> = (0..dim).map(|_| (0..n).map(|_| rng.next_f64() - 0.5).collect()).collect();
    let orthonormalise = |v: &mut Vec<Vec<f64>>| {
        for a in 0..v.len() {
            for b in 0..a {
                let dot: f64 = v[a].iter().zip(&v[b]).map(|(x, y)| x * y).sum();
                let vb = v[b].clone();
                v[a].iter_mut().zip(&vb).for_each(|(x, y)| *x -= dot * y);
            }
            let norm = v[a].iter().map(|x| x * x).sum::<f64>().sqrt();
            if norm < 1e-300 {
                return false;
            }
            v[a].iter_mut().for_each(|x| *x /= norm);
        }
        true
    };
    if !orthonormalise(&mut v) {
        return None;
    }
    for _ in 0..300 {
        for vec in v.iter_mut() {
            // (M + I) / 2 has the same eigenvectors as M with a non-negative spectrum
            let mut out = vec![0.0; n];
            for i in 0..n {
                let mut s = 0.0;
                for &(j, w) in &adj[i] {
                    s += w * isq[i] * isq[j as usize] * vec[j as usize];
                }
                out[i] = 0.5 * (s + vec[i]);
            }
            *vec = out;
        }
        if !orthonormalise(&mut v) {
            return None;
        }
    }
    // drop the vector closest to the trivial eigenvector sqrt(deg)
    let triv: Vec<f64> = {
        let t: Vec<f64> = deg.iter().map(|d| d.sqrt()).collect();
        let nn = t.iter().map(|x| x * x).sum::<f64>().sqrt();
        t.iter().map(|x| x / nn).collect()
    };
    let align: Vec<f64> = v.iter().map(|vec| vec.iter().zip(&triv).map(|(x, y)| x * y).sum::<f64>().abs()).collect();
    let drop = align.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
    let keep: Vec<usize> = (0..dim).filter(|&c| c != drop).collect();
    let mut y = vec![0.0; n * 2];
    for (c, &src) in keep.iter().enumerate() {
        for i in 0..n {
            y[2 * i + c] = v[src][i] * isq[i];
        }
    }
    if y.iter().any(|x| !x.is_finite()) {
        return None;
    }
    let maxabs = y.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    if maxabs == 0.0 {
        return None;
    }
    let expand = 10.0 / maxabs;
    for val in y.iter_mut() {
        *val = *val * expand + 1e-4 * (rng.next_f64() - 0.5);
    }
    Some(y)
}

/// Attractive step size for an edge at squared embedding distance `d2` (zero at `d2 = 0`).
pub fn attraction_coeff(d2: f64, a: f64, b: f64) -> f64 {
    if d2 > 0.0 {
        -2.0 * a * b * d2.powf(b - 1.0) / (a * d2.powf(b) + 1.0)
    } else {
        0.0
    }
}

/// Repulsive step size for a negative sample at squared distance `d2`.
pub fn repulsion_coeff(d2: f64, a: f64, b: f64, gamma: f64) -> f64 {
    if d2 > 0.0 {
        2.0 * gamma * b / ((0.001 + d2) * (a * d2.powf(b) + 1.0))
    } else {
        0.0
    }
}

pub fn umap(data: &[f64], n: usize, d: usize, params: &UmapParams) -> Result<Vec<f64>, String> {
    if data.len() != n * d || d == 0 {
        return Err("UMAP input must be a non-empty n x d matrix".into());
    }
    if data.iter().any(|v| !v.is_finite()) {
        return Err("UMAP input contains non-finite values".into());
    }
    if params.n_neighbors >= n {
        return Err(format!("n_neighbors ({}) must be smaller than the number of points ({n})", params.n_neighbors));
    }
    let (a, b) = find_ab(params.min_dist, params.spread)?;
    let graph = fuzzy_graph(data, n, d, params.n_neighbors)?;
    let n_epochs = params.n_epochs.unwrap_or(if n <= 10_000 { 500 } else { 200 });
    let mut rng = Rng::new(params.seed);
    let mut y = spectral_init(&graph, &mut rng).unwrap_or_else(|| (0..n * 2).map(|_| (rng.next_f64() - 0.5) * 20.0).collect());

    // drop edges too weak to be sampled even once, then schedule the rest by weight
    let wmax = graph.edges.iter().map(|e| e.2).fold(0.0, f64::max);
    let edges: Vec<&(u32, u32, f64)> = graph.edges.iter().filter(|e| e.2 >= wmax / n_epochs as f64).collect();
    let eps: Vec<f64> = edges.iter().map(|e| wmax / e.2).collect(); // epochs per sample
    let mut next_pos: Vec<f64> = eps.clone();
    let neg_eps: Vec<f64> = eps.iter().map(|e| e / params.negative_sample_rate as f64).collect();
    let mut next_neg: Vec<f64> = neg_eps.clone();
    for epoch in 0..n_epochs {
        let alpha = params.learning_rate * (1.0 - epoch as f64 / n_epochs as f64);
        for (e, &&(i, j, _)) in edges.iter().enumerate() {
            if next_pos[e] > epoch as f64 {
                continue;
            }
            let (i, j) = (i as usize, j as usize);
            let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
            let c = attraction_coeff(dx * dx + dy * dy, a, b);
            let (gx, gy) = (clip(c * dx), clip(c * dy));
            y[2 * i] += gx * alpha;
            y[2 * i + 1] += gy * alpha;
            y[2 * j] -= gx * alpha;
            y[2 * j + 1] -= gy * alpha;
            next_pos[e] += eps[e];
            let n_neg = ((epoch as f64 - next_neg[e]) / neg_eps[e]).floor() + 1.0;
            for _ in 0..(n_neg.max(0.0) as usize) {
                let k = rng.below(n);
                if k == i {
                    continue;
                }
                let (dx, dy) = (y[2 * i] - y[2 * k], y[2 * i + 1] - y[2 * k + 1]);
                let c = repulsion_coeff(dx * dx + dy * dy, a, b, 1.0);
                let (gx, gy) = if c > 0.0 { (clip(c * dx), clip(c * dy)) } else { (4.0, 4.0) };
                y[2 * i] += gx * alpha;
                y[2 * i + 1] += gy * alpha;
            }
            next_neg[e] += n_neg.max(0.0) * neg_eps[e];
        }
    }
    if y.iter().any(|v| !v.is_finite()) {
        return Err("UMAP diverged (non-finite coordinates)".into());
    }
    Ok(y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blobs(per: usize, seed: u64) -> (Vec<f64>, Vec<usize>) {
        let centres = [[0.0, 0.0, 0.0, 0.0, 0.0], [8.0, 0.0, 8.0, 0.0, 0.0], [0.0, 8.0, 0.0, 8.0, 0.0]];
        let mut rng = Rng::new(seed);
        let (mut x, mut t) = (Vec::new(), Vec::new());
        for i in 0..per * 3 {
            let c = i % 3;
            for m in 0..5 {
                let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
                x.push(centres[c][m] + (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos());
            }
            t.push(c);
        }
        (x, t)
    }

    #[test]
    fn ab_fit_is_a_good_fit_and_matches_the_known_defaults() {
        let (a, b) = find_ab(0.1, 1.0).unwrap();
        // umap-learn's documented default fit: a ~ 1.577, b ~ 0.895
        assert!((a - 1.577).abs() < 0.01 && (b - 0.895).abs() < 0.01, "a={a} b={b}");
        // larger min_dist -> flatter core -> larger b
        let (_, b2) = find_ab(0.5, 1.0).unwrap();
        assert!(b2 > b);
        assert!(find_ab(2.0, 1.0).is_err() && find_ab(0.1, 0.0).is_err());
    }

    #[test]
    fn attraction_and_repulsion_are_the_gradients_of_their_cross_entropy_terms() {
        let (a, b) = find_ab(0.1, 1.0).unwrap();
        // attractive loss for a weight-1 edge: -log q(d) with q = 1/(1 + a d^2b); step = -dL/dy_i
        let loss_att = |dx: f64, dy: f64| (1.0 + a * (dx * dx + dy * dy).powf(b)).ln();
        let (dx, dy) = (1.3, -0.7);
        let c = attraction_coeff(dx * dx + dy * dy, a, b);
        let h = 1e-6;
        let fdx = -(loss_att(dx + h, dy) - loss_att(dx - h, dy)) / (2.0 * h);
        let fdy = -(loss_att(dx, dy + h) - loss_att(dx, dy - h)) / (2.0 * h);
        assert!((c * dx - fdx).abs() < 1e-6 && (c * dy - fdy).abs() < 1e-6, "{} vs {fdx}", c * dx);
        // repulsive loss for a weight-0 pair: -log(1 - q); the umap step adds a 0.001 regulariser to d^2,
        // negligible at larger separations
        let loss_rep = |dx: f64, dy: f64| -(1.0 - 1.0 / (1.0 + a * (dx * dx + dy * dy).powf(b))).ln();
        let (dx, dy) = (4.0, 3.0);
        let c = repulsion_coeff(dx * dx + dy * dy, a, b, 1.0);
        let fdx = -(loss_rep(dx + h, dy) - loss_rep(dx - h, dy)) / (2.0 * h);
        assert!((c * dx - fdx).abs() < 1e-3 * fdx.abs().max(1e-3), "{} vs {fdx}", c * dx);
        assert_eq!(attraction_coeff(0.0, a, b), 0.0);
        assert_eq!(repulsion_coeff(0.0, a, b, 1.0), 0.0);
    }

    #[test]
    fn smooth_knn_hits_the_target_and_rho_is_the_nearest_distance() {
        let ds: Vec<f64> = (1..=14).map(|i| i as f64 * 0.3).collect();
        let (rho, sigma) = smooth_knn(&ds, 15f64.log2());
        assert!((rho - 0.3).abs() < 1e-12);
        let psum: f64 = ds.iter().map(|&d| if d - rho > 0.0 { (-(d - rho) / sigma).exp() } else { 1.0 }).sum();
        assert!((psum - 15f64.log2()).abs() < 1e-4, "{psum}");
        // duplicates at distance 0 are ignored for rho
        assert_eq!(smooth_knn(&[0.0, 0.0, 2.0, 3.0], 2.0).0, 2.0);
    }

    #[test]
    fn graph_weights_are_valid_fuzzy_memberships_with_a_union_symmetrisation() {
        let (x, _) = blobs(40, 1);
        let g = fuzzy_graph(&x, 120, 5, 10).unwrap();
        assert!(g.edges.iter().all(|e| e.2 > 0.0 && e.2 <= 1.0 && e.0 < e.1));
        let mut seen = std::collections::HashSet::new();
        assert!(g.edges.iter().all(|e| seen.insert((e.0, e.1))), "no duplicate edges");
        // every point has at least its nearest neighbour at full membership (distance == rho gives weight 1)
        let mut best = vec![0.0f64; 120];
        for e in &g.edges {
            best[e.0 as usize] = best[e.0 as usize].max(e.2);
            best[e.1 as usize] = best[e.1 as usize].max(e.2);
        }
        assert!(best.iter().all(|&b| (b - 1.0).abs() < 1e-12));
        assert!(fuzzy_graph(&x, 120, 5, 1).is_err());
    }

    #[test]
    fn umap_separates_blobs_keeps_neighbourhoods_and_is_reproducible() {
        let (x, truth) = blobs(120, 5);
        let n = 360;
        let p = UmapParams { n_epochs: Some(300), ..Default::default() };
        let y = umap(&x, n, 5, &p).unwrap();
        assert!(y.iter().all(|v| v.is_finite()));
        let trust = crate::embedmetrics::trustworthiness(&x, 5, &y, 2, n, 12).unwrap();
        assert!(trust > 0.95, "trustworthiness {trust}");
        let km = crate::kmeans::kmeans(&y, n, 2, 3, 5, 100, 1).unwrap();
        assert!(crate::cluster::adjusted_rand_index(&km.labels, &truth) > 0.95);
        assert_eq!(y, umap(&x, n, 5, &p).unwrap(), "same seed, same layout");
    }

    #[test]
    fn bad_inputs_are_errors() {
        let (x, _) = blobs(10, 2);
        assert!(umap(&x, 30, 5, &UmapParams { n_neighbors: 30, ..Default::default() }).is_err());
        let mut bad = x.clone();
        bad[0] = f64::INFINITY;
        assert!(umap(&bad, 30, 5, &UmapParams::default()).is_err());
        assert!(umap(&x[..7], 30, 5, &UmapParams::default()).is_err());
    }
}
