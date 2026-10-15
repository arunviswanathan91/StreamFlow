"""Tests for engine/spectral.py. Run: python -m pytest engine/tests -q  (or python engine/tests/test_spectral.py)."""
import os, sys
import numpy as np
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import spectral as sp

D = 12
def _sigs():
    x = np.arange(D)
    peaks = [1, 4, 7, 10]
    M = np.array([np.exp(-0.5 * ((x - p) / 1.4) ** 2) for p in peaks])
    return M / M.max(axis=1, keepdims=True)

def _control(M, f, n=3000, seed=0, bright=5000.0, af=None):
    rng = np.random.default_rng(seed)
    pos = rng.random(n) < 0.5
    ab = np.where(pos, bright * rng.lognormal(0, 0.2, n), 0.0)
    ev = np.outer(ab, M[f]) + (af if af is not None else 0)
    return rng.poisson(np.clip(ev, 0, None) + 20).astype(float)

def test_signature_recovered():
    M = _sigs()
    for f in range(M.shape[0]):
        r = sp.build_signature(_control(M, f, seed=f))
        assert r["ok"], r["reason"]
        assert np.allclose(r["signature"], M[f], atol=0.06)
        assert r["signature"].max() == 1.0

def test_unmix_recovers_abundances_ols_and_wls():
    M = _sigs(); rng = np.random.default_rng(1)
    truth = rng.uniform(0, 4000, (5000, 4))
    ev = rng.poisson(truth @ M + 30).astype(float)
    for m in ("ols", "wls"):
        ab, W = sp.unmix(ev, M, m)
        assert W.shape == (4, D)
        err = np.median(np.abs(ab - truth))
        assert err < 120, (m, err)

def test_wls_not_worse_than_ols_on_poisson():
    M = _sigs(); rng = np.random.default_rng(2)
    truth = np.zeros((6000, 4)); truth[:, 0] = rng.uniform(0, 20000, 6000); truth[:, 2] = rng.uniform(0, 300, 6000)
    ev = rng.poisson(truth @ M + 10).astype(float)
    e_ols = np.mean((sp.unmix(ev, M, "ols")[0] - truth) ** 2)
    e_wls = np.mean((sp.unmix(ev, M, "wls")[0] - truth) ** 2)
    assert e_wls <= e_ols * 1.05

def test_ols_is_wls_with_unit_weights():
    M = _sigs()
    assert np.allclose(sp.unmixing_matrix(M), sp.unmixing_matrix(M, np.ones(D)))
    assert np.allclose(sp.unmixing_matrix(M) @ M.T, np.eye(4), atol=1e-9)  # W M^T = I

def test_qc_flags_collinear_pair():
    M = _sigs(); rng = np.random.default_rng(0)
    # genuinely NEAR-collinear: 97% of row 1 plus 3% of an independent spectrum (not of other rows)
    M2 = np.vstack([M, 0.97 * M[1] + 0.03 * rng.random(M.shape[1])])
    q = sp.signature_qc(M2)
    assert q["cosine"][1, 4] > 0.99
    assert np.isfinite(q["collinearity"]).all()
    assert q["collinearity"][1] > 5 * q["collinearity"][0] and q["collinearity"][4] > 5 * q["collinearity"][0]
    assert np.allclose(np.diag(q["cosine"]), 1.0)

def test_qc_dependent_set_is_infinite_not_zero():
    # exact linear combination of existing rows: the old inverse-Gram code reported 0.0 here
    M = _sigs(); M3 = np.vstack([M, 0.97 * M[1] + 0.03 * M[2]])
    c = sp.signature_qc(M3)["collinearity"]
    assert np.isinf(c[[1, 2, 4]]).all(), c
    assert np.isfinite(c[[0, 3]]).all() and (c[[0, 3]] < 2).all()

def test_saturation_excluded():
    e = np.random.default_rng(0).uniform(0, 1000, (1000, 5)); e[:50, 2] = 1000
    m = sp.saturation_mask(e, ceiling=np.full(5, 1000.0))
    assert (~m).sum() >= 50 and not m[:50].any()

def test_peak_detector_dim_control():
    M = _sigs(); ev = _control(M, 2, bright=400.0, seed=3)
    assert sp.peak_detector(ev) in (6, 7, 8)

def test_af_library_deterministic_and_per_cell_af_improves():
    M = _sigs(); rng = np.random.default_rng(4)
    afs = np.abs(rng.normal(0, 1, (2, D))) ; afs = afs / afs.max(axis=1, keepdims=True)
    labels = rng.integers(0, 2, 4000)
    unst = rng.poisson(300 * afs[labels] + 10).astype(float)
    l1, c1 = sp.extract_af_library(unst, k=2, seed=7); l2, c2 = sp.extract_af_library(unst, k=2, seed=7)
    assert np.array_equal(l1, l2) and np.array_equal(c1, c2)
    truth = rng.uniform(0, 2000, (3000, 4)); lab = rng.integers(0, 2, 3000)
    ev = rng.poisson(truth @ M + 400 * afs[lab] + 10).astype(float)
    plain, _ = sp.unmix(ev, M, "wls")
    ab, idx, res = sp.unmix_with_af(ev, M, l1, "wls")
    assert ab.shape == (3000, 5) and set(np.unique(idx)) <= {0, 1}
    assert np.mean((ab[:, :4] - truth) ** 2) < np.mean((plain - truth) ** 2)

def test_unknown_method_raises():
    try: sp.unmix(np.ones((3, D)), _sigs(), "nope"); assert False
    except ValueError as e: assert "nope" in str(e)

if __name__ == "__main__":
    for k, v in list(globals().items()):
        if k.startswith("test_"): v(); print("ok", k)
