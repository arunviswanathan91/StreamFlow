//! Quality measures for low-dimensional embeddings (t-SNE, UMAP). Coordinates of such embeddings are
//! arbitrary up to rotation and the optimiser's random path, so they cannot be compared with another
//! implementation directly; what can be compared is how well neighbourhoods are preserved.
//!
//! * [`trustworthiness`] (Venna & Kaski 2001; the same definition as scikit-learn's): the fraction of
//!   embedding-space neighbours that are really neighbours in the data, with intruders penalised by how
//!   far down the data-space ranking they sit. 1 = perfect, ~0.5 = random.
//! * [`knn_preservation`]: the average share of each point's `k` data-space neighbours that are also among
//!   its `k` nearest in the embedding.

fn sq_dists(x: &[f64], n: usize, d: usize, i: usize) -> Vec<f64> {
    let a = &x[i * d..(i + 1) * d];
    (0..n)
        .map(|j| {
            if j == i {
                f64::INFINITY // self is never its own neighbour
            } else {
                (0..d).map(|c| (a[c] - x[j * d + c]).powi(2)).sum()
            }
        })
        .collect()
}

fn order(dist: &[f64]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..dist.len()).collect();
    idx.sort_by(|&a, &b| dist[a].partial_cmp(&dist[b]).unwrap().then(a.cmp(&b)));
    idx
}

/// Trustworthiness of embedding `y` (`n x dy`) of data `x` (`n x dx`) for `k` neighbours. Needs `k < n/2`.
pub fn trustworthiness(x: &[f64], dx: usize, y: &[f64], dy: usize, n: usize, k: usize) -> Result<f64, String> {
    if x.len() != n * dx || y.len() != n * dy {
        return Err("matrix sizes do not match n".into());
    }
    if k == 0 || 2 * k >= n {
        return Err(format!("need 0 < k < n/2 (k={k}, n={n})"));
    }
    let mut penalty = 0.0;
    for i in 0..n {
        let ord_x = order(&sq_dists(x, n, dx, i)); // self last (infinite distance)
        let mut rank = vec![0usize; n]; // 1-based data-space rank of every other point
        for (r, &j) in ord_x.iter().take(n - 1).enumerate() {
            rank[j] = r + 1;
        }
        let ord_y = order(&sq_dists(y, n, dy, i));
        for &j in ord_y.iter().take(k) {
            if rank[j] > k {
                penalty += (rank[j] - k) as f64;
            }
        }
    }
    let (nf, kf) = (n as f64, k as f64);
    Ok(1.0 - 2.0 / (nf * kf * (2.0 * nf - 3.0 * kf - 1.0)) * penalty)
}

/// Mean fraction of each point's `k` nearest data neighbours that are also `k`-nearest in the embedding.
pub fn knn_preservation(x: &[f64], dx: usize, y: &[f64], dy: usize, n: usize, k: usize) -> Result<f64, String> {
    if x.len() != n * dx || y.len() != n * dy || k == 0 || k >= n {
        return Err("bad arguments".into());
    }
    let mut total = 0.0;
    for i in 0..n {
        let a: std::collections::HashSet<usize> = order(&sq_dists(x, n, dx, i)).into_iter().take(k).collect();
        let hits = order(&sq_dists(y, n, dy, i)).into_iter().take(k).filter(|j| a.contains(j)).count();
        total += hits as f64 / k as f64;
    }
    Ok(total / n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_isometric_embedding_is_perfectly_trustworthy() {
        let x: Vec<f64> = (0..40).flat_map(|i| [i as f64, (i * i % 7) as f64]).collect();
        // rotate by 90 degrees and translate: all distances preserved
        let y: Vec<f64> = x.chunks(2).flat_map(|p| [-p[1] + 3.0, p[0] - 1.0]).collect();
        assert_eq!(trustworthiness(&x, 2, &y, 2, 40, 5).unwrap(), 1.0);
        assert_eq!(knn_preservation(&x, 2, &y, 2, 40, 5).unwrap(), 1.0);
    }

    #[test]
    fn a_hand_computed_intruder_penalty() {
        // data on a line 0,1,2,3,4,5 (n=6, k=1). Embedding swaps point 1 and 3 positions: y = 0,3,2,1,4,5
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [0.0, 3.0, 2.0, 1.0, 4.0, 5.0];
        // nearest in y to each point i: 0->{3 (|0-1|=1)} data rank of 3 around 0 is 3 -> penalty 2
        // 1(y=3): pt2 and pt4 tie at distance 1 -> lower index pt2; around x=1 pt0 and pt2 tie at distance 1, so by index pt0 is rank 1 and pt2 rank 2 -> penalty 1
        // 2(y=2): y=1 (pt3) and y=3 (pt1) tie -> pt1; data rank of 1 around 2 is 1 -> 0
        // 3(y=1): nearest y=0 (pt0) / y=2 (pt2) tie -> pt0; data rank of 0 around 3 (x=3): order 2,4,1,5,0 -> rank 5 -> penalty 4
        // 4(y=4): y=5 (pt5) and y=3 (pt1) tie -> pt1; data rank of 1 around 4: 3,5,2,1.. wait order 3,5,2,1,0 -> rank 4 -> penalty 3
        // 5(y=5): y=4 -> pt4, rank 1 -> 0
        let penalty = 2.0 + 1.0 + 0.0 + 4.0 + 3.0 + 0.0;
        let want = 1.0 - 2.0 / (6.0 * 1.0 * (12.0 - 3.0 - 1.0)) * penalty;
        let got = trustworthiness(&x, 1, &y, 1, 6, 1).unwrap();
        assert!((got - want).abs() < 1e-12, "{got} vs {want}");
    }

    #[test]
    fn bad_arguments_are_errors() {
        let x = [0.0; 10];
        assert!(trustworthiness(&x, 1, &x, 1, 10, 5).is_err(), "k must be < n/2");
        assert!(trustworthiness(&x, 1, &x[..9], 1, 10, 2).is_err());
        assert!(knn_preservation(&x, 1, &x, 1, 10, 0).is_err());
    }
}
