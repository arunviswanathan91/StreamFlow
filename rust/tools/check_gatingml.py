r"""Loads the GatingML written by the Rust exporter into FlowKit and compares its event counts with the
counts Rust computed for the same gates (files written by the `flowkit` test in sf-gating).

    cargo test -p sf-gating --test flowkit      (writes <target>/tmp/export.xml and export_counts.txt)
    engine\py-env\Scripts\python.exe rust\tools\check_gatingml.py <that tmp dir>
"""
import os
import sys
import flowkit as fk

d = sys.argv[1]
FIXTURE = os.path.join(os.path.dirname(__file__), "..", "..", "test-assests", "fcs", "multi stain", "AF700 alone.fcs")
smp = fk.Sample(FIXTURE, ignore_offset_error=True)
stem = sys.argv[2] if len(sys.argv) > 2 else "export"
sess = fk.Session(gating_strategy=os.path.join(d, stem + ".xml"))
sess.add_samples([smp])
sess.analyze_samples()
want = dict((l.rsplit(" ", 1)[0], int(l.rsplit(" ", 1)[1])) for l in open(os.path.join(d, stem + "_counts.txt")).read().splitlines() if l)
res = sess.get_gating_results(smp.id)
bad = 0
for name, n in want.items():
    got = int(res.get_gate_membership(name).sum())
    flag = "ok" if got == n else "MISMATCH"
    bad += got != n
    print("%-10s rust %6d  flowkit %6d  %s" % (name, n, got, flag))
print("gates checked:", len(want), "mismatches:", bad)
sys.exit(1 if bad else 0)
