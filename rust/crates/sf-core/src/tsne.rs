//! t-SNE (van der Maaten & Hinton, JMLR 2008) with the Barnes-Hut approximation of the repulsive forces
//! (van der Maaten, JMLR 2014).
//!
//! Pipeline: perplexity-calibrated Gaussian affinities on the `3 * perplexity` nearest neighbours,
//! symmetrised and normalised (`P`, summing to 1); PCA initialisation scaled to a tiny spread; gradient
//! descent with momentum and per-parameter gains (Jacobs 1988), early exaggeration of `P`, and a quadtree
//! for the all-pairs repulsion. Learning rate defaults to `n / exaggeration` (Belkina et al. 2019).
//!
//! With `Q_ij = w_ij / Z`, `w_ij = 1 / (1 + |y_i - y_j|^2)` and `Z = sum_{k != l} w_kl` the cost is
//! `KL(P || Q)` and its gradient is
//! `dC/dy_i = 4 * ( sum_j p_ij w_ij (y_i - y_j)  -  (1/Z) * sum_j w_ij^2 (y_i - y_j) )`
//! (attractive sparse sum, repulsive all-pairs sum). [`gradient_exact`] evaluates it directly and is the
//! reference the Barnes-Hut [`gradient_bh`] and the tests check against.

use crate::graphcluster::knn;
use crate::linalg::sym_eig;
use crate::rng::Rng;

/// Symmetric, normalised input affinities in CSR form (each unordered pair stored in both directions).
#[derive(Debug, Clone)]
pub struct Affinities {
    pub n: usize,
    pub row_ptr: Vec<usize>,
    pub col: Vec<u32>,
    pub val: Vec<f64>,
}

/// Conditional distribution `p_{j|i}` over one point's neighbours with entropy `ln(perplexity)`,
/// found by bisection on the precision `beta = 1 / (2 sigma^2)`. `sq` are squared distances.
pub fn conditional_row(sq: &[f64], perplexity: f64) -> Vec<f64> {
    let target = perplexity.ln();
    let min = sq.iter().cloned().fold(f64::INFINITY, f64::min);
    let shifted: Vec<f64> = sq.iter().map(|d| d - min).collect(); // a shift cancels on normalisation
    let (mut beta, mut lo, mut hi) = (1.0f64, 0.0f64, f64::INFINITY);
    let mut p = vec![0.0; sq.len()];
    for _ in 0..200 {
        let mut sum = 0.0;
        for (pi, d) in p.iter_mut().zip(&shifted) {
            *pi = (-beta * d).exp();
            sum += *pi;
        }
        let dp: f64 = p.iter().zip(&shifted).map(|(pi, d)| pi * d).sum();
        let h = sum.ln() + beta * dp / sum; // entropy of the normalised row
        let diff = h - target;
        if diff.abs() < 1e-7 {
            break;
        }
        if diff > 0.0 {
            lo = beta; // too flat -> sharpen
            beta = if hi.is_infinite() { beta * 2.0 } else { (beta + hi) / 2.0 };
        } else {
            hi = beta;
            beta = (beta + lo) / 2.0;
        }
    }
    let sum: f64 = p.iter().sum();
    p.iter().map(|v| v / sum).collect()
}

/// Input affinities from the data: kNN, per-point perplexity calibration, `(P + P^T) / 2n`.
pub fn affinities(data: &[f64], n: usize, d: usize, perplexity: f64) -> Result<Affinities, String> {
    if !(perplexity > 1.0) {
        return Err(format!("perplexity must be > 1 (got {perplexity})"));
    }
    let k = ((3.0 * perplexity).floor() as usize).min(n.saturating_sub(1));
    if (k as f64) <= perplexity {
        return Err(format!("{n} points are too few for perplexity {perplexity} (need more than {} points)", 3.0 * perplexity));
    }
    let nb = knn(data, n, d, k)?;
    let mut entries: std::collections::HashMap<(u32, u32), f64> = std::collections::HashMap::with_capacity(n * k);
    for i in 0..n {
        let sq: Vec<f64> = nb[i * k..(i + 1) * k]
            .iter()
            .map(|&j| (0..d).map(|c| (data[i * d + c] - data[j as usize * d + c]).powi(2)).sum())
            .collect();
        let p = conditional_row(&sq, perplexity);
        for (&j, pj) in nb[i * k..(i + 1) * k].iter().zip(p) {
            *entries.entry((i as u32, j)).or_default() += pj;
            *entries.entry((j, i as u32)).or_default() += pj;
        }
    }
    let mut rows: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n];
    for ((i, j), v) in entries {
        rows[i as usize].push((j, v / (2.0 * n as f64)));
    }
    let (mut row_ptr, mut col, mut val) = (vec![0usize], Vec::new(), Vec::new());
    for r in rows.iter_mut() {
        r.sort_by_key(|e| e.0);
        for &(j, v) in r.iter() {
            col.push(j);
            val.push(v);
        }
        row_ptr.push(col.len());
    }
    Ok(Affinities { n, row_ptr, col, val })
}

impl Affinities {
    pub fn total(&self) -> f64 {
        self.val.iter().sum()
    }
}

/// Exact `Z` and gradient (O(n^2)); `exaggeration` scales `P`.
pub fn gradient_exact(p: &Affinities, y: &[f64], exaggeration: f64) -> (Vec<f64>, f64) {
    let n = p.n;
    let mut z = 0.0;
    let mut rep = vec![0.0; n * 2];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
            let w = 1.0 / (1.0 + dx * dx + dy * dy);
            z += w;
            rep[2 * i] += w * w * dx;
            rep[2 * i + 1] += w * w * dy;
        }
    }
    let mut g = vec![0.0; n * 2];
    for i in 0..n {
        let (mut ax, mut ay) = (0.0, 0.0);
        for e in p.row_ptr[i]..p.row_ptr[i + 1] {
            let j = p.col[e] as usize;
            let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
            let w = 1.0 / (1.0 + dx * dx + dy * dy);
            ax += exaggeration * p.val[e] * w * dx;
            ay += exaggeration * p.val[e] * w * dy;
        }
        g[2 * i] = 4.0 * (ax - rep[2 * i] / z);
        g[2 * i + 1] = 4.0 * (ay - rep[2 * i + 1] / z);
    }
    (g, z)
}

/// `KL(P || Q)` with the exact normaliser.
pub fn kl_divergence(p: &Affinities, y: &[f64]) -> f64 {
    let n = p.n;
    let mut z = 0.0;
    for i in 0..n {
        for j in 0..n {
            if i != j {
                let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
                z += 1.0 / (1.0 + dx * dx + dy * dy);
            }
        }
    }
    let mut kl = 0.0;
    for i in 0..n {
        for e in p.row_ptr[i]..p.row_ptr[i + 1] {
            let j = p.col[e] as usize;
            let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
            let q = 1.0 / (1.0 + dx * dx + dy * dy) / z;
            if p.val[e] > 0.0 {
                kl += p.val[e] * (p.val[e] / q).ln();
            }
        }
    }
    kl
}

const CAP_DEPTH: usize = 48;

struct QNode {
    cx: f64,
    cy: f64,
    half: f64,
    sum_x: f64,
    sum_y: f64,
    count: u32,
    point: i32,
    child: [u32; 4], // 0 = none (the root is index 0 and is never a child)
}

struct QuadTree {
    nodes: Vec<QNode>,
}

impl QuadTree {
    fn build(y: &[f64], n: usize) -> QuadTree {
        let (mut x0, mut x1, mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..n {
            x0 = x0.min(y[2 * i]);
            x1 = x1.max(y[2 * i]);
            y0 = y0.min(y[2 * i + 1]);
            y1 = y1.max(y[2 * i + 1]);
        }
        let half = ((x1 - x0).max(y1 - y0) / 2.0).max(1e-12) * (1.0 + 1e-9);
        let mk = |cx, cy, half| QNode { cx, cy, half, sum_x: 0.0, sum_y: 0.0, count: 0, point: -1, child: [0; 4] };
        let mut t = QuadTree { nodes: vec![mk((x0 + x1) / 2.0, (y0 + y1) / 2.0, half)] };
        for i in 0..n {
            t.insert(i as i32, y[2 * i], y[2 * i + 1], y, &mk);
        }
        t
    }

    fn quadrant(node: &QNode, px: f64, py: f64) -> usize {
        (px >= node.cx) as usize | (((py >= node.cy) as usize) << 1)
    }

    fn child_of(&mut self, at: usize, q: usize, mk: &dyn Fn(f64, f64, f64) -> QNode) -> usize {
        if self.nodes[at].child[q] == 0 {
            let (cx, cy, h) = (self.nodes[at].cx, self.nodes[at].cy, self.nodes[at].half / 2.0);
            let node = mk(cx + if q & 1 == 1 { h } else { -h }, cy + if q & 2 == 2 { h } else { -h }, h);
            self.nodes.push(node);
            let id = (self.nodes.len() - 1) as u32;
            self.nodes[at].child[q] = id;
        }
        self.nodes[at].child[q] as usize
    }

    fn insert(&mut self, p: i32, px: f64, py: f64, y: &[f64], mk: &dyn Fn(f64, f64, f64) -> QNode) {
        let (mut at, mut depth) = (0usize, 0usize);
        loop {
            let before = self.nodes[at].count;
            self.nodes[at].sum_x += px;
            self.nodes[at].sum_y += py;
            self.nodes[at].count += 1;
            let has_children = self.nodes[at].child.iter().any(|&c| c != 0);
            if before == 0 {
                self.nodes[at].point = p;
                return;
            }
            if !has_children && depth >= CAP_DEPTH {
                return; // coincident points share a leaf
            }
            if !has_children {
                // push the resident point down one level first
                let q = self.nodes[at].point;
                self.nodes[at].point = -1;
                let (qx, qy) = (y[2 * q as usize], y[2 * q as usize + 1]);
                let qd = Self::quadrant(&self.nodes[at], qx, qy);
                let c = self.child_of(at, qd, mk);
                self.nodes[c].sum_x += qx;
                self.nodes[c].sum_y += qy;
                self.nodes[c].count = 1;
                self.nodes[c].point = q;
            }
            let qd = Self::quadrant(&self.nodes[at], px, py);
            at = self.child_of(at, qd, mk);
            depth += 1;
        }
    }

    /// Repulsive sums for point `i`: returns `(sum count*w, sum count*w^2*dx, sum count*w^2*dy)`.
    fn repulsion(&self, i: usize, yi: (f64, f64), theta: f64) -> (f64, f64, f64) {
        let (mut z, mut fx, mut fy) = (0.0, 0.0, 0.0);
        let mut stack = vec![0usize];
        let t2 = theta * theta;
        while let Some(a) = stack.pop() {
            let nd = &self.nodes[a];
            if nd.count == 0 {
                continue;
            }
            let leaf = nd.child.iter().all(|&c| c == 0);
            if leaf && nd.count == 1 && nd.point == i as i32 {
                continue;
            }
            let c = nd.count as f64;
            let (comx, comy) = (nd.sum_x / c, nd.sum_y / c);
            let (dx, dy) = (yi.0 - comx, yi.1 - comy);
            let d2 = dx * dx + dy * dy;
            let width = 2.0 * nd.half;
            if leaf || width * width < t2 * d2 {
                // a coincident multi-point leaf that contains i itself: remove i's own (zero-distance) share
                let cnt = if leaf && nd.count > 1 && d2 < 1e-24 { c - 1.0 } else { c };
                let w = 1.0 / (1.0 + d2);
                z += cnt * w;
                fx += cnt * w * w * dx;
                fy += cnt * w * w * dy;
            } else {
                stack.extend(nd.child.iter().filter(|&&k| k != 0).map(|&k| k as usize));
            }
        }
        (z, fx, fy)
    }
}

/// Barnes-Hut gradient and `Z`. `theta = 0` visits every point and equals [`gradient_exact`].
pub fn gradient_bh(p: &Affinities, y: &[f64], exaggeration: f64, theta: f64) -> (Vec<f64>, f64) {
    let n = p.n;
    let tree = QuadTree::build(y, n);
    let threads = std::thread::available_parallelism().map_or(1, |t| t.get()).min(16).max(1);
    let chunk = n.div_ceil(threads).max(1);
    let mut rep = vec![0.0; n * 2];
    let mut zs = vec![0.0; threads];
    std::thread::scope(|s| {
        for ((t, slab), zslot) in rep.chunks_mut(chunk * 2).enumerate().zip(zs.iter_mut()) {
            let tree = &tree;
            s.spawn(move || {
                let mut zt = 0.0;
                for (r, out) in slab.chunks_mut(2).enumerate() {
                    let i = t * chunk + r;
                    let (z, fx, fy) = tree.repulsion(i, (y[2 * i], y[2 * i + 1]), theta);
                    zt += z;
                    out[0] = fx;
                    out[1] = fy;
                }
                *zslot = zt;
            });
        }
    });
    let z: f64 = zs.iter().sum();
    let mut g = vec![0.0; n * 2];
    for i in 0..n {
        let (mut ax, mut ay) = (0.0, 0.0);
        for e in p.row_ptr[i]..p.row_ptr[i + 1] {
            let j = p.col[e] as usize;
            let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
            let w = 1.0 / (1.0 + dx * dx + dy * dy);
            ax += exaggeration * p.val[e] * w * dx;
            ay += exaggeration * p.val[e] * w * dy;
        }
        g[2 * i] = 4.0 * (ax - rep[2 * i] / z);
        g[2 * i + 1] = 4.0 * (ay - rep[2 * i + 1] / z);
    }
    (g, z)
}

#[derive(Debug, Clone)]
pub struct TsneParams {
    pub perplexity: f64,
    pub n_iter: usize,
    pub early_exaggeration_iters: usize,
    pub exaggeration: f64,
    /// `None` = `max(n / exaggeration, 50)`.
    pub learning_rate: Option<f64>,
    pub theta: f64,
    pub seed: u64,
}

impl Default for TsneParams {
    fn default() -> Self {
        TsneParams { perplexity: 30.0, n_iter: 750, early_exaggeration_iters: 250, exaggeration: 12.0, learning_rate: None, theta: 0.5, seed: 42 }
    }
}

#[derive(Debug, Clone)]
pub struct TsneResult {
    /// `n x 2`, row-major, centred on the origin.
    pub y: Vec<f64>,
    /// `(iteration, KL estimate)` every 50 iterations after the exaggeration phase.
    pub kl_history: Vec<(usize, f64)>,
}

/// PCA initialisation: the first two principal components, scaled so the first has SD `1e-4`.
fn pca_init(data: &[f64], n: usize, d: usize, rng: &mut Rng) -> Vec<f64> {
    let mean: Vec<f64> = (0..d).map(|c| (0..n).map(|i| data[i * d + c]).sum::<f64>() / n as f64).collect();
    let mut cov = vec![0.0; d * d];
    for i in 0..n {
        for a in 0..d {
            for b in a..d {
                cov[a * d + b] += (data[i * d + a] - mean[a]) * (data[i * d + b] - mean[b]);
            }
        }
    }
    for a in 0..d {
        for b in a..d {
            cov[a * d + b] /= n as f64;
            cov[b * d + a] = cov[a * d + b];
        }
    }
    let mut y = vec![0.0; n * 2];
    if let Ok((_vals, vecs)) = sym_eig(&cov, d) {
        for comp in 0..d.min(2) {
            for i in 0..n {
                y[2 * i + comp] = (0..d).map(|c| (data[i * d + c] - mean[c]) * vecs[c * d + comp]).sum();
            }
        }
    }
    let m0 = (0..n).map(|i| y[2 * i]).sum::<f64>() / n as f64;
    let sd0 = ((0..n).map(|i| (y[2 * i] - m0).powi(2)).sum::<f64>() / n as f64).sqrt();
    let scale = if sd0 > 0.0 { 1e-4 / sd0 } else { 1.0 };
    for i in 0..n {
        y[2 * i] *= scale;
        y[2 * i + 1] *= scale;
        if d < 2 || y[2 * i + 1] == 0.0 {
            y[2 * i + 1] += 1e-4 * (rng.next_f64() - 0.5); // break the degenerate second axis
        }
    }
    y
}

pub fn tsne(data: &[f64], n: usize, d: usize, params: &TsneParams) -> Result<TsneResult, String> {
    if data.len() != n * d || d == 0 {
        return Err("t-SNE input must be a non-empty n x d matrix".into());
    }
    if data.iter().any(|v| !v.is_finite()) {
        return Err("t-SNE input contains non-finite values".into());
    }
    if params.n_iter <= params.early_exaggeration_iters {
        return Err("n_iter must exceed the early-exaggeration iterations".into());
    }
    let p = affinities(data, n, d, params.perplexity)?;
    let mut rng = Rng::new(params.seed);
    let mut y = pca_init(data, n, d, &mut rng);
    let lr = params.learning_rate.unwrap_or_else(|| (n as f64 / params.exaggeration).max(50.0));
    let (mut update, mut gains) = (vec![0.0; n * 2], vec![1.0; n * 2]);
    let mut history = Vec::new();
    for it in 0..params.n_iter {
        let early = it < params.early_exaggeration_iters;
        let (exag, mom) = if early { (params.exaggeration, 0.5) } else { (1.0, 0.8) };
        let (g, z) = gradient_bh(&p, &y, exag, params.theta);
        for q in 0..n * 2 {
            let same_sign = (g[q] > 0.0) == (update[q] > 0.0);
            let g2: f64 = if same_sign { gains[q] * 0.8 } else { gains[q] + 0.2 };
            gains[q] = g2.max(0.01);
            update[q] = mom * update[q] - lr * gains[q] * g[q];
            y[q] += update[q];
        }
        let (mx, my) = ((0..n).map(|i| y[2 * i]).sum::<f64>() / n as f64, (0..n).map(|i| y[2 * i + 1]).sum::<f64>() / n as f64);
        for i in 0..n {
            y[2 * i] -= mx;
            y[2 * i + 1] -= my;
        }
        if !early && (it + 1) % 50 == 0 {
            // KL with the Barnes-Hut normaliser from this iteration's gradient pass
            let mut kl = 0.0;
            for i in 0..n {
                for e in p.row_ptr[i]..p.row_ptr[i + 1] {
                    let j = p.col[e] as usize;
                    let (dx, dy) = (y[2 * i] - y[2 * j], y[2 * i + 1] - y[2 * j + 1]);
                    let q = 1.0 / (1.0 + dx * dx + dy * dy) / z;
                    kl += p.val[e] * (p.val[e] / q.max(1e-300)).ln();
                }
            }
            history.push((it + 1, kl));
        }
    }
    if y.iter().any(|v| !v.is_finite()) {
        return Err("t-SNE diverged (non-finite coordinates); try a smaller learning rate".into());
    }
    Ok(TsneResult { y, kl_history: history })
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
    fn each_conditional_row_hits_the_requested_perplexity() {
        let sq: Vec<f64> = (1..=40).map(|i| (i as f64).powi(2) * 0.1).collect();
        for perp in [3.0, 10.0, 25.0] {
            let p = conditional_row(&sq, perp);
            let h: f64 = -p.iter().filter(|&&v| v > 0.0).map(|v| v * v.ln()).sum::<f64>();
            assert!((h.exp() - perp).abs() < 1e-4, "perplexity {perp}: got {}", h.exp());
            assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn affinities_are_symmetric_and_sum_to_one() {
        let (x, _) = blobs(60, 1);
        let p = affinities(&x, 180, 5, 10.0).unwrap();
        assert!((p.total() - 1.0).abs() < 1e-9, "{}", p.total());
        let get = |i: usize, j: u32| (p.row_ptr[i]..p.row_ptr[i + 1]).find(|&e| p.col[e] == j).map(|e| p.val[e]);
        for i in 0..180 {
            for e in p.row_ptr[i]..p.row_ptr[i + 1] {
                assert_eq!(get(p.col[e] as usize, i as u32), Some(p.val[e]), "asymmetric at ({i},{})", p.col[e]);
            }
        }
        assert!(affinities(&x, 180, 5, 1.0).is_err() && affinities(&x[..20 * 5], 20, 5, 25.0).is_err());
    }

    #[test]
    fn the_analytic_gradient_equals_the_finite_difference_of_kl() {
        let (x, _) = blobs(10, 2);
        let p = affinities(&x, 30, 5, 4.0).unwrap();
        let mut rng = Rng::new(5);
        let y: Vec<f64> = (0..60).map(|_| (rng.next_f64() - 0.5) * 4.0).collect();
        let (g, _) = gradient_exact(&p, &y, 1.0);
        for q in [0usize, 7, 20, 33, 58] {
            let h = 1e-6;
            let (mut a, mut b) = (y.clone(), y.clone());
            a[q] += h;
            b[q] -= h;
            let fd = (kl_divergence(&p, &a) - kl_divergence(&p, &b)) / (2.0 * h);
            assert!((g[q] - fd).abs() < 1e-6 * (1.0 + fd.abs()), "coord {q}: analytic {} vs finite difference {fd}", g[q]);
        }
    }

    #[test]
    fn barnes_hut_equals_exact_at_theta_zero_and_is_close_at_the_default() {
        let (x, _) = blobs(100, 3);
        let n = 300;
        let p = affinities(&x, n, 5, 15.0).unwrap();
        let mut rng = Rng::new(9);
        let y: Vec<f64> = (0..n * 2).map(|_| (rng.next_f64() - 0.5) * 20.0).collect();
        let (ge, ze) = gradient_exact(&p, &y, 1.0);
        let (g0, z0) = gradient_bh(&p, &y, 1.0, 0.0);
        assert!((z0 - ze).abs() < 1e-9 * ze, "Z {z0} vs {ze}");
        let max0 = ge.iter().zip(&g0).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(max0 < 1e-12, "theta=0 differs from exact by {max0:e}");
        let (g5, z5) = gradient_bh(&p, &y, 1.0, 0.5);
        assert!((z5 - ze).abs() < 0.02 * ze, "Z at theta .5: {z5} vs {ze}");
        let num: f64 = ge.iter().zip(&g5).map(|(a, b)| (a - b).powi(2)).sum::<f64>().sqrt();
        let den: f64 = ge.iter().map(|a| a * a).sum::<f64>().sqrt();
        assert!(num / den < 0.05, "relative gradient error {:.4}", num / den);
    }

    #[test]
    fn coincident_points_do_not_hang_or_corrupt_the_tree() {
        let n = 50;
        let mut y = vec![0.0; n * 2];
        y[..20].fill(1.0); // ten identical points
        for i in 10..n {
            y[2 * i] = i as f64 * 0.1;
            y[2 * i + 1] = -(i as f64) * 0.05;
        }
        let (x, _) = blobs(17, 4);
        let p = affinities(&x[..n * 5], n, 5, 5.0).unwrap();
        let (ge, ze) = gradient_exact(&p, &y, 1.0);
        let (g0, z0) = gradient_bh(&p, &y, 1.0, 0.0);
        assert!((z0 - ze).abs() < 1e-8 * ze);
        assert!(ge.iter().zip(&g0).all(|(a, b)| (a - b).abs() < 1e-9));
    }

    #[test]
    fn tsne_lowers_kl_separates_blobs_and_keeps_neighbourhoods() {
        let (x, truth) = blobs(150, 6);
        let n = 450;
        let params = TsneParams { n_iter: 600, early_exaggeration_iters: 200, perplexity: 20.0, ..Default::default() };
        let r = tsne(&x, n, 5, &params).unwrap();
        assert!(r.y.iter().all(|v| v.is_finite()));
        let kl: Vec<f64> = r.kl_history.iter().map(|h| h.1).collect();
        assert!(kl.len() >= 5 && kl.last().unwrap() < &kl[0], "KL should fall: {kl:?}");
        let trust = crate::embedmetrics::trustworthiness(&x, 5, &r.y, 2, n, 12).unwrap();
        assert!(trust > 0.95, "trustworthiness {trust}");
        let km = crate::kmeans::kmeans(&r.y, n, 2, 3, 5, 100, 1).unwrap();
        let ari = crate::cluster::adjusted_rand_index(&km.labels, &truth);
        assert!(ari > 0.95, "blobs not separated in the map: ARI {ari}");
        // same seed, same map
        let again = tsne(&x, n, 5, &params).unwrap();
        let drift = r.y.iter().zip(&again.y).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(drift < 1e-6, "seeded runs must match (thread-order sums may differ by rounding only): {drift:e}");
    }

    #[test]
    fn bad_inputs_are_errors() {
        let (x, _) = blobs(20, 7);
        assert!(tsne(&x, 60, 5, &TsneParams { n_iter: 100, early_exaggeration_iters: 200, ..Default::default() }).is_err());
        let mut bad = x.clone();
        bad[3] = f64::NAN;
        assert!(tsne(&bad, 60, 5, &TsneParams::default()).is_err());
        assert!(tsne(&x[..10], 60, 5, &TsneParams::default()).is_err());
    }
}
