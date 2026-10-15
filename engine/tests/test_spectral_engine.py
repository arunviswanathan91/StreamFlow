"""Drives the real spectral engine commands against synthetic samples (no flowkit/FCS needed:
`_sample_by_name` is the only seam stubbed). Run: python engine/tests/test_spectral_engine.py"""
import os, sys, tempfile
import numpy as np
HERE = os.path.dirname(os.path.abspath(__file__)); ENG = os.path.join(HERE, "..")
sys.path.insert(0, ENG)
sys.argv = [sys.argv[0], "--control-dir", tempfile.mkdtemp()]
import streamflow_engine as E
import spectral as sp

D = 10
CH = ["FSC-A", "SSC-A"] + ["UV%d-A" % k for k in range(D)]
x = np.arange(D)
M_TRUE = np.array([np.exp(-0.5 * ((x - p) / 1.3) ** 2) for p in (1, 4, 7)])
M_TRUE /= M_TRUE.max(axis=1, keepdims=True)

class FakeSample:
    def __init__(self, ev): self._ev = ev; self.pnn_labels = CH
    def get_events(self, source="raw"): return self._ev

def make(ab, af=0.0, seed=0):
    rng = np.random.default_rng(seed)
    f = rng.poisson(ab @ M_TRUE + af + 20).astype(float)
    sc = rng.normal(5e4, 5e3, (ab.shape[0], 2))
    return np.hstack([sc, f])

def control(f, seed):
    rng = np.random.default_rng(seed); n = 4000
    ab = np.zeros((n, 3)); pos = rng.random(n) < 0.5
    ab[pos, f] = 6000 * rng.lognormal(0, 0.2, pos.sum())
    return make(ab, seed=seed)

SAMPLES = {"ctl_A.fcs": control(0, 1), "ctl_B.fcs": control(1, 2), "ctl_C.fcs": control(2, 3),
           "unst.fcs": make(np.zeros((4000, 3)), af=150.0, seed=4)}
rng = np.random.default_rng(9); TRUTH = rng.uniform(0, 3000, (3000, 3))
SAMPLES["full.fcs"] = make(TRUTH, seed=5)
E._sample_by_name = lambda name: FakeSample(SAMPLES[name or "full.fcs"])
E.STATE["meta"] = [{"name": n, "file": n, "pnn": CH, "pns": [""] * len(CH), "events": len(v)} for n, v in SAMPLES.items()]
E.STATE["files"] = list(SAMPLES)

def call(cmd, **a): return E.COMMANDS[cmd]("t", a)

def test_requires_model_first():
    E.STATE["spectral"] = None
    try: call("apply_unmixing", sample="full.fcs"); assert False
    except ValueError as e: assert "build_spectral_model" in str(e)

def test_build_and_unmix_recovers_truth():
    r = call("build_spectral_model", controls=[{"sample": s, "name": n} for s, n in
             (("ctl_A.fcs", "A"), ("ctl_B.fcs", "B"), ("ctl_C.fcs", "C"))], unstained="unst.fcs")
    assert r["names"] == ["A", "B", "C"] and all(c["ok"] for c in r["controls"])
    assert len(r["qc"]["collinearity"]) == 3
    u = call("apply_unmixing", sample="full.fcs", method="wls")
    assert u["channels"] == ["FSC-A", "SSC-A", "A", "B", "C", "Residual"]
    blob = np.fromfile(u["file"], dtype="<f4").reshape(u["rows"], u["cols"])
    assert np.median(np.abs(blob[:, 2:5] - TRUTH)) < 150
    assert u["method"] == "wls" and u["af"] is False

def test_ols_and_af_paths_and_determinism():
    call("extract_af_library", unstained="unst.fcs", k=2, seed=3)
    a = call("apply_unmixing", sample="full.fcs", method="ols", af=True, n=1000, seed=1)
    b = call("apply_unmixing", sample="full.fcs", method="ols", af=True, n=1000, seed=1)
    assert a["channels"][-3:] == ["C", "AF", "Residual"]
    assert np.array_equal(np.fromfile(a["file"], "<f4"), np.fromfile(b["file"], "<f4"))

def test_af_without_library_fails_loudly():
    E.STATE["spectral"]["af"] = None
    try: call("apply_unmixing", sample="full.fcs", af=True); assert False
    except ValueError as e: assert "extract_af_library" in str(e)

def test_export_unmixed_fcs_roundtrip():
    out = os.path.join(tempfile.mkdtemp(), "u.fcs")
    r = call("export_unmixed", sample="full.fcs", file=out)
    raw = open(out, "rb").read()
    assert raw[:6] == b"FCS3.1" and b"$TOT/3000/" in raw
    t0, t1, d0, d1 = (int(raw[i:i + 8]) for i in (10, 18, 26, 34))
    data = np.frombuffer(raw[d0:d1 + 1], "<f4").reshape(3000, len(r["channels"]))
    assert data.shape[1] == 6

def test_reset_clears_model():
    call("reset"); assert E.STATE["spectral"] is None

def test_bad_inputs():
    E.STATE["meta"] = [{"name": n, "file": n, "pnn": CH, "pns": [""] * len(CH), "events": 1} for n in SAMPLES]
    for kw in ({"controls": [{"sample": "nope.fcs"}]}, {"detectors": ["XYZ"]}):
        try: call("build_spectral_model", **kw); assert False
        except ValueError: pass

def test_dependent_signatures_serialise_as_valid_json():
    import json
    # three controls where C is (almost exactly) a mix of A and B: a dependent set
    mix = 0.6 * M_TRUE[0] + 0.4 * M_TRUE[1]
    def ctl_from(sig, seed):
        rng = np.random.default_rng(seed); n = 4000
        ab = np.zeros(n); pos = rng.random(n) < 0.5; ab[pos] = 6000 * rng.lognormal(0, 0.2, pos.sum())
        f = rng.poisson(np.outer(ab, sig) + 20).astype(float)
        return np.hstack([rng.normal(5e4, 5e3, (n, 2)), f])
    SAMPLES["dep_A.fcs"], SAMPLES["dep_B.fcs"], SAMPLES["dep_C.fcs"] = ctl_from(M_TRUE[0], 21), ctl_from(M_TRUE[1], 22), ctl_from(mix, 23)
    E.STATE["meta"] = [{"name": n, "file": n, "pnn": CH, "pns": [""] * len(CH), "events": len(v)} for n, v in SAMPLES.items()]
    r = call("build_spectral_model", controls=[{"sample": s} for s in ("dep_A.fcs", "dep_B.fcs", "dep_C.fcs")])
    json.dumps(r, allow_nan=False)                  # raises on Infinity/NaN
    # measured signatures carry noise, so C is not *exactly* dependent: its index is huge but finite
    coll = r["qc"]["collinearity"]
    assert coll[2] is not None and coll[2] > 20 and coll[2] > 5 * min(c for c in coll if c is not None) / 5, coll

    # an exactly singular result must serialise as null + be listed, never as the invalid token Infinity
    import spectral as spmod
    real = spmod.signature_qc
    def fake(M):
        q = real(M); q["collinearity"] = q["collinearity"].copy(); q["collinearity"][1] = np.inf; q["condition"] = np.inf
        return q
    spmod.signature_qc = fake
    try:
        r2 = call("build_spectral_model", controls=[{"sample": s} for s in ("dep_A.fcs", "dep_B.fcs", "dep_C.fcs")])
    finally:
        spmod.signature_qc = real
    text = json.dumps(r2, allow_nan=False)
    assert "Infinity" not in text and r2["qc"]["collinearity"][1] is None and r2["qc"]["condition"] is None
    assert r2["qc"]["dependent"] == [r2["names"][1]]

if __name__ == "__main__":
    for k, v in list(globals().items()):
        if k.startswith("test_"): v(); print("ok", k)
