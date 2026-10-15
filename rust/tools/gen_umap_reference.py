r"""Writes rust/crates/sf-core/tests/reference_umap.txt from umap-learn (the library the Python engine uses):

  * `a, b` from umap's own curve fit for several min_dist values;
  * the fuzzy simplicial set (sigmas, rhos and every edge weight) umap-learn builds from exact k-nearest
    neighbours of 600 cells x 6 markers, so the Rust graph can be pinned to it edge for edge;
  * umap-learn's embedding of 1500 cells and its trustworthiness, for layout quality comparison.

    engine\py-env\Scripts\python.exe rust\tools\gen_umap_reference.py
"""
import os
import numpy as np

HERE = os.path.dirname(__file__)
OUT = os.path.join(HERE, "..", "crates", "sf-core", "tests", "reference_umap.txt")
SRC = os.path.join(HERE, "..", "crates", "sf-core", "tests", "reference_embed.txt")


def main():
    import umap
    from umap.umap_ import find_ab_params, fuzzy_simplicial_set
    from sklearn.manifold import trustworthiness
    from sklearn.neighbors import NearestNeighbors

    rows = [l.split() for l in open(SRC).read().splitlines()[3:]]
    d = len(rows[0]) - 5
    Z = np.array([[float(v) for v in r[:d]] for r in rows])
    lines = []
    for md in (0.0, 0.1, 0.5, 1.0):
        a, b = find_ab_params(1.0, md)
        lines.append("ab %r %r %r" % (md, float(a), float(b)))

    k = 15
    Zg = Z[:600]
    nn = NearestNeighbors(n_neighbors=k, algorithm="brute").fit(Zg)
    dists, idx = nn.kneighbors(Zg)             # column 0 is the point itself
    assert (idx[:, 0] == np.arange(len(Zg))).all(), "self must be first"
    # sklearn computes self-distance with a rounding error on some rows; umap would then take that ~1e-8 as the
    # nearest non-zero distance (rho). The point is at distance exactly 0 from itself.
    dists[:, 0] = 0.0
    graph, sigmas, rhos = fuzzy_simplicial_set(Zg, k, np.random.RandomState(0), "euclidean", knn_indices=idx, knn_dists=dists)
    g = graph.tocoo()
    lines.append("graph %d %d %d" % (len(Zg), d, g.nnz))
    for i in range(len(Zg)):
        lines.append("z " + " ".join(repr(float(v)) for v in Zg[i]) + " %r %r" % (float(sigmas[i]), float(rhos[i])))
    for i, j, w in zip(g.row, g.col, g.data):
        if i < j:
            lines.append("w %d %d %r" % (i, j, float(w)))

    Zl = Z[:1500]
    Y = umap.UMAP(n_neighbors=15, min_dist=0.1, random_state=42).fit_transform(Zl)
    t = trustworthiness(Zl, Y, n_neighbors=12)
    lines.append("embed %d %d %r" % (len(Zl), d, float(t)))
    for z, y in zip(Zl, Y):
        lines.append("y " + " ".join(repr(float(v)) for v in z) + " %r %r" % (float(y[0]), float(y[1])))
    with open(OUT, "w", newline="\n") as fh:
        fh.write("\n".join(lines) + "\n")
    print("umap-learn trustworthiness %.4f; graph edges %d" % (t, g.nnz // 2))
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
