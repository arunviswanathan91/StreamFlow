"""Regenerates rust/crates/sf-gating/tests/reference_gating.txt: gate definitions plus the event counts
FlowKit's own gating engine (the one the Python engine uses) produces on a real fixture. The Rust test
builds the SAME gates from these numbers and must reproduce every count.

    engine\\py-env\\Scripts\\python.exe rust\\tools\\gen_gating_reference.py

Line formats (channel names contain no spaces):
    rect    <name> <xchan> <ychan> x0 x1 y0 y1 <flowkit_count> <inclusive_count>
    interval<name> <chan> x0 x1 <flowkit_count> <inclusive_count>
    poly    <name> <xchan> <ychan> <n> x1 y1 ... xn yn <count>
    ellipse <name> <xchan> <ychan> cx cy rx ry angle <count>
    bool    <name> <and|or|not> <ref,!ref,...> <count>
    events  <n>
"""
import os
import numpy as np
import flowkit as fk

HERE = os.path.dirname(__file__)
FIXTURE = os.path.join(HERE, "..", "..", "test-assests", "fcs", "multi stain", "AF700 alone.fcs")
OUT = os.path.join(HERE, "..", "crates", "sf-gating", "tests", "reference_gating.txt")

smp = fk.Sample(FIXTURE, ignore_offset_error=True)
raw = np.asarray(smp.get_events(source="raw"), dtype=float)
pnn = list(smp.pnn_labels)
col = lambda c: raw[:, pnn.index(c)]
fmt = lambda v: repr(float(v))

gs = fk.GatingStrategy()
lines = ["events %d" % raw.shape[0]]
counts = {}
kinds = {}


def dim(c, lo=None, hi=None):
    # compensation_ref must be explicit: left unset, FlowKit applies the FCS file's own spillover to fluorescence
    # channels, so a gate on FITC-A would silently be tested on compensated values
    return fk.Dimension(c, compensation_ref="uncompensated", transformation_ref=None, range_min=lo, range_max=hi)


def register(gate, name, line_prefix):
    gs.add_gate(gate, ("root",))
    kinds[name] = line_prefix


pairs = [("FSC-A", "SSC-A"), ("FITC-A", "PE-A")]
specs = []  # (name, kind-line-builder)
for (xc, yc) in pairs:
    x, y = col(xc), col(yc)
    p = lambda q, a: float(np.percentile(a, q))
    tag = xc.split("-")[0]
    # rectangle on a mid-population box (boundaries are irrational-looking so no event sits on an edge)
    x0, x1, y0, y1 = p(20.123, x), p(80.456, x), p(15.789, y), p(70.321, y)
    g = fk.gates.RectangleGate("rect_" + tag, [dim(xc, x0, x1), dim(yc, y0, y1)])
    register(g, "rect_" + tag, ("rect", xc, yc, [x0, x1, y0, y1]))
    # 1-D interval
    i0, i1 = p(10.111, x), p(60.222, x)
    g = fk.gates.RectangleGate("int_" + tag, [dim(xc, i0, i1)])
    register(g, "int_" + tag, ("interval", xc, None, [i0, i1]))
    # concave polygon (an "L"): the bounding box minus its upper-right quarter
    qx0, qx1, qy0, qy1 = p(5.3, x), p(95.1, x), p(5.7, y), p(95.9, y)
    mx, my = p(50.17, x), p(50.29, y)
    verts = [(qx0, qy0), (qx1, qy0), (qx1, my), (mx, my), (mx, qy1), (qx0, qy1)]
    g = fk.gates.PolygonGate("poly_" + tag, [dim(xc), dim(yc)], [tuple(v) for v in verts])
    register(g, "poly_" + tag, ("poly", xc, yc, verts))
    # ellipses: axis-aligned and rotated 0.6 rad
    cx, cy = float(np.median(x)), float(np.median(y))
    rx, ry = (p(90.5, x) - p(9.5, x)) / 2, (p(90.5, y) - p(9.5, y)) / 2
    for ang, suffix in ((0.0, "e0"), (0.6, "e1")):
        c, s = np.cos(ang), np.sin(ang)
        R = np.array([[c, -s], [s, c]])
        cov = R @ np.diag([rx * rx, ry * ry]) @ R.T
        g = fk.gates.EllipsoidGate("%s_%s" % (suffix, tag), [dim(xc), dim(yc)], (cx, cy), cov, 1.0)
        register(g, "%s_%s" % (suffix, tag), ("ellipse", xc, yc, [cx, cy, rx, ry, ang]))

# Boolean gates over the scatter pair (all references live at root)
def ref(name, comp=False):
    return {"ref": name, "path": ("root",), "complement": comp}

bools = [
    ("and_ab", "and", [("rect_FSC", False), ("e1_FSC", False)]),
    ("or_ab", "or", [("rect_FSC", False), ("poly_FITC", False)]),
    ("not_a", "not", [("poly_FSC", False)]),
    ("and_notb", "and", [("rect_FITC", False), ("e0_FITC", True)]),
]
for name, op, refs in bools:
    g = fk.gates.BooleanGate(name, op, [ref(n, c) for n, c in refs])
    gs.add_gate(g, ("root",))
    kinds[name] = ("bool", op, refs)

res = gs.gate_sample(smp)
for name, k in kinds.items():
    n = int(res.get_gate_membership(name).sum())
    if k[0] == "rect":
        x0, x1, y0, y1 = k[3]
        inc = int(((col(k[1]) >= x0) & (col(k[1]) <= x1) & (col(k[2]) >= y0) & (col(k[2]) <= y1)).sum())
        lines.append("rect %s %s %s %s %d %d" % (name, k[1], k[2], " ".join(fmt(v) for v in k[3]), n, inc))
    elif k[0] == "interval":
        a0, a1 = k[3]
        inc = int(((col(k[1]) >= a0) & (col(k[1]) <= a1)).sum())
        lines.append("interval %s %s %s %d %d" % (name, k[1], " ".join(fmt(v) for v in k[3]), n, inc))
    elif k[0] == "poly":
        flat = " ".join("%s %s" % (fmt(a), fmt(b)) for a, b in k[3])
        lines.append("poly %s %s %s %d %s %d" % (name, k[1], k[2], len(k[3]), flat, n))
    elif k[0] == "ellipse":
        lines.append("ellipse %s %s %s %s %d" % (name, k[1], k[2], " ".join(fmt(v) for v in k[3]), n))
    else:
        refs = ",".join(("!" if c else "") + r for r, c in k[2])
        lines.append("bool %s %s %s %d" % (name, k[1], refs, n))

# Sanity: FlowKit's rectangle is GatingML's half-open [min, max) EXACTLY (not inclusive: on this quantised
# fixture the two differ by thousands of events).
for name, k in kinds.items():
    if k[0] == "rect":
        x0, x1, y0, y1 = k[3]
        half = int(((col(k[1]) >= x0) & (col(k[1]) < x1) & (col(k[2]) >= y0) & (col(k[2]) < y1)).sum())
        assert half == int(res.get_gate_membership(name).sum()), (name, half)
    if k[0] == "interval":
        half = int(((col(k[1]) >= k[3][0]) & (col(k[1]) < k[3][1])).sum())
        assert half == int(res.get_gate_membership(name).sum()), (name, half)

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w", newline="\n") as f:
    f.write("\n".join(lines) + "\n")
print("wrote", os.path.normpath(OUT))
for l in lines:
    print(l[:110])
