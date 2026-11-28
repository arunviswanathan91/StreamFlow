"""Regenerate THIRD-PARTY-NOTICES.txt: the crates linked into the Windows release binary, with
their licence texts (identical texts are printed once).

    cd rust && cargo fetch --locked && python tools/gen_notices.py > ../THIRD-PARTY-NOTICES.txt
"""
import glob, hashlib, os, re, subprocess, sys

tree = subprocess.run(
    ["cargo", "tree", "--locked", "-p", "sf-app", "--target", "x86_64-pc-windows-msvc", "-e", "normal",
     "--prefix", "none", "--format", "{p}|{l}"],
    capture_output=True, text=True, check=True).stdout
crates = {}
for line in tree.splitlines():
    m = re.match(r"(\S+) v(\S+)[^|]*\|(.*)", line.replace(" (*)", ""))
    if m and not m.group(1).startswith("sf-"):
        crates[(m.group(1), m.group(2))] = m.group(3).strip()

home = os.environ.get("CARGO_HOME", os.path.expanduser("~/.cargo"))
texts, order, missing = {}, [], []
for (name, ver), lic in sorted(crates.items()):
    dirs = glob.glob(os.path.join(home, "registry", "src", "*", f"{name}-{ver}"))
    files = []
    for d in dirs[:1]:
        for f in sorted(os.listdir(d)):
            if re.match(r"(?i)(licen[sc]e|copying|notice|unlicense|copyright)", f) and os.path.isfile(os.path.join(d, f)):
                files.append(os.path.join(d, f))
    if not files:
        missing.append(f"{name} {ver}")
    for f in files:
        body = open(f, encoding="utf-8", errors="replace").read().replace("\r\n", "\n").strip()
        key = hashlib.sha256(re.sub(r"\s+", " ", body).encode()).hexdigest()
        if key not in texts:
            texts[key] = (body, [])
            order.append(key)
        texts[key][1].append(f"{name} {ver} ({os.path.basename(f)})")

out = ["StreamFLOW - third-party notices", "",
       "The StreamFLOW executable is built from the Rust crates listed below. Each remains under its own",
       "licence; the licence texts shipped with those crates follow the list.", "",
       f"{'crate':<34}{'version':<14}licence", "-" * 78]
out += [f"{n:<34}{v:<14}{l}" for (n, v), l in sorted(crates.items())]
if missing:
    out += ["", "Crates that ship no licence file (the licence named above applies): " + ", ".join(missing)]
for key in order:
    body, users = texts[key]
    out += ["", "=" * 78, "Applies to: " + ", ".join(users), "=" * 78, "", body]
sys.stdout.write("\n".join(out) + "\n")
