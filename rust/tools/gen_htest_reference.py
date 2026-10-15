"""Writes rust/crates/sf-core/tests/reference_htest.txt: the numbers the engine's SciPy calls give.
Run from the repo root with the bundled interpreter:  engine/py-env/Scripts/python.exe -I rust/tools/gen_htest_reference.py
"""
import sys, io, importlib.util
import numpy as np
from scipy import stats
sys.argv = ["x"]
spec = importlib.util.spec_from_file_location("eng", "engine/streamflow_engine.py")
eng = importlib.util.module_from_spec(spec)
real = sys.stdout
sys.stdout = io.StringIO()
spec.loader.exec_module(eng)
sys.stdout = real

rng = np.random.default_rng(20260101)
out = []
def fmt(a): return " ".join(repr(float(v)) for v in a)

def mwu(x, y, tag):
    r = stats.mannwhitneyu(x, y, alternative="two-sided")
    out.append(f"mwu {tag} {float(r.pvalue)!r} {float(r.statistic)!r} ; {fmt(x)} ; {fmt(y)}")

for n1, n2 in [(3, 3), (5, 8), (8, 30), (2, 50), (1, 7), (9, 9), (15, 20), (40, 45), (8, 8)]:
    mwu(rng.normal(0, 1, n1), rng.normal(0.7, 1, n2), f"cont{n1}x{n2}")
for n1, n2 in [(6, 7), (12, 12), (5, 30), (20, 25)]:
    mwu(np.round(rng.normal(5, 2, n1)), np.round(rng.normal(6, 2, n2)), f"ties{n1}x{n2}")

def wil(x, y, tag):
    r = stats.wilcoxon(x, y)
    out.append(f"wil {tag} {float(r.pvalue)!r} {float(r.statistic)!r} ; {fmt(x - y)}")

for n in [5, 8, 12, 13, 14, 20, 30, 50, 51, 80]:
    x = rng.normal(0, 1, n); y = x + rng.normal(0.4, 1, n)
    wil(x, y, f"cont{n}")
for n in [6, 10, 13, 14, 25, 40]:           # ties and zeros: permutation (n<=13) / asymptotic
    x = np.round(rng.normal(10, 3, n)); y = x + np.round(rng.normal(1, 2, n))
    wil(x, y, f"tieszero{n}")
x = np.arange(1.0, 9.0); wil(x + 3.0, x, "alldiff8")

def kw(groups, tag):
    h = stats.kruskal(*groups)
    names = [f"G{i+1}" for i in range(len(groups))]
    d = {n: g.tolist() for n, g in zip(names, groups)}
    dunn = eng._dunn_posthoc(d)
    out.append(f"kw {tag} {float(h.pvalue)!r} {float(h.statistic)!r} ; " + " ; ".join(fmt(g) for g in groups))
    out.append(f"dunn {tag} " + " ".join(repr(float(r["p"])) for r in dunn) + " ; " + " ; ".join(fmt(g) for g in groups))

kw([rng.normal(0, 1, 5), rng.normal(1, 1, 6), rng.normal(2.5, 1, 7)], "cont3")
kw([rng.normal(m, 1.5, n) for m, n in [(0, 8), (0.5, 9), (2, 10), (4, 7)]], "cont4")
kw([np.round(rng.normal(m, 2, 9)) for m in (3, 4, 7)], "ties3")
kw([rng.normal(0, 1, 10), rng.normal(0, 1, 12), rng.normal(0, 1, 9)], "null3")

# distribution anchors
for z in [0.0, 0.5, 1.96, 3.0, 5.0, 8.0, 12.0]:
    out.append(f"norm {z!r} {float(stats.norm.sf(z))!r}")
for x, k in [(0.5, 1), (3.84, 1), (10.0, 4), (1.0, 2), (30.0, 3), (100.0, 5), (0.1, 6)]:
    out.append(f"chi2 {x!r} {k} {float(stats.chi2.sf(x, k))!r}")
open("rust/crates/sf-core/tests/reference_htest.txt", "w").write("\n".join(out) + "\n")
print(len(out), "cases")
