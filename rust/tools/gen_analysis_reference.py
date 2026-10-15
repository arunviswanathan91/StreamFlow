import sys, os, json, importlib.util, math, io
import numpy as np
sys.argv = ["x"]
spec = importlib.util.spec_from_file_location("eng", "engine/streamflow_engine.py")
eng = importlib.util.module_from_spec(spec)
real_stdout = sys.stdout
sys.stdout = io.StringIO()          # the engine prints protocol lines
spec.loader.exec_module(eng)
C = eng.COMMANDS
out = {}
def run(name, args):
    return C[name](1, args)

# ---- cell cycle on the real PI file
f = "test-assests/fcs/single stain/single-stain-pe.fcs"
run("load_fcs", {"files": [os.path.abspath(f)]})
for model in ("watson", "djf"):
    r = run("run_cell_cycle", {"sample": "single-stain-pe.fcs", "channel": "PI-A", "model": model})
    r.pop("curves")
    out["cc_" + model] = r

# ---- apoptosis on the real annexin / dual files
for fn, key in (("annexin alone.fcs", "ap_annexin"), ("dual-stain-pe-fitc.fcs", "ap_dual")):
    p = os.path.abspath("test-assests/fcs/dual stain/" + fn)
    run("load_fcs", {"files": [p]})
    try:
        r = run("run_apoptosis", {"sample": fn, "annexin_channel": "FL1-A", "pi_channel": "FL2-A"})
        r.pop("events", None); r.pop("png", None)
        out[key] = r
    except Exception as e:
        out[key] = {"error": str(e)}

# ---- proliferation on a closed-form dye dataset
def vdc(k):
    x, f, n = 0.0, 0.5, k + 1
    while n > 0:
        x += f * (n & 1); n >>= 1; f *= 0.5
    return x
def dye(m_per_unit=2000):
    vals = []
    weights = [1, 2, 4, 8]
    for g, w in enumerate(weights):
        m = m_per_unit * w
        for k in range(m):
            u1 = (k + 0.5) / m; u2 = vdc(k) * 0.999 + 0.0005
            z = math.sqrt(-2.0 * math.log(u1)) * math.cos(2.0 * math.pi * u2)
            vals.append(10 ** (4.2 - g * math.log10(2) + 0.05 * z))
    return vals
vals = np.array(dye())
class FakeSample:
    pnn_labels = ["FSC-A", "CFSE-A"]
    def get_events(self, source="raw"):
        return np.column_stack([np.ones_like(vals), vals])
eng._sample_by_name = lambda name: FakeSample()
eng.STATE["meta"] = [{"name": "dye"}]
r = run("run_proliferation", {"sample": "dye", "channel": "CFSE-A", "n_peaks": 8})
r.pop("curves")
out["prolif"] = r
# the same closed-form values for cell cycle too (values path)
sys.stdout = real_stdout
print(json.dumps(out, indent=1))
