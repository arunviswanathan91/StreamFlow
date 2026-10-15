//! Clustering building blocks and a FlowSOM-style pipeline.
//!
//! * [`flowsom`]: self-organising map (Kohonen 1990; grid, online updates with a linearly shrinking
//!   learning rate and neighbourhood radius, as in Van Gassen et al., Cytometry A 2015) followed by
//!   consensus hierarchical metaclustering of the SOM nodes (Monti et al., Machine Learning 2003: repeated
//!   subsampling, average-linkage trees, co-association matrix, final average-linkage cut).
//! * [`scale`]: the per-marker scaling the engine applies before clustering (z-score with the population
//!   SD, or min-max).
//! * [`adjusted_rand_index`]: Hubert & Arabie 1985, for comparing two labelings.
//!
//! Everything is seeded; the same seed gives the same labels on every run and platform. The labels are
//! not bit-identical to the Python `flowsom` package (different RNG streams); agreement is measured with
//! the adjusted Rand index in the tests.

use crate::rng::Rng;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    None,
    /// `(x - mean) / sd`, population SD, constant columns left at 0 spread (sd treated as 1).
    ZScore,
    MinMax,
}

/// Scale the columns of a row-major `n x d` matrix (returns a new matrix).
pub fn scale(data: &[f64], n: usize, d: usize, how: Scale) -> Vec<f64> {
    assert_eq!(data.len(), n * d);
    let mut out = data.to_vec();
    if n == 0 || how == Scale::None {
        return out;
    }
    for j in 0..d {
        let col = (0..n).map(|i| data[i * d + j]);
        match how {
            Scale::ZScore => {
                let mean = col.clone().sum::<f64>() / n as f64;
                let var = col.map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
                let sd = if var.sqrt() == 0.0 { 1.0 } else { var.sqrt() };
                for i in 0..n {
                    out[i * d + j] = (data[i * d + j] - mean) / sd;
                }
            }
            Scale::MinMax => {
                let lo = col.clone().fold(f64::INFINITY, f64::min);
                let hi = col.fold(f64::NEG_INFINITY, f64::max);
                let span = if hi - lo == 0.0 { 1.0 } else { hi - lo };
                for i in 0..n {
                    out[i * d + j] = (data[i * d + j] - lo) / span;
                }
            }
            Scale::None => {}
        }
    }
    out
}

/// Per-column `(x - offset) / divisor`, fitted on one matrix and applied to another, so events mapped onto
/// a trained SOM are scaled with the TRAINING statistics, never their own.
#[derive(Debug, Clone, PartialEq)]
pub struct Scaler {
    pub offset: Vec<f64>,
    pub divisor: Vec<f64>,
}

impl Scaler {
    pub fn fit(data: &[f64], n: usize, d: usize, how: Scale) -> Scaler {
        assert_eq!(data.len(), n * d);
        let (mut offset, mut divisor) = (vec![0.0; d], vec![1.0; d]);
        if n > 0 {
            for j in 0..d {
                let col = (0..n).map(|i| data[i * d + j]);
                match how {
                    Scale::ZScore => {
                        let mean = col.clone().sum::<f64>() / n as f64;
                        let var = col.map(|v| (v - mean) * (v - mean)).sum::<f64>() / n as f64;
                        offset[j] = mean;
                        divisor[j] = if var.sqrt() == 0.0 { 1.0 } else { var.sqrt() };
                    }
                    Scale::MinMax => {
                        let lo = col.clone().fold(f64::INFINITY, f64::min);
                        let hi = col.fold(f64::NEG_INFINITY, f64::max);
                        offset[j] = lo;
                        divisor[j] = if hi - lo == 0.0 { 1.0 } else { hi - lo };
                    }
                    Scale::None => {}
                }
            }
        }
        Scaler { offset, divisor }
    }

    pub fn transform(&self, data: &[f64], n: usize) -> Vec<f64> {
        let d = self.offset.len();
        assert_eq!(data.len(), n * d);
        (0..n * d).map(|q| (data[q] - self.offset[q % d]) / self.divisor[q % d]).collect()
    }
}

fn choose2(x: u64) -> f64 {
    (x * x.saturating_sub(1) / 2) as f64
}

/// Adjusted Rand index of two labelings of the same items (1 = identical partitions up to renaming,
/// ~0 = chance agreement, can be negative).
pub fn adjusted_rand_index(a: &[usize], b: &[usize]) -> f64 {
    assert_eq!(a.len(), b.len(), "labelings must cover the same items");
    let n = a.len() as u64;
    if n < 2 {
        return 1.0;
    }
    let mut cont: HashMap<(usize, usize), u64> = HashMap::new();
    let (mut ra, mut rb): (HashMap<usize, u64>, HashMap<usize, u64>) = (HashMap::new(), HashMap::new());
    for (&x, &y) in a.iter().zip(b) {
        *cont.entry((x, y)).or_default() += 1;
        *ra.entry(x).or_default() += 1;
        *rb.entry(y).or_default() += 1;
    }
    let sum_ij: f64 = cont.values().map(|&v| choose2(v)).sum();
    let sum_a: f64 = ra.values().map(|&v| choose2(v)).sum();
    let sum_b: f64 = rb.values().map(|&v| choose2(v)).sum();
    let expected = sum_a * sum_b / choose2(n);
    let max = (sum_a + sum_b) / 2.0;
    if max == expected {
        return 1.0; // both partitions trivial (all one cluster, or all singletons)
    }
    (sum_ij - expected) / (max - expected)
}

/// Average-linkage (UPGMA) agglomerative clustering of `n` items from a full symmetric distance matrix,
/// cut into `k` clusters. Labels are numbered by first appearance. O(n^3): intended for SOM nodes.
pub fn average_linkage_cut(dist: &[f64], n: usize, k: usize) -> Result<Vec<usize>, String> {
    if dist.len() != n * n {
        return Err("distance matrix must be n x n".into());
    }
    if k == 0 || k > n {
        return Err(format!("cannot cut {n} items into {k} clusters"));
    }
    let mut d = dist.to_vec();
    let mut size = vec![1.0f64; n];
    let mut active = vec![true; n];
    let mut parent: Vec<usize> = (0..n).collect(); // union-find on representative = lowest index kept
    let find = |p: &mut Vec<usize>, mut x: usize| {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    };
    for _ in 0..(n - k) {
        let (mut bi, mut bj, mut best) = (0, 0, f64::INFINITY);
        for i in 0..n {
            if !active[i] {
                continue;
            }
            for j in i + 1..n {
                if active[j] && d[i * n + j] < best {
                    (bi, bj, best) = (i, j, d[i * n + j]);
                }
            }
        }
        // merge bj into bi; Lance-Williams update for average linkage
        for m in 0..n {
            if active[m] && m != bi && m != bj {
                let nd = (size[bi] * d[bi * n + m] + size[bj] * d[bj * n + m]) / (size[bi] + size[bj]);
                d[bi * n + m] = nd;
                d[m * n + bi] = nd;
            }
        }
        size[bi] += size[bj];
        active[bj] = false;
        let rb = find(&mut parent, bj);
        let ra = find(&mut parent, bi);
        parent[rb] = ra;
    }
    let mut next = 0usize;
    let mut name: HashMap<usize, usize> = HashMap::new();
    let mut labels = Vec::with_capacity(n);
    for i in 0..n {
        let r = find(&mut parent, i);
        let l = *name.entry(r).or_insert_with(|| {
            next += 1;
            next - 1
        });
        labels.push(l);
    }
    Ok(labels)
}

fn euclid_matrix(x: &[f64], n: usize, d: usize) -> Vec<f64> {
    let mut m = vec![0.0; n * n];
    for i in 0..n {
        for j in i + 1..n {
            let s: f64 = (0..d).map(|c| (x[i * d + c] - x[j * d + c]).powi(2)).sum::<f64>().sqrt();
            m[i * n + j] = s;
            m[j * n + i] = s;
        }
    }
    m
}

/// Consensus clustering of `n` points into `k` clusters: `reps` subsamples of fraction `p_item`, each
/// clustered by average linkage; items are then clustered on `1 - co-association`.
pub fn consensus_cluster(x: &[f64], n: usize, d: usize, k: usize, reps: usize, p_item: f64, seed: u64) -> Result<Vec<usize>, String> {
    if k == 0 || k > n {
        return Err(format!("cannot cluster {n} points into {k} groups"));
    }
    if !(p_item > 0.0 && p_item <= 1.0) || reps == 0 {
        return Err("consensus needs reps >= 1 and 0 < p_item <= 1".into());
    }
    let full = euclid_matrix(x, n, d);
    if k == n {
        return average_linkage_cut(&full, n, k);
    }
    let mut rng = Rng::new(seed);
    let m = ((n as f64 * p_item).round() as usize).clamp(k.max(2), n);
    let (mut together, mut both) = (vec![0u32; n * n], vec![0u32; n * n]);
    for _ in 0..reps {
        let idx = rng.choose_sorted(n, m);
        let mut sub = vec![0.0; m * m];
        for (a, &i) in idx.iter().enumerate() {
            for (b, &j) in idx.iter().enumerate() {
                sub[a * m + b] = full[i * n + j];
            }
        }
        let lab = average_linkage_cut(&sub, m, k)?;
        for a in 0..m {
            for b in 0..m {
                both[idx[a] * n + idx[b]] += 1;
                if lab[a] == lab[b] {
                    together[idx[a] * n + idx[b]] += 1;
                }
            }
        }
    }
    let dist: Vec<f64> = (0..n * n)
        .map(|q| {
            let (i, j) = (q / n, q % n);
            if i == j {
                0.0
            } else if both[q] == 0 {
                1.0
            } else {
                1.0 - together[q] as f64 / both[q] as f64
            }
        })
        .collect();
    average_linkage_cut(&dist, n, k)
}

#[derive(Debug, Clone)]
pub struct Som {
    pub xdim: usize,
    pub ydim: usize,
    pub d: usize,
    /// `(xdim*ydim) x d`, node `y*xdim + x`.
    pub codebook: Vec<f64>,
}

fn nearest(codebook: &[f64], nodes: usize, d: usize, row: &[f64]) -> usize {
    let (mut best, mut bd) = (0, f64::INFINITY);
    for c in 0..nodes {
        let w = &codebook[c * d..(c + 1) * d];
        let mut s = 0.0;
        for t in 0..d {
            let diff = row[t] - w[t];
            s += diff * diff;
            if s >= bd {
                break;
            }
        }
        if s < bd {
            (best, bd) = (c, s);
        }
    }
    best
}

impl Som {
    /// Train a SOM: codebook initialised from random events, `rlen` passes over the data, learning rate
    /// 0.05 -> 0.01 and bubble-neighbourhood radius (67th percentile of grid distances) -> 0, both linear.
    pub fn train(data: &[f64], n: usize, d: usize, xdim: usize, ydim: usize, rlen: usize, seed: u64) -> Result<Som, String> {
        let nodes = xdim * ydim;
        if n == 0 || d == 0 || nodes == 0 || rlen == 0 {
            return Err("SOM needs events, markers, a non-empty grid and rlen >= 1".into());
        }
        if data.len() != n * d || data.iter().any(|v| !v.is_finite()) {
            return Err("SOM input must be a finite n x d matrix".into());
        }
        let mut rng = Rng::new(seed);
        let mut codebook = Vec::with_capacity(nodes * d);
        for _ in 0..nodes {
            let r = rng.below(n);
            codebook.extend_from_slice(&data[r * d..(r + 1) * d]);
        }
        let gd = |a: usize, b: usize| {
            let (ax, ay, bx, by) = ((a % xdim) as f64, (a / xdim) as f64, (b % xdim) as f64, (b / xdim) as f64);
            ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt()
        };
        let mut all: Vec<f64> = Vec::with_capacity(nodes * nodes);
        for a in 0..nodes {
            for b in 0..nodes {
                all.push(gd(a, b));
            }
        }
        all.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let r0 = all[((all.len() - 1) as f64 * 0.67).floor() as usize];
        let (a0, a1) = (0.05f64, 0.01f64);
        let total = (rlen * n) as f64;
        let mut order: Vec<usize> = (0..n).collect();
        let mut step = 0.0f64;
        for _ in 0..rlen {
            for i in (1..n).rev() {
                order.swap(i, rng.below(i + 1)); // Fisher-Yates
            }
            for &e in &order {
                let frac = step / total;
                step += 1.0;
                let alpha = a0 + (a1 - a0) * frac;
                let radius = r0 * (1.0 - frac);
                let row = &data[e * d..(e + 1) * d];
                let bmu = nearest(&codebook, nodes, d, row);
                for c in 0..nodes {
                    if c == bmu || gd(bmu, c) <= radius {
                        for t in 0..d {
                            codebook[c * d + t] += alpha * (row[t] - codebook[c * d + t]);
                        }
                    }
                }
            }
        }
        Ok(Som { xdim, ydim, d, codebook })
    }

    /// Nearest node of every event.
    pub fn assign(&self, data: &[f64], n: usize) -> Vec<usize> {
        let nodes = self.xdim * self.ydim;
        (0..n).map(|i| nearest(&self.codebook, nodes, self.d, &data[i * self.d..(i + 1) * self.d])).collect()
    }
}

#[derive(Debug, Clone)]
pub struct FlowSom {
    pub som: Som,
    /// SOM node of each event.
    pub node_of_event: Vec<usize>,
    /// Metacluster of each SOM node.
    pub meta_of_node: Vec<usize>,
    /// Metacluster of each event.
    pub labels: Vec<usize>,
    pub n_clusters: usize,
}

/// SOM + consensus metaclustering into `k` clusters. `xdim x ydim` is the SOM grid (FlowSOM default 10x10).
pub fn flowsom(data: &[f64], n: usize, d: usize, xdim: usize, ydim: usize, k: usize, rlen: usize, seed: u64) -> Result<FlowSom, String> {
    let nodes = xdim * ydim;
    if k == 0 || k > nodes {
        return Err(format!("k={k} must be between 1 and the {nodes} SOM nodes"));
    }
    let som = Som::train(data, n, d, xdim, ydim, rlen, seed)?;
    let node_of_event = som.assign(data, n);
    let meta_of_node = consensus_cluster(&som.codebook, nodes, d, k, 100, 0.9, seed ^ 0x9E37_79B9_7F4A_7C15)?;
    let labels = node_of_event.iter().map(|&nd| meta_of_node[nd]).collect();
    Ok(FlowSom { som, node_of_event, meta_of_node, labels, n_clusters: k })
}

impl FlowSom {
    /// Map events NOT used for training (e.g. every event in the file, after training on a downsample) onto
    /// the trained SOM and its metaclusters. Returns `None` for an event with a non-finite value — it has no
    /// meaningful nearest node, and silently filing it under node 0 would put it in a wrong population.
    pub fn predict(&self, data: &[f64], n: usize) -> Result<Vec<Option<usize>>, String> {
        let d = self.som.d;
        if data.len() != n * d {
            return Err(format!("expected {} columns per event, got a matrix of {} values for {n} events", d, data.len()));
        }
        let nodes = self.som.xdim * self.som.ydim;
        Ok((0..n)
            .map(|i| {
                let row = &data[i * d..(i + 1) * d];
                if row.iter().all(|v| v.is_finite()) {
                    Some(self.meta_of_node[nearest(&self.som.codebook, nodes, d, row)])
                } else {
                    None
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ari_matches_a_hand_computed_value_and_is_label_permutation_invariant() {
        // a = [0,0,1,1], b = [0,0,1,2]: sum C(nij,2)=1, sum_a=2, sum_b=1, C(4,2)=6 -> (1-1/3)/(1.5-1/3)=4/7
        let v = adjusted_rand_index(&[0, 0, 1, 1], &[0, 0, 1, 2]);
        assert!((v - 4.0 / 7.0).abs() < 1e-12, "{v}");
        assert!((adjusted_rand_index(&[0, 0, 1, 1, 2], &[7, 7, 3, 3, 9]) - 1.0).abs() < 1e-12);
        // independent halves: ARI is exactly the chance-corrected 0 or slightly negative
        let a: Vec<usize> = (0..100).map(|i| i % 2).collect();
        let b: Vec<usize> = (0..100).map(|i| (i / 2) % 2).collect();
        assert!(adjusted_rand_index(&a, &b).abs() < 0.05);
    }

    #[test]
    fn scaling_uses_population_sd_and_survives_constant_columns() {
        let x = [1.0, 5.0, 2.0, 5.0, 3.0, 5.0];
        let z = scale(&x, 3, 2, Scale::ZScore);
        let sd = (2.0f64 / 3.0).sqrt();
        assert!((z[0] + 1.0 / sd).abs() < 1e-12 && z[2].abs() < 1e-12 && (z[4] - 1.0 / sd).abs() < 1e-12);
        assert_eq!((z[1], z[3], z[5]), (0.0, 0.0, 0.0), "constant column stays finite");
        let m = scale(&x, 3, 2, Scale::MinMax);
        assert_eq!((m[0], m[2], m[4]), (0.0, 0.5, 1.0));
    }

    #[test]
    fn a_fitted_scaler_applies_training_statistics_to_new_events_and_matches_scale() {
        let train = [0.0, 10.0, 2.0, 10.0, 4.0, 10.0];
        let sc = Scaler::fit(&train, 3, 2, Scale::ZScore);
        assert_eq!(sc.transform(&train, 3), scale(&train, 3, 2, Scale::ZScore), "fit+transform equals scale");
        let other = [2.0, 10.0];
        let sd = (8.0f64 / 3.0).sqrt();
        assert!(sc.transform(&other, 1)[0].abs() < 1e-12, "the training mean maps to 0");
        let shifted = sc.transform(&[4.0, 10.0], 1)[0];
        assert!((shifted - 2.0 / sd).abs() < 1e-12, "uses the TRAINING sd, not the new matrix");
        assert_eq!(Scaler::fit(&train, 3, 2, Scale::MinMax).transform(&[1.0, 10.0], 1), vec![0.25, 0.0]);
    }

    #[test]
    fn average_linkage_on_a_line_groups_the_obvious_neighbours() {
        let pts: [f64; 5] = [0.0, 1.0, 10.0, 11.0, 100.0];
        let n = pts.len();
        let d: Vec<f64> = (0..n * n).map(|q| (pts[q / n] - pts[q % n]).abs()).collect();
        assert_eq!(average_linkage_cut(&d, n, 3).unwrap(), vec![0, 0, 1, 1, 2]);
        assert_eq!(average_linkage_cut(&d, n, 2).unwrap(), vec![0, 0, 0, 0, 1], "10,11 merge with 0,1 before 100 does");
        assert_eq!(average_linkage_cut(&d, n, 5).unwrap(), vec![0, 1, 2, 3, 4]);
        assert!(average_linkage_cut(&d, n, 6).is_err());
        // UPGMA distance is the AVERAGE over pairs: {0,1} vs {10}: (10 + 9)/2 = 9.5 < {10,11} vs {100}
        assert_eq!(average_linkage_cut(&d, n, 4).unwrap(), vec![0, 0, 1, 2, 3]);
    }

    fn blobs(per: usize, centres: &[[f64; 4]], sd: f64, seed: u64) -> (Vec<f64>, Vec<usize>) {
        let mut rng = Rng::new(seed);
        let (mut x, mut y) = (Vec::new(), Vec::new());
        for i in 0..per * centres.len() {
            let c = i % centres.len();
            for t in 0..4 {
                // Box-Muller
                let (u1, u2) = (rng.next_f64().max(1e-300), rng.next_f64());
                x.push(centres[c][t] + sd * (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos());
            }
            y.push(c);
        }
        (x, y)
    }

    #[test]
    fn flowsom_recovers_separated_populations_and_is_reproducible() {
        let centres = [[0.0, 0.0, 0.0, 0.0], [6.0, 0.0, 6.0, 0.0], [0.0, 6.0, 0.0, 6.0], [6.0, 6.0, -6.0, 6.0]];
        let (x, truth) = blobs(500, &centres, 1.0, 11);
        let n = truth.len();
        let a = flowsom(&x, n, 4, 8, 8, 4, 10, 1).unwrap();
        assert!(adjusted_rand_index(&a.labels, &truth) > 0.97, "ARI {}", adjusted_rand_index(&a.labels, &truth));
        let b = flowsom(&x, n, 4, 8, 8, 4, 10, 1).unwrap();
        assert_eq!(a.labels, b.labels, "same seed, same labels");
        let c = flowsom(&x, n, 4, 8, 8, 4, 10, 2).unwrap();
        assert!(adjusted_rand_index(&c.labels, &truth) > 0.95, "different seed must still find the clusters");
        // every node got a metacluster and every event a node
        assert!(a.meta_of_node.iter().all(|&m| m < 4) && a.node_of_event.iter().all(|&nd| nd < 64));
    }

    #[test]
    fn bad_inputs_are_errors_not_panics() {
        assert!(flowsom(&[1.0, 2.0], 1, 2, 2, 2, 5, 1, 0).is_err(), "k above the node count");
        assert!(flowsom(&[f64::NAN, 1.0], 1, 2, 2, 2, 2, 1, 0).is_err());
        assert!(flowsom(&[], 0, 2, 2, 2, 2, 1, 0).is_err());
        assert!(consensus_cluster(&[1.0, 2.0], 2, 1, 3, 10, 0.9, 0).is_err());
    }
}
