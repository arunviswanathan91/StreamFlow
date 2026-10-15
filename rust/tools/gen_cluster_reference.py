r"""Writes rust/crates/sf-core/tests/reference_cluster.txt: a data set with known populations plus the labels the
Python engine's own backends (FlowSOM, k-means) assign to it. The Rust test checks that its clustering agrees
with those labels as well as the Python methods agree with the truth (adjusted Rand index).

    engine\py-env\Scripts\python.exe rust\tools\gen_cluster_reference.py
"""
import os
import numpy as np
import anndata as ad
import flowsom as fs
from sklearn.cluster import MiniBatchKMeans
from sklearn.metrics import adjusted_rand_score as ari

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "sf-core", "tests", "reference_cluster.txt")
rs = np.random.default_rng(7)
D, per = 6, 600
centres = rs.uniform(-4, 4, (5, D))
X = np.vstack([rs.normal(c, 1.1, (per, D)) for c in centres])
truth = np.repeat(np.arange(5), per)
perm = rs.permutation(len(X))
X, truth = X[perm], truth[perm]
Z = (X - X.mean(0)) / X.std(0)                      # the engine's z-score step (population SD)

f = fs.FlowSOM(ad.AnnData(Z), n_clusters=5, cols_to_use=list(range(D)), seed=42)
lab_fs = np.asarray(f.metacluster_labels, dtype=int)
lab_km = MiniBatchKMeans(n_clusters=5, random_state=42, n_init=3).fit_predict(Z)
print("python flowsom vs truth ARI", round(ari(truth, lab_fs), 4))
print("python kmeans  vs truth ARI", round(ari(truth, lab_km), 4))
print("python flowsom vs kmeans ARI", round(ari(lab_fs, lab_km), 4))

with open(OUT, "w", newline="\n") as fh:
    fh.write("dims %d %d\n" % X.shape)
    fh.write("ari_flowsom_truth %r\nari_kmeans_truth %r\n" % (ari(truth, lab_fs), ari(truth, lab_km)))
    for row, t, a, b in zip(X, truth, lab_fs, lab_km):
        fh.write(" ".join(repr(float(v)) for v in row) + " %d %d %d\n" % (t, a, b))
print("wrote", os.path.normpath(OUT))
