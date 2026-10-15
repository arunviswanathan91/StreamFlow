//! k-means (Lloyd) with k-means++ seeding and several restarts, seeded and deterministic.
//!
//! Results are reproducible for a given `(data, k, seed)` but are **not** bit-identical to
//! scikit-learn's (different RNG and initialisation order); anything validated against it is checked
//! on cluster *structure*, not label numbering.

use crate::rng::Rng;

#[derive(Debug, Clone)]
pub struct KMeans {
    /// Cluster index per row.
    pub labels: Vec<usize>,
    /// `k x d` row-major centres.
    pub centers: Vec<f64>,
    /// Sum of squared distances to the assigned centre.
    pub inertia: f64,
}

fn dist2(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

/// `data` is `n x d` row-major. Keeps the best of `n_init` restarts (lowest inertia).
pub fn kmeans(data: &[f64], n: usize, d: usize, k: usize, n_init: usize, max_iter: usize, seed: u64) -> Result<KMeans, String> {
    if d == 0 || data.len() != n * d {
        return Err(format!("kmeans: data length {} is not {n} x {d}", data.len()));
    }
    if k == 0 || k > n {
        return Err(format!("kmeans: need 1 <= k <= n, got k={k}, n={n}"));
    }
    if data.iter().any(|v| !v.is_finite()) {
        return Err("kmeans: data contains NaN or infinite values".into());
    }
    let mut rng = Rng::new(seed);
    let mut best: Option<KMeans> = None;
    for _ in 0..n_init.max(1) {
        let run = lloyd(data, n, d, k, max_iter, &mut rng);
        if best.as_ref().map_or(true, |b| run.inertia < b.inertia) {
            best = Some(run);
        }
    }
    Ok(best.expect("n_init >= 1"))
}

fn lloyd(data: &[f64], n: usize, d: usize, k: usize, max_iter: usize, rng: &mut Rng) -> KMeans {
    let row = |i: usize| &data[i * d..(i + 1) * d];
    // k-means++ seeding
    let mut centers = Vec::with_capacity(k * d);
    centers.extend_from_slice(row(rng.below(n)));
    let mut nearest: Vec<f64> = (0..n).map(|i| dist2(row(i), &centers[..d])).collect();
    while centers.len() < k * d {
        let total: f64 = nearest.iter().sum();
        let pick = if total <= 0.0 {
            rng.below(n)
        } else {
            let mut target = rng.next_f64() * total;
            let mut chosen = n - 1;
            for (i, &w) in nearest.iter().enumerate() {
                if target < w {
                    chosen = i;
                    break;
                }
                target -= w;
            }
            chosen
        };
        let c0 = centers.len();
        centers.extend_from_slice(row(pick));
        for i in 0..n {
            nearest[i] = nearest[i].min(dist2(row(i), &centers[c0..c0 + d]));
        }
    }
    let mut labels = vec![usize::MAX; n];
    for _ in 0..max_iter.max(1) {
        let mut changed = false;
        for i in 0..n {
            let (mut bl, mut bd) = (0usize, f64::INFINITY);
            for c in 0..k {
                let dd = dist2(row(i), &centers[c * d..(c + 1) * d]);
                if dd < bd {
                    bd = dd;
                    bl = c;
                }
            }
            if labels[i] != bl {
                labels[i] = bl;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        let mut sums = vec![0.0; k * d];
        let mut counts = vec![0usize; k];
        for i in 0..n {
            counts[labels[i]] += 1;
            for j in 0..d {
                sums[labels[i] * d + j] += data[i * d + j];
            }
        }
        for c in 0..k {
            if counts[c] > 0 {
                for j in 0..d {
                    centers[c * d + j] = sums[c * d + j] / counts[c] as f64;
                }
            } else {
                // empty cluster: restart it at the point farthest from its own centre
                let far = (0..n)
                    .max_by(|&p, &q| {
                        dist2(row(p), &centers[labels[p] * d..(labels[p] + 1) * d])
                            .total_cmp(&dist2(row(q), &centers[labels[q] * d..(labels[q] + 1) * d]))
                    })
                    .unwrap();
                centers[c * d..(c + 1) * d].copy_from_slice(row(far));
            }
        }
    }
    let inertia = (0..n).map(|i| dist2(row(i), &centers[labels[i] * d..(labels[i] + 1) * d])).sum();
    KMeans { labels, centers, inertia }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blobs(seed: u64) -> (Vec<f64>, usize) {
        let mut r = Rng::new(seed);
        let mut v = Vec::new();
        for (cx, cy) in [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)] {
            for _ in 0..100 {
                v.push(cx + (r.next_f64() - 0.5));
                v.push(cy + (r.next_f64() - 0.5));
            }
        }
        (v, 300)
    }

    #[test]
    fn recovers_three_well_separated_blobs() {
        let (data, n) = blobs(3);
        let km = kmeans(&data, n, 2, 3, 5, 100, 1).unwrap();
        // each true blob must be one pure cluster
        for blob in 0..3 {
            let first = km.labels[blob * 100];
            assert!(km.labels[blob * 100..(blob + 1) * 100].iter().all(|&l| l == first), "blob {blob} split");
        }
        let mut ids: Vec<usize> = (0..3).map(|b| km.labels[b * 100]).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 3);
        assert!(km.inertia < 300.0 * 0.5, "inertia {}", km.inertia);
    }

    #[test]
    fn deterministic_for_a_seed() {
        let (data, n) = blobs(4);
        let a = kmeans(&data, n, 2, 3, 3, 100, 77).unwrap();
        let b = kmeans(&data, n, 2, 3, 3, 100, 77).unwrap();
        assert_eq!(a.labels, b.labels);
        assert_eq!(a.centers, b.centers);
    }

    #[test]
    fn rejects_bad_input_with_messages() {
        assert!(kmeans(&[1.0, 2.0], 2, 1, 3, 1, 10, 0).unwrap_err().contains("k"));
        assert!(kmeans(&[1.0, f64::NAN], 2, 1, 1, 1, 10, 0).unwrap_err().contains("NaN"));
        assert!(kmeans(&[1.0], 2, 1, 1, 1, 10, 0).is_err());
    }
}
