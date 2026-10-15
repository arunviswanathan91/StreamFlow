"""Regenerates rust/crates/sf-core/tests/reference.txt from FlowKit/flowutils (the libraries the
Python engine uses), at full float64 precision. Run with the bundled interpreter:

    engine\\py-env\\Scripts\\python.exe rust\\tools\\gen_reference.py

The Rust tests pin themselves to this file, so any drift from the engine's maths is caught.
Format: blocks `# name params...` followed by `x y` lines (or matrix rows for compensation).
"""
import os
import numpy as np
import flowkit as fk

OUT = os.path.join(os.path.dirname(__file__), "..", "crates", "sf-core", "tests", "reference.txt")
lines = []


def block(name, rows):
    lines.append("# " + name)
    for r in rows:
        lines.append(" ".join(repr(float(v)) for v in r))


xs = np.array([-100000, -20000, -5000, -1000, -300, -50, -1, 0, 1, 10, 50, 100, 300, 1000, 3000,
               10000, 30000, 100000, 200000, 262144, 300000], dtype=float)

for (t, w, m, a) in [(262144, 0.5, 4.5, 0.0), (262144, 1.0, 4.5, 0.0), (262144, 0.5, 4.5, 0.5),
                     (1000000, 0.7, 5.0, 0.0)]:
    xf = fk.transforms.LogicleTransform(param_t=t, param_w=w, param_m=m, param_a=a)
    y = np.asarray(xf.apply(xs.reshape(-1, 1)), dtype=float).ravel()
    block("logicle %s %s %s %s" % (t, w, m, a), zip(xs, y))

pos = xs[xs > 0]
xf = fk.transforms.LogTransform(param_t=262144, param_m=4.5)
block("log 262144 4.5", zip(pos, np.asarray(xf.apply(pos.reshape(-1, 1)), dtype=float).ravel()))

for cof in (5.0, 150.0, 6000.0):
    block("arcsinh %s" % cof, zip(xs, np.arcsinh(xs / cof)))

# Compensation on REAL events: FlowKit's own Matrix.apply is the expected value (it must equal
# raw @ inv(S) on the detector columns and leave every other column untouched).
S = np.array([[1.00, 0.12, 0.01, 0.00],
              [0.03, 1.00, 0.20, 0.02],
              [0.00, 0.05, 1.00, 0.30],
              [0.01, 0.00, 0.08, 1.00]])
detectors = ["FITC-A", "PE-A", "APC-A", "BV421-A"]
FIXTURE = os.path.join(os.path.dirname(__file__), "..", "..", "test-assests", "fcs", "multi stain", "AF700 alone.fcs")
smp = fk.Sample(FIXTURE, ignore_offset_error=True)
m = fk.Matrix(S, detectors=detectors)
full_raw = np.asarray(smp.get_events(source="raw"), dtype=float)
full_comp = np.asarray(m.apply(smp), dtype=float)
idx = [list(smp.pnn_labels).index(d) for d in detectors]
assert np.allclose(full_raw[:, idx] @ np.linalg.inv(S), full_comp[:, idx], atol=1e-8), "convention differs"
assert np.array_equal(np.delete(full_comp, idx, 1), np.delete(full_raw, idx, 1)), "other columns changed"
raw = full_raw[:6, idx]
comp = full_comp[:6, idx]
block("compensation spillover 4x4", S)
block("compensation raw 6x4", raw)
block("compensation expected 6x4", comp)

# Non-negative compensation (engine mode "nnls"): per event solve S^T x = y, x >= 0 (scipy.optimize.nnls).
from scipy.optimize import nnls
y_nn = np.vstack([raw[:4], [[-300.0, 400.0, -50.0, 10.0], [0.0, 0.0, 0.0, 0.0]]])
x_nn = np.array([nnls(S.T, row)[0] for row in y_nn])
block("nnls raw 6x4", y_nn)
block("nnls expected 6x4", x_nn)

# ---- FlowJo transforms as FlowKit implements them for .wsp import ------------------------------------------
xb = np.array([-300000, -100000, -20000, -5000, -1000, -300, -50, -1, 0, 1, 10, 50, 100, 300, 1000, 3000,
               10000, 30000, 100000, 200000, 262144, 262144.000029, 300000], dtype=float)
for (neg_, wid_, pos_, mx_) in [(0.0, -10.0, 4.41854, 262144.000029), (0.0, -100.0, 4.41854, 262144.000029),
                                (1.0, -30.0, 4.41854, 262144.000029), (0.5, -1000.0, 4.0, 1000000.0),
                                (0.0, -1.0, 4.41854, 262144.000029)]:
    bx = fk.transforms.WSPBiexTransform(negative=neg_, width=wid_, positive=pos_, max_value=mx_)
    block("biex %r %r %r %r" % (neg_, wid_, pos_, mx_), zip(xb, np.asarray(bx.apply(xb), dtype=float)))
    yb = np.array([0, 1, 100, 500, 1000, 1500, 2000, 2048, 2500, 3000, 3500, 4000, 4095, 4096, 4097.5, -5.0])
    block("biexinv %r %r %r %r" % (neg_, wid_, pos_, mx_), zip(yb, np.asarray(bx.inverse(yb), dtype=float)))
for (off_, dec_) in [(1.0, 4.0), (0.5, 3.5)]:
    lx = fk.transforms.WSPLogTransform(offset=off_, decades=dec_)
    block("wsplog %r %r" % (off_, dec_), zip(xb, np.asarray(lx.apply(xb), dtype=float)))

# ---- spectral unmixing: the Python module (engine/spectral.py) is the reference ---------------------------
import sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "engine"))
import spectral as sp

D = 12
xg = np.arange(D)
M = np.array([np.exp(-0.5 * ((xg - pk) / 1.4) ** 2) for pk in (1, 4, 7, 10)])
M /= M.max(axis=1, keepdims=True)
rs = np.random.default_rng(11)
truth = rs.uniform(0, 3000, (8, 4))
ev8 = rs.poisson(truth @ M + 25).astype(float)
block("spectral M 4x12", M)
block("spectral events 8x12", ev8)
w = sp.poisson_weights(ev8)
block("spectral weights 12", [w])
block("spectral W ols 4x12", sp.unmixing_matrix(M))
block("spectral W wls 4x12", sp.unmixing_matrix(M, w))
ab_w, _ = sp.unmix(ev8, M, "wls")
ab_o, _ = sp.unmix(ev8, M, "ols")
block("spectral unmix wls 8x4", ab_w)
block("spectral unmix ols 8x4", ab_o)
block("spectral residuals 8", [sp.residuals(ev8, M, ab_w)])
block("spectral cond M 1", [[np.linalg.cond(M)]])
qc = sp.signature_qc(np.vstack([M, 0.97 * M[1] + 0.03 * M[2]]))      # exactly dependent -> inf
block("spectral qc cosine 5x5", qc["cosine"])
block("spectral qc collinearity 5", [qc["collinearity"]])
block("spectral qc condition 1", [[qc["condition"]]])
near = np.vstack([M, 0.97 * M[1] + 0.03 * np.random.default_rng(3).random(M.shape[1])])   # near-collinear, finite
qn = sp.signature_qc(near)
block("spectral qc near matrix 5x12", near)
block("spectral qc near collinearity 5", [qn["collinearity"]])
block("spectral qc near cosine 5x5", qn["cosine"])

sym = rs.normal(size=(6, 6)); sym = sym @ sym.T
block("eig sym 6x6", sym)
block("eig values desc 6", [np.sort(np.linalg.eigvalsh(sym))[::-1]])

# signature building on a noisy control (50% positive), with and without a negative baseline / saturation
rc = np.random.default_rng(5)
pos = rc.random(300) < 0.5
ab = np.where(pos, 6000 * rc.lognormal(0, 0.2, 300), 0.0)
ctl = rc.poisson(np.outer(ab, M[2]) + 30).astype(float)
neg = np.full(D, 30.0)
block("spectral control 300x12", ctl)
block("spectral negative 12", [neg])
for tag, kw in (("plain", {}), ("negative", {"negative": neg}), ("saturation", {"saturation": ctl.max(axis=0)})):
    r = sp.build_signature(ctl, **kw)
    assert r["ok"], r
    block("spectral signature %s head" % tag, [[r["peak"], r["n_pos"], r["n_neg"], r["threshold"], r["n_saturated"]]])
    block("spectral signature %s" % tag, [r["signature"]])
block("spectral peak detector 1", [[sp.peak_detector(ctl)]])
block("spectral saturation mask 300", [sp.saturation_mask(ctl, ctl.max(axis=0)).astype(float)])

# per-cell autofluorescence with a fixed 2-signature library
afl = np.abs(rs.normal(0, 1, (2, D))); afl /= afl.max(axis=1, keepdims=True)
evaf = rs.poisson(truth @ M + 300 * afl[rs.integers(0, 2, 8)] + 10).astype(float)
abf, idx, res = sp.unmix_with_af(evaf, M, afl, "wls")
block("spectral af library 2x12", afl)
block("spectral af events 8x12", evaf)
block("spectral af unmix wls 8x5", abf)
block("spectral af index 8", [idx.astype(float)])
block("spectral af residual2 8", [res])

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w", newline="\n") as f:
    f.write("\n".join(lines) + "\n")
print("wrote", os.path.normpath(OUT), len(lines), "lines")
