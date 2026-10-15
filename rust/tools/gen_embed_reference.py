r"""Writes rust/crates/sf-core/tests/reference_embed.txt for the t-SNE (and later UMAP) tests:

  * 1500 cells x 6 markers (z-scored);
  * a fixed 2-D embedding (the first two principal components) with scikit-learn's trustworthiness at k=12,
    which pins the Rust `trustworthiness` to the library definition;
  * openTSNE's embedding of the same cells (perplexity 30, seed 42) and its trustworthiness, so the Rust
    t-SNE can be judged against the implementation the Python engine uses.

    engine\py-env\Scripts\python.exe rust\tools\gen_embed_reference.py
"""
import os
import numpy as np

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "sf-core", "tests", "reference_embed.txt")
SRC = os.path.join(os.path.dirname(__file__), "..", "crates", "sf-core", "tests", "reference_cluster.txt")


def main():
    from sklearn.manifold import trustworthiness
    from openTSNE import TSNE

    rows = [l.split() for l in open(SRC).read().splitlines()[3:]]
    d = len(rows[0]) - 3
    X = np.array([[float(v) for v in r[:d]] for r in rows])[:1500]
    truth = np.array([int(r[d]) for r in rows])[:1500]
    Z = (X - X.mean(0)) / X.std(0)

    U, S, Vt = np.linalg.svd(Z - Z.mean(0), full_matrices=False)
    pca = (Z - Z.mean(0)) @ Vt[:2].T
    t_pca = trustworthiness(Z, pca, n_neighbors=12)

    emb = TSNE(perplexity=30, n_jobs=1, random_state=42, verbose=False).fit(Z)
    Y = np.asarray(emb)
    t_tsne = trustworthiness(Z, Y, n_neighbors=12)
    print("sklearn trustworthiness: pca %.4f  openTSNE %.4f" % (t_pca, t_tsne))

    with open(OUT, "w", newline="\n") as fh:
        fh.write("cells %d %d\n" % Z.shape)
        fh.write("trust_pca %r\ntrust_tsne %r\n" % (float(t_pca), float(t_tsne)))
        for z, t, p, y in zip(Z, truth, pca, Y):
            fh.write(" ".join(repr(float(v)) for v in z) + " %d %r %r %r %r\n" % (t, float(p[0]), float(p[1]), float(y[0]), float(y[1])))
    print("wrote", os.path.normpath(OUT))


if __name__ == "__main__":
    main()
