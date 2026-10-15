"""Writes rust/crates/sf-core/tests/reference_classify.txt from the engine's own scikit-learn pipeline.
PCA cases are exact; Random Forest cases record the MEAN OOB accuracy and mean permutation importance over 20
seeds (a single forest is one random draw, so the Rust forest is compared with the average).
Run from the repo root:  engine/py-env/Scripts/python.exe -I rust/tools/gen_classify_reference.py
"""
import numpy as np
from sklearn.preprocessing import StandardScaler
from sklearn.decomposition import PCA
from sklearn.ensemble import RandomForestClassifier
from sklearn.inspection import permutation_importance

rng = np.random.default_rng(424242)
f = lambda a: " ".join(repr(float(v)) for v in np.asarray(a).ravel())
out = []


def pca_case(n, p, tag):
    X = rng.normal(0, 1, (n, p)) * rng.uniform(0.5, 5, p) + rng.uniform(-3, 3, p)
    X[:, 0] += 0.8 * X[:, 1]  # some correlation so the components are not trivial
    Xs = StandardScaler().fit_transform(X)
    k = min(2, p, n - 1)
    m = PCA(n_components=k)
    Z = m.fit_transform(Xs)
    L = m.components_.T
    out.append(f"pca {tag} {n} {p} {k} ; {f(X)} ; {f(Z)} ; {f(L)} ; {f(m.explained_variance_ratio_)}")


pca_case(8, 5, "8x5")
pca_case(12, 7, "12x7")
pca_case(6, 10, "wide6x10")
pca_case(20, 3, "20x3")
pca_case(4, 2, "4x2")


def rf_case(tag, n_per, p, informative, shift):
    n = 2 * n_per
    y = np.array([i % 2 for i in range(n)])
    X = rng.normal(0, 1, (n, p))
    for j in informative:
        X[:, j] += shift * y
    Xs = StandardScaler().fit_transform(X)
    oobs, imps = [], []
    for seed in range(20):
        rf = RandomForestClassifier(n_estimators=200, random_state=seed, oob_score=True, bootstrap=True).fit(Xs, y)
        perm = permutation_importance(rf, Xs, y, n_repeats=10, random_state=seed)
        oobs.append(rf.oob_score_)
        imps.append(perm.importances_mean)
    out.append(f"rf {tag} {n} {p} ; {f(X)} ; {' '.join(map(str, y))} ; {float(np.mean(oobs))!r} ; {f(np.mean(imps, axis=0))}")


rf_case("one-informative", 12, 6, [0], 4.0)
rf_case("two-informative", 12, 8, [1, 3], 3.5)
rf_case("null", 12, 6, [], 0.0)
rf_case("weak", 15, 6, [2], 1.5)
open("rust/crates/sf-core/tests/reference_classify.txt", "w").write("\n".join(out) + "\n")
print(len(out), "cases")
