//! Graph-based clustering: k-nearest-neighbour graph, Jaccard weighting and Louvain community detection,
//! combined as PhenoGraph (Levine et al., Cell 2015).
//!
//! * [`knn`]: exact Euclidean k nearest neighbours (self excluded), multi-threaded.
//! * [`jaccard_graph`]: edge `i-j` weighted by the overlap of the two cells' neighbour sets,
//!   `shared / (2k - shared)`; the directed graph is symmetrised by averaging with its transpose, so a
//!   one-way neighbour link carries half the weight of a mutual one (the reference implementation's rule).
//! * [`louvain`]: Blondel et al. 2008 — greedy local moving of nodes between communities to raise
//!   modularity, then aggregation of communities into super-nodes, repeated until nothing improves.
//!   Nodes are visited in a seeded random order, so results are reproducible.
//! * [`phenograph`]: all of the above; clusters smaller than `min_cluster_size` become unclustered
//!   (`None`) and the rest are numbered by size, largest first.

use crate::rng::Rng;
use std::collections::HashMap;

/// Exact `k` nearest neighbours of every row (Euclidean), self excluded, nearest first, ties by index.
/// Returns `n x k` indices. Errors if `k >= n`.
pub fn knn(data: &[f64], n: usize, d: usize, k: usize) -> Result<Vec<u32>, String> {
    if data.len() != n * d || d == 0 {
        return Err("kNN input must be a non-empty n x d matrix".into());
    }
    if k == 0 || k >= n {
        return Err(format!("k={k} must be between 1 and n-1 (n={n})"));
    }
    if data.iter().any(|v| !v.is_finite()) {
        return Err("kNN input contains non-finite values".into());
    }
    let mut out = vec![0u32; n * k];
    let threads = std::thread::available_parallelism().map_or(1, |p| p.get()).min(16);
    let chunk = n.div_ceil(threads);
    std::thread::scope(|s| {
        for (t, slab) in out.chunks_mut(chunk * k).enumerate() {
            s.spawn(move || {
                let mut dist: Vec<(f64, u32)> = Vec::with_capacity(n);
                for (r, row_out) in slab.chunks_mut(k).enumerate() {
                    let i = t * chunk + r;
                    let a = &data[i * d..(i + 1) * d];
                    dist.clear();
                    for j in 0..n {
                        if j == i {
                            continue;
                        }
                        let b = &data[j * d..(j + 1) * d];
                        let mut s2 = 0.0;
                        for c in 0..d {
                            let diff = a[c] - b[c];
                            s2 += diff * diff;
                        }
                        dist.push((s2, j as u32));
                    }
                    let cmp = |x: &(f64, u32), y: &(f64, u32)| x.0.partial_cmp(&y.0).unwrap().then(x.1.cmp(&y.1));
                    dist.select_nth_unstable_by(k - 1, cmp);
                    dist[..k].sort_by(cmp);
                    for (o, e) in row_out.iter_mut().zip(&dist[..k]) {
                        *o = e.1;
                    }
                }
            });
        }
    });
    Ok(out)
}

/// An undirected weighted graph in adjacency-list form.
#[derive(Debug, Clone)]
pub struct Graph {
    pub n: usize,
    pub adj: Vec<Vec<(u32, f64)>>,
}

impl Graph {
    /// Build from undirected edges `(i, j, w)`, `i != j`; parallel edges are summed.
    pub fn from_edges(n: usize, edges: impl IntoIterator<Item = (usize, usize, f64)>) -> Graph {
        let mut adj = vec![Vec::new(); n];
        let mut merged: HashMap<(usize, usize), f64> = HashMap::new();
        for (i, j, w) in edges {
            assert!(i < n && j < n && i != j, "bad edge ({i},{j})");
            *merged.entry((i.min(j), i.max(j))).or_default() += w;
        }
        let mut list: Vec<_> = merged.into_iter().collect();
        list.sort_by_key(|e| e.0); // deterministic adjacency order
        for ((i, j), w) in list {
            adj[i].push((j as u32, w));
            adj[j].push((i as u32, w));
        }
        Graph { n, adj }
    }

    pub fn n_edges(&self) -> usize {
        self.adj.iter().map(|a| a.len()).sum::<usize>() / 2
    }
}

/// Jaccard-weighted, symmetrised kNN graph from an `n x k` neighbour table.
pub fn jaccard_graph(neighbors: &[u32], n: usize, k: usize) -> Graph {
    assert_eq!(neighbors.len(), n * k);
    let mut sorted = neighbors.to_vec();
    for row in sorted.chunks_mut(k) {
        row.sort_unstable();
    }
    let shared = |a: usize, b: usize| {
        let (x, y) = (&sorted[a * k..(a + 1) * k], &sorted[b * k..(b + 1) * k]);
        let (mut i, mut j, mut c) = (0, 0, 0usize);
        while i < k && j < k {
            match x[i].cmp(&y[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    c += 1;
                    i += 1;
                    j += 1;
                }
            }
        }
        c
    };
    // directed entries i -> j carry J; (G + G^T) / 2 halves each, and a mutual pair sums both halves
    let mut edges = Vec::new();
    for i in 0..n {
        for &j in &neighbors[i * k..(i + 1) * k] {
            let s = shared(i, j as usize) as f64;
            let w = s / (2.0 * k as f64 - s);
            if w > 0.0 && i != j as usize {
                edges.push((i, j as usize, w / 2.0));
            }
        }
    }
    Graph::from_edges(n, edges)
}

/// Newman modularity of `community` on `g`.
pub fn modularity(g: &Graph, community: &[usize]) -> f64 {
    let m2: f64 = g.adj.iter().flat_map(|a| a.iter().map(|e| e.1)).sum();
    if m2 == 0.0 {
        return 0.0;
    }
    let nc = community.iter().max().map_or(0, |m| m + 1);
    let (mut inside, mut tot) = (vec![0.0; nc], vec![0.0; nc]);
    for i in 0..g.n {
        for &(j, w) in &g.adj[i] {
            tot[community[i]] += w;
            if community[i] == community[j as usize] {
                inside[community[i]] += w;
            }
        }
    }
    (0..nc).map(|c| inside[c] / m2 - (tot[c] / m2).powi(2)).sum()
}

/// Weighted graph with explicit self-loops, the working form for Louvain aggregation.
struct Work {
    /// `adj[i]` includes `(i, A_ii)` when node `i` has a self-loop.
    adj: Vec<Vec<(usize, f64)>>,
}

fn local_moving(w: &Work, rng: &mut Rng, min_gain: f64) -> (Vec<usize>, bool) {
    let n = w.adj.len();
    let deg: Vec<f64> = w.adj.iter().map(|a| a.iter().map(|e| e.1).sum()).collect();
    let m2: f64 = deg.iter().sum();
    let mut comm: Vec<usize> = (0..n).collect();
    let mut tot = deg.clone();
    let mut order: Vec<usize> = (0..n).collect();
    let mut improved_any = false;
    let mut link = vec![0.0f64; n]; // weight from the current node into each community
    let mut touched: Vec<usize> = Vec::new();
    loop {
        for i in (1..n).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let mut moved = false;
        for &v in &order {
            let old = comm[v];
            touched.clear();
            for &(u, wt) in &w.adj[v] {
                if u != v {
                    let c = comm[u];
                    if link[c] == 0.0 {
                        touched.push(c);
                    }
                    link[c] += wt;
                }
            }
            tot[old] -= deg[v];
            // gain of joining community c  ~  link[c] - tot[c] * deg[v] / m2
            let stay = link[old] - tot[old] * deg[v] / m2;
            let (mut best, mut best_gain) = (old, stay);
            for &c in &touched {
                let gain = link[c] - tot[c] * deg[v] / m2;
                if gain > best_gain + min_gain {
                    (best, best_gain) = (c, gain);
                }
            }
            tot[best] += deg[v];
            comm[v] = best;
            if best != old {
                moved = true;
                improved_any = true;
            }
            for &c in &touched {
                link[c] = 0.0;
            }
            link[old] = 0.0;
        }
        if !moved {
            break;
        }
    }
    (comm, improved_any)
}

/// Louvain community detection. Returns `(community of each node, modularity)`, communities numbered
/// `0..` by first appearance.
pub fn louvain(g: &Graph, seed: u64) -> (Vec<usize>, f64) {
    let mut rng = Rng::new(seed);
    let mut work = Work { adj: g.adj.iter().map(|a| a.iter().map(|&(j, w)| (j as usize, w)).collect()).collect() };
    let mut node_comm: Vec<usize> = (0..g.n).collect(); // original node -> current super-node
    loop {
        let (comm, improved) = local_moving(&work, &mut rng, 1e-12);
        if !improved {
            break;
        }
        // renumber communities densely
        let mut map: HashMap<usize, usize> = HashMap::new();
        let dense: Vec<usize> = comm
            .iter()
            .map(|&c| {
                let next = map.len();
                *map.entry(c).or_insert(next)
            })
            .collect();
        for c in node_comm.iter_mut() {
            *c = dense[*c];
        }
        let nc = map.len();
        // aggregate: A'_cd = sum of A_ij over i in c, j in d (self-loops collect the internal weight)
        let mut agg: Vec<HashMap<usize, f64>> = vec![HashMap::new(); nc];
        for (i, edges) in work.adj.iter().enumerate() {
            for &(j, wt) in edges {
                *agg[dense[i]].entry(dense[j]).or_default() += wt;
            }
        }
        work = Work {
            adj: agg
                .into_iter()
                .map(|m| {
                    let mut v: Vec<(usize, f64)> = m.into_iter().collect();
                    v.sort_by_key(|e| e.0);
                    v
                })
                .collect(),
        };
        if nc == 1 {
            break;
        }
    }
    let mut map: HashMap<usize, usize> = HashMap::new();
    let labels: Vec<usize> = node_comm
        .iter()
        .map(|&c| {
            let next = map.len();
            *map.entry(c).or_insert(next)
        })
        .collect();
    let q = modularity(g, &labels);
    (labels, q)
}

#[derive(Debug, Clone)]
pub struct PhenoGraph {
    /// `None` = unclustered (in a cluster smaller than `min_cluster_size`). Clusters are numbered by size, largest = 0.
    pub labels: Vec<Option<usize>>,
    pub n_clusters: usize,
    pub modularity: f64,
    pub graph: Graph,
}

/// `runs` independent Louvain optimisations (different seeds) are made and the highest-modularity one is kept,
/// as the reference implementation does; it is a heuristic, so a single run can land in a poor local optimum.
pub fn phenograph(data: &[f64], n: usize, d: usize, k: usize, min_cluster_size: usize, runs: usize, seed: u64) -> Result<PhenoGraph, String> {
    if runs == 0 {
        return Err("phenograph needs at least one Louvain run".into());
    }
    let nb = knn(data, n, d, k)?;
    let graph = jaccard_graph(&nb, n, k);
    let (comm, q) = (0..runs as u64).map(|r| louvain(&graph, seed.wrapping_add(r.wrapping_mul(0x9E37_79B9)))).max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
    let nc = comm.iter().max().map_or(0, |m| m + 1);
    let mut size = vec![0usize; nc];
    for &c in &comm {
        size[c] += 1;
    }
    let mut order: Vec<usize> = (0..nc).filter(|&c| size[c] >= min_cluster_size).collect();
    order.sort_by(|&a, &b| size[b].cmp(&size[a]).then(a.cmp(&b)));
    let rank: HashMap<usize, usize> = order.iter().enumerate().map(|(r, &c)| (c, r)).collect();
    let labels = comm.iter().map(|c| rank.get(c).copied()).collect();
    Ok(PhenoGraph { labels, n_clusters: order.len(), modularity: q, graph })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_cliques() -> Graph {
        let mut e = Vec::new();
        for base in [0usize, 5] {
            for i in 0..5 {
                for j in i + 1..5 {
                    e.push((base + i, base + j, 1.0));
                }
            }
        }
        e.push((4, 5, 1.0));
        Graph::from_edges(10, e)
    }

    #[test]
    fn modularity_of_two_cliques_matches_the_hand_value() {
        // m = 21; each community: inside = 20 (ordered), tot = 21 -> Q = 2 (20/42 - (21/42)^2) = 0.452381
        let q = modularity(&two_cliques(), &[0, 0, 0, 0, 0, 1, 1, 1, 1, 1]);
        assert!((q - 0.4523809523809524).abs() < 1e-12, "{q}");
        assert!(modularity(&two_cliques(), &[0; 10]).abs() < 1e-12, "one community has Q = 0");
    }

    #[test]
    fn louvain_splits_two_cliques_and_reports_their_modularity() {
        for seed in 0..5 {
            let (c, q) = louvain(&two_cliques(), seed);
            assert!(c[..5].iter().all(|&x| x == c[0]) && c[5..].iter().all(|&x| x == c[5]) && c[0] != c[5], "seed {seed}: {c:?}");
            assert!((q - 0.4523809523809524).abs() < 1e-12);
        }
        // an edgeless graph keeps every node alone and does not divide by zero
        let (c, q) = louvain(&Graph::from_edges(3, std::iter::empty()), 1);
        assert_eq!((c, q), (vec![0, 1, 2], 0.0));
    }

    #[test]
    fn knn_matches_brute_force_with_index_tie_breaks() {
        // points on a line 0,1,2,3,10: neighbours of 2 are 1 and 3 (tie by index), of 10 are 3 then 2
        let x = [0.0, 1.0, 2.0, 3.0, 10.0];
        let nb = knn(&x, 5, 1, 2).unwrap();
        assert_eq!(&nb[2 * 2..3 * 2], &[1, 3]);
        assert_eq!(&nb[4 * 2..5 * 2], &[3, 2]);
        assert_eq!(&nb[0..2], &[1, 2]);
        assert!(knn(&x, 5, 1, 5).is_err() && knn(&x, 5, 1, 0).is_err());
        assert!(knn(&[f64::NAN, 1.0, 2.0], 3, 1, 1).is_err());
    }

    #[test]
    fn knn_multithreaded_equals_a_naive_scan() {
        let mut rng = Rng::new(3);
        let (n, d, k) = (300, 3, 7);
        let x: Vec<f64> = (0..n * d).map(|_| rng.next_f64()).collect();
        let got = knn(&x, n, d, k).unwrap();
        for i in 0..n {
            let mut all: Vec<(f64, usize)> = (0..n).filter(|&j| j != i).map(|j| ((0..d).map(|c| (x[i * d + c] - x[j * d + c]).powi(2)).sum::<f64>(), j)).collect();
            all.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let want: Vec<u32> = all[..k].iter().map(|e| e.1 as u32).collect();
            assert_eq!(&got[i * k..(i + 1) * k], &want[..], "row {i}");
        }
    }

    #[test]
    fn jaccard_weights_follow_the_reference_formula() {
        // n=4, k=2. N(0)={1,2} N(1)={0,2} N(2)={0,1} N(3)={0,1}
        let nb = [1, 2, 0, 2, 0, 1, 0, 1];
        let g = jaccard_graph(&nb, 4, 2);
        let w = |a: usize, b: usize| g.adj[a].iter().find(|e| e.0 as usize == b).map_or(0.0, |e| e.1);
        // shared(0,1)=|{1,2}&{0,2}|=1 -> J=1/3 ; mutual (0->1 and 1->0) -> (1/3 + 1/3)/2 = 1/3
        assert!((w(0, 1) - 1.0 / 3.0).abs() < 1e-12);
        // 3->0 only (0 does not list 3): shared(3,0)=|{0,1}&{1,2}|=1 -> J=1/3 -> one-way -> 1/6
        assert!((w(3, 0) - 1.0 / 6.0).abs() < 1e-12, "{}", w(3, 0));
        // 2 and 3 are not linked in either direction
        assert_eq!(w(2, 3), 0.0);
        assert_eq!(w(0, 1), w(1, 0));
    }

    #[test]
    fn phenograph_is_seed_reproducible_and_flags_tiny_clusters_as_unclustered() {
        let mut rng = Rng::new(8);
        let mut x = Vec::new();
        for i in 0..400 {
            let c = (i % 2) as f64 * 10.0;
            x.extend([c + rng.next_f64(), rng.next_f64()]);
        }
        let a = phenograph(&x, 400, 2, 15, 10, 3, 4).unwrap();
        let b = phenograph(&x, 400, 2, 15, 10, 3, 4).unwrap();
        assert_eq!(a.labels, b.labels);
        assert!(a.n_clusters >= 2, "uniform blobs may split into several communities, never fewer than the 2 blobs");
        for c in 0..a.n_clusters {
            let blobs: std::collections::HashSet<usize> = (0..400).filter(|&i| a.labels[i] == Some(c)).map(|i| i % 2).collect();
            assert_eq!(blobs.len(), 1, "cluster {c} mixes the two separated blobs");
        }
        assert!(a.modularity > 0.4);
        // an absurd min size drops everything to unclustered
        let none = phenograph(&x, 400, 2, 15, 1_000, 3, 4).unwrap();
        assert!(none.labels.iter().all(|l| l.is_none()) && none.n_clusters == 0);
    }
}
