//! Sample classifier: PCA of per-sample feature vectors and, when groups are given, a Random Forest with
//! out-of-bag accuracy and permutation importance. Port of the engine's `run_classifier`.
//!
//! * Features are standardised with the population SD (`StandardScaler`, a constant feature gets scale 1).
//! * PCA is exact (checked against scikit-learn): scores, loadings and variance ratios, with scikit-learn's sign
//!   convention (the largest-magnitude loading of each component is positive).
//! * The forest is a seeded CART forest (Gini, bootstrap rows, `sqrt(p)` candidate features per split, fully
//!   grown). Its trees cannot reproduce scikit-learn's random draws, so it is checked statistically (accuracy and
//!   which feature matters), and it is fully reproducible for a given seed.

use crate::linalg::sym_eig;
use crate::rng::Rng;

// ---- standardise + PCA ---------------------------------------------------------------------------------------------

/// Column means and population SDs; a zero-spread column gets scale 1 so it standardises to zeros.
fn standardise(x: &[f64], n: usize, p: usize) -> Vec<f64> {
    let mut out = vec![0.0; n * p];
    for j in 0..p {
        let mean = (0..n).map(|i| x[i * p + j]).sum::<f64>() / n as f64;
        let var = (0..n).map(|i| (x[i * p + j] - mean).powi(2)).sum::<f64>() / n as f64;
        let sd = var.sqrt();
        let scale = if sd <= 1e-12 * mean.abs().max(1.0) { 1.0 } else { sd };
        for i in 0..n {
            out[i * p + j] = (x[i * p + j] - mean) / scale;
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct Pca {
    /// `n x k` scores, row per sample.
    pub scores: Vec<Vec<f64>>,
    /// `p x k` loadings (unit-length components), row per feature.
    pub loadings: Vec<Vec<f64>>,
    pub var_ratio: Vec<f64>,
}

/// PCA of the standardised features: at most 2 components, never more than `p` or `n - 1`.
pub fn pca(x: &[f64], n: usize, p: usize) -> Result<(Pca, Vec<f64>), String> {
    if n < 2 || p < 1 {
        return Err("Need at least 1 feature and 2 samples for PCA.".into());
    }
    if x.len() != n * p || x.iter().any(|v| !v.is_finite()) {
        return Err("The feature matrix has the wrong shape or non-finite values.".into());
    }
    let xs = standardise(x, n, p);
    let k = 2.min(p).min(n - 1);
    // Gram matrix (n x n): its eigenvectors are the left singular vectors, eigenvalues the squared singular values
    let mut g = vec![0.0; n * n];
    for a in 0..n {
        for b in a..n {
            let v: f64 = (0..p).map(|j| xs[a * p + j] * xs[b * p + j]).sum();
            g[a * n + b] = v;
            g[b * n + a] = v;
        }
    }
    let (vals, vecs) = sym_eig(&g, n).map_err(|e| e.to_string())?;
    let total: f64 = vals.iter().map(|v| v.max(0.0)).sum();
    let mut scores = vec![vec![0.0; k]; n];
    let mut loadings = vec![vec![0.0; k]; p];
    let mut var_ratio = vec![0.0; k];
    for c in 0..k {
        let lam = vals[c].max(0.0);
        var_ratio[c] = if total > 0.0 { lam / total } else { 0.0 };
        let s = lam.sqrt();
        if s <= 1e-12 {
            continue; // no variance left: the component is zero
        }
        let u: Vec<f64> = (0..n).map(|i| vecs[i * n + c]).collect();
        let mut load: Vec<f64> = (0..p).map(|j| (0..n).map(|i| xs[i * p + j] * u[i]).sum::<f64>() / s).collect();
        let mut sc: Vec<f64> = u.iter().map(|v| v * s).collect();
        // scikit-learn's svd_flip: the largest |loading| of a component is positive
        let big = load.iter().cloned().fold(0.0_f64, |a, b| if b.abs() > a.abs() { b } else { a });
        if big < 0.0 {
            load.iter_mut().for_each(|v| *v = -*v);
            sc.iter_mut().for_each(|v| *v = -*v);
        }
        for i in 0..n {
            scores[i][c] = sc[i];
        }
        for j in 0..p {
            loadings[j][c] = load[j];
        }
    }
    Ok((Pca { scores, loadings, var_ratio }, xs))
}

// ---- random forest -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Leaf(Vec<f64>),
    Split { feature: usize, threshold: f64, left: usize, right: usize },
}

#[derive(Debug, Clone)]
struct Tree {
    nodes: Vec<Node>,
}

impl Tree {
    fn proba(&self, row: &[f64]) -> &[f64] {
        let mut at = 0;
        loop {
            match &self.nodes[at] {
                Node::Leaf(p) => return p,
                Node::Split { feature, threshold, left, right } => at = if row[*feature] <= *threshold { *left } else { *right },
            }
        }
    }
}

fn gini(counts: &[f64], total: f64) -> f64 {
    if total <= 0.0 {
        return 0.0;
    }
    1.0 - counts.iter().map(|c| (c / total) * (c / total)).sum::<f64>()
}

/// The best split of `rows` on one feature: `(weighted child impurity, threshold)`.
fn best_split_on(x: &[f64], p: usize, y: &[usize], k: usize, rows: &[usize], f: usize) -> Option<(f64, f64)> {
    let mut order: Vec<usize> = rows.to_vec();
    order.sort_by(|&a, &b| x[a * p + f].total_cmp(&x[b * p + f]));
    let n = order.len() as f64;
    let mut right = vec![0.0; k];
    for &r in &order {
        right[y[r]] += 1.0;
    }
    let mut left = vec![0.0; k];
    let mut best: Option<(f64, f64)> = None;
    for w in 0..order.len() - 1 {
        let r = order[w];
        left[y[r]] += 1.0;
        right[y[r]] -= 1.0;
        let (a, b) = (x[r * p + f], x[order[w + 1] * p + f]);
        if a == b {
            continue;
        }
        let nl = (w + 1) as f64;
        let imp = (nl * gini(&left, nl) + (n - nl) * gini(&right, n - nl)) / n;
        if best.map_or(true, |(bi, _)| imp < bi - 1e-15) {
            best = Some((imp, a + (b - a) / 2.0));
        }
    }
    best
}

fn build_tree(x: &[f64], p: usize, y: &[usize], k: usize, boot: &[usize], max_features: usize, rng: &mut Rng) -> Tree {
    let mut nodes: Vec<Node> = Vec::new();
    // (rows, index of the slot to fill)
    let mut stack: Vec<(Vec<usize>, usize)> = vec![(boot.to_vec(), 0)];
    nodes.push(Node::Leaf(vec![]));
    while let Some((rows, slot)) = stack.pop() {
        let mut counts = vec![0.0; k];
        for &r in &rows {
            counts[y[r]] += 1.0;
        }
        let total = rows.len() as f64;
        let leaf = |counts: &[f64]| Node::Leaf(counts.iter().map(|c| c / total).collect());
        if counts.iter().filter(|c| **c > 0.0).count() <= 1 || rows.len() < 2 {
            nodes[slot] = leaf(&counts);
            continue;
        }
        // draw features without replacement; keep going past max_features only while no valid split exists
        let mut feats: Vec<usize> = (0..p).collect();
        for i in (1..p).rev() {
            feats.swap(i, rng.below(i + 1));
        }
        let mut best: Option<(f64, f64, usize)> = None;
        for (tried, &f) in feats.iter().enumerate() {
            if tried >= max_features && best.is_some() {
                break;
            }
            if let Some((imp, th)) = best_split_on(x, p, y, k, &rows, f) {
                if best.map_or(true, |(bi, _, _)| imp < bi - 1e-15) {
                    best = Some((imp, th, f));
                }
            }
        }
        let Some((_, th, f)) = best else {
            nodes[slot] = leaf(&counts);
            continue;
        };
        let (l, r): (Vec<usize>, Vec<usize>) = rows.iter().partition(|&&i| x[i * p + f] <= th);
        let (li, ri) = (nodes.len(), nodes.len() + 1);
        nodes.push(Node::Leaf(vec![]));
        nodes.push(Node::Leaf(vec![]));
        nodes[slot] = Node::Split { feature: f, threshold: th, left: li, right: ri };
        stack.push((l, li));
        stack.push((r, ri));
    }
    Tree { nodes }
}

#[derive(Debug, Clone)]
pub struct Forest {
    trees: Vec<Tree>,
    in_bag: Vec<Vec<bool>>,
    k: usize,
    p: usize,
}

impl Forest {
    /// Fit on `n x p` features `x` and class indices `y` (`0..k`).
    pub fn fit(x: &[f64], n: usize, p: usize, y: &[usize], k: usize, n_trees: usize, seed: u64) -> Forest {
        let mut rng = Rng::new(seed);
        let max_features = ((p as f64).sqrt().floor() as usize).max(1);
        let mut trees = Vec::with_capacity(n_trees);
        let mut in_bag = Vec::with_capacity(n_trees);
        for _ in 0..n_trees {
            let boot: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
            let mut bag = vec![false; n];
            boot.iter().for_each(|&i| bag[i] = true);
            trees.push(build_tree(x, p, y, k, &boot, max_features, &mut rng));
            in_bag.push(bag);
        }
        Forest { trees, in_bag, k, p }
    }

    /// Mean class probabilities of the trees for one row.
    pub fn proba(&self, row: &[f64]) -> Vec<f64> {
        let mut out = vec![0.0; self.k];
        for t in &self.trees {
            for (o, v) in out.iter_mut().zip(t.proba(row)) {
                *o += v;
            }
        }
        out.iter_mut().for_each(|v| *v /= self.trees.len() as f64);
        out
    }

    pub fn predict(&self, row: &[f64]) -> usize {
        argmax(&self.proba(row))
    }

    /// Accuracy of each sample predicted only by the trees that never saw it (`None` for a sample every tree saw).
    /// Returns `(accuracy, samples counted)`.
    pub fn oob_accuracy(&self, x: &[f64], y: &[usize]) -> Option<(f64, usize)> {
        let n = y.len();
        let (mut hit, mut seen) = (0usize, 0usize);
        for i in 0..n {
            let row = &x[i * self.p..(i + 1) * self.p];
            let mut acc = vec![0.0; self.k];
            let mut votes = 0;
            for (t, bag) in self.trees.iter().zip(&self.in_bag) {
                if !bag[i] {
                    votes += 1;
                    for (a, v) in acc.iter_mut().zip(t.proba(row)) {
                        *a += v;
                    }
                }
            }
            if votes == 0 {
                continue;
            }
            seen += 1;
            if argmax(&acc) == y[i] {
                hit += 1;
            }
        }
        (seen > 0).then(|| (hit as f64 / seen as f64, seen))
    }

    fn accuracy(&self, x: &[f64], y: &[usize]) -> f64 {
        let n = y.len();
        (0..n).filter(|&i| self.predict(&x[i * self.p..(i + 1) * self.p]) == y[i]).count() as f64 / n as f64
    }

    /// Mean drop in accuracy when one feature's column is shuffled, over `repeats` shuffles (scikit-learn's
    /// `permutation_importance`, scored on the data it is given).
    pub fn permutation_importance(&self, x: &[f64], y: &[usize], repeats: usize, seed: u64) -> Vec<f64> {
        let n = y.len();
        let base = self.accuracy(x, y);
        let mut rng = Rng::new(seed);
        let mut work = x.to_vec();
        (0..self.p)
            .map(|f| {
                let mut total = 0.0;
                for _ in 0..repeats {
                    let mut col: Vec<f64> = (0..n).map(|i| x[i * self.p + f]).collect();
                    for i in (1..n).rev() {
                        col.swap(i, rng.below(i + 1));
                    }
                    for i in 0..n {
                        work[i * self.p + f] = col[i];
                    }
                    total += self.accuracy(&work, y);
                }
                for i in 0..n {
                    work[i * self.p + f] = x[i * self.p + f];
                }
                base - total / repeats as f64
            })
            .collect()
    }
}

fn argmax(v: &[f64]) -> usize {
    let mut b = 0;
    for (i, x) in v.iter().enumerate() {
        if *x > v[b] {
            b = i;
        }
    }
    b
}

// ---- the whole classifier --------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ClassifierResult {
    pub method: String,
    pub sample_names: Vec<String>,
    pub groups: Vec<Option<String>>,
    pub scores: Vec<Vec<f64>>,
    pub feature_names: Vec<String>,
    pub loadings: Vec<Vec<f64>>,
    pub var_explained: Vec<f64>,
    /// Indices of the (up to 8) features with the largest loading magnitude.
    pub top_loading: Vec<usize>,
    pub oob_accuracy: Option<f64>,
    /// Samples the OOB accuracy was computed on, and how many samples the forest was trained on.
    pub oob_counted: usize,
    pub rf_samples: usize,
    /// Up to 10 `(feature, mean accuracy drop)` pairs, largest first.
    pub importance: Vec<(String, f64)>,
    pub n_samples: usize,
    pub n_features: usize,
}

/// `features` is row-major `n x p`. `groups[i]` is the label of sample `i` (`None` = unlabelled: it is placed on
/// the PCA plot but not used to train or score the forest). With fewer than two groups only PCA is run.
pub fn classify(sample_names: &[String], feature_names: &[String], features: &[f64], groups: &[Option<String>], seed: u64) -> Result<ClassifierResult, String> {
    let (n, p) = (sample_names.len(), feature_names.len());
    if n < 2 {
        return Err("Need at least 2 samples with gate feature vectors.".into());
    }
    if p == 0 {
        return Err("Feature vectors are empty — gate at least one population first.".into());
    }
    if groups.len() != n {
        return Err("One group label (or none) per sample is required.".into());
    }
    let (pc, xs) = pca(features, n, p)?;
    let mut distinct: Vec<String> = groups.iter().flatten().cloned().collect();
    distinct.sort();
    distinct.dedup();
    let mut method = "PCA".to_string();
    let (mut oob, mut oob_counted, mut rf_samples, mut importance) = (None, 0, 0, vec![]);
    if distinct.len() >= 2 {
        let rows: Vec<usize> = (0..n).filter(|&i| groups[i].is_some()).collect();
        let y: Vec<usize> = rows.iter().map(|&i| distinct.iter().position(|d| Some(d) == groups[i].as_ref()).unwrap()).collect();
        let smallest = distinct.iter().map(|d| groups.iter().flatten().filter(|g| *g == d).count()).min().unwrap();
        if smallest < 3 {
            return Err(format!("Need at least 3 samples per group for a Random Forest classification (smallest group has {smallest})."));
        }
        let xr: Vec<f64> = rows.iter().flat_map(|&i| xs[i * p..(i + 1) * p].iter().copied()).collect();
        let forest = Forest::fit(&xr, rows.len(), p, &y, distinct.len(), 200, seed);
        if let Some((acc, counted)) = forest.oob_accuracy(&xr, &y) {
            oob = Some(acc);
            oob_counted = counted;
        }
        rf_samples = rows.len();
        let imp = forest.permutation_importance(&xr, &y, 30, seed.wrapping_add(1));
        let mut ranked: Vec<(String, f64)> = feature_names.iter().cloned().zip(imp).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        ranked.truncate(10);
        importance = ranked;
        method = match oob {
            Some(a) => format!("PCA + Random Forest (OOB accuracy {a:.2})"),
            None => "PCA + Random Forest".to_string(),
        };
    }
    let k = pc.var_ratio.len();
    let mag: Vec<f64> = (0..p).map(|j| (0..k).map(|c| pc.loadings[j][c].powi(2)).sum::<f64>().sqrt()).collect();
    let mut order: Vec<usize> = (0..p).collect();
    order.sort_by(|&a, &b| mag[b].total_cmp(&mag[a]));
    order.truncate(8);
    Ok(ClassifierResult {
        method,
        sample_names: sample_names.to_vec(),
        groups: groups.to_vec(),
        scores: pc.scores,
        feature_names: feature_names.to_vec(),
        loadings: pc.loadings,
        var_explained: pc.var_ratio,
        top_loading: order,
        oob_accuracy: oob,
        oob_counted,
        rf_samples,
        importance,
        n_samples: n,
        n_features: p,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i}")).collect()
    }

    /// Two groups separated on feature 0 only; the other features are noise.
    fn separable(n_per: usize, p: usize, seed: u64) -> (Vec<f64>, Vec<Option<String>>) {
        let mut rng = Rng::new(seed);
        let mut x = Vec::new();
        let mut g = Vec::new();
        for i in 0..2 * n_per {
            let grp = i % 2;
            x.push(grp as f64 * 5.0 + rng.next_f64());
            for _ in 1..p {
                x.push(rng.next_f64() * 5.0);
            }
            g.push(Some(if grp == 0 { "A".to_string() } else { "B".to_string() }));
        }
        (x, g)
    }

    #[test]
    fn pca_of_a_rank_one_dataset_has_one_component_carrying_everything() {
        // feature 2 = 2 x feature 1: all variance on one axis
        let x = [1.0, 2.0, 2.0, 4.0, 3.0, 6.0, 4.0, 8.0];
        let (pc, _) = pca(&x, 4, 2).unwrap();
        assert!((pc.var_ratio[0] - 1.0).abs() < 1e-12 && pc.var_ratio[1].abs() < 1e-12, "{:?}", pc.var_ratio);
        // standardised features are identical, so both loadings are +1/sqrt(2) (positive by the sign convention)
        assert!((pc.loadings[0][0] - 0.5f64.sqrt()).abs() < 1e-9 && (pc.loadings[1][0] - 0.5f64.sqrt()).abs() < 1e-9);
        // scores are centred
        assert!(pc.scores.iter().map(|s| s[0]).sum::<f64>().abs() < 1e-9);
        assert!(pca(&x, 1, 2).is_err() && pca(&[f64::NAN, 1.0, 2.0, 3.0], 2, 2).is_err());
    }

    #[test]
    fn a_constant_feature_standardises_to_zero_and_cannot_dominate() {
        let x = [1.0, 7.0, 2.0, 7.0, 3.0, 7.0, 5.0, 7.0];
        let (pc, xs) = pca(&x, 4, 2).unwrap();
        assert!(xs.chunks(2).all(|r| r[1] == 0.0));
        assert!((pc.var_ratio[0] - 1.0).abs() < 1e-12);
        assert!(pc.loadings[1][0].abs() < 1e-12);
    }

    #[test]
    fn the_forest_finds_the_informative_feature_and_scores_honestly() {
        let (n, p) = (12, 6);
        let (x, g) = separable(n, p, 3);
        let r = classify(&names("s", 2 * n), &names("f", p), &x, &g, 1).unwrap();
        assert!(r.oob_accuracy.unwrap() >= 0.9, "{:?}", r.oob_accuracy);
        assert_eq!(r.importance[0].0, "f0", "{:?}", r.importance);
        assert!(r.importance[0].1 > 0.2 && r.importance[1].1 < r.importance[0].1);
        assert!(r.method.starts_with("PCA + Random Forest (OOB accuracy"));
        // the same seed is the same forest
        let r2 = classify(&names("s", 2 * n), &names("f", p), &x, &g, 1).unwrap();
        assert_eq!((r.oob_accuracy, r.importance.clone()), (r2.oob_accuracy, r2.importance));
        // labels that carry no information score near chance, not near 1
        let mut accs = Vec::new();
        for seed in 0..6 {
            let mut rng = Rng::new(100 + seed);
            let xn: Vec<f64> = (0..2 * n * p).map(|_| rng.next_f64()).collect();
            let gn: Vec<Option<String>> = (0..2 * n).map(|i| Some(if i % 2 == 0 { "A".into() } else { "B".into() })).collect();
            accs.push(classify(&names("s", 2 * n), &names("f", p), &xn, &gn, seed).unwrap().oob_accuracy.unwrap());
        }
        let mean = accs.iter().sum::<f64>() / accs.len() as f64;
        assert!(mean < 0.75, "no signal, so OOB accuracy should be near 0.5: {accs:?}");
    }

    #[test]
    fn group_rules_match_the_engine_and_unlabelled_samples_stay_out_of_the_forest() {
        let (n, p) = (5, 3);
        let (x, mut g) = separable(n, p, 9);
        // a group with fewer than 3 samples is refused with the engine's message
        g[0] = Some("C".into());
        let e = classify(&names("s", 2 * n), &names("f", p), &x, &g, 1).unwrap_err();
        assert!(e.contains("at least 3 samples per group") && e.contains("smallest group has 1"), "{e}");
        // one group only: PCA alone
        let one: Vec<Option<String>> = vec![Some("A".into()); 2 * n];
        assert_eq!(classify(&names("s", 2 * n), &names("f", p), &x, &one, 1).unwrap().method, "PCA");
        // unlabelled samples are plotted but do not train or score the forest
        let (x2, mut g2) = separable(6, p, 4);
        g2[0] = None;
        g2[1] = None;
        let r = classify(&names("s", 12), &names("f", p), &x2, &g2, 1).unwrap();
        assert_eq!((r.rf_samples, r.n_samples), (10, 12));
        assert_eq!(r.scores.len(), 12);
        assert!(classify(&names("s", 1), &names("f", p), &x[..p], &g[..1], 1).is_err());
        assert!(classify(&names("s", 4), &[], &[], &vec![None; 4], 1).unwrap_err().contains("gate at least one"));
    }
}
