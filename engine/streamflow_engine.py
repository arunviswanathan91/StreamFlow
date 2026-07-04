"""StreamFLOW — headless Python compute engine.

Drop-in replacement for streamflow_engine.R: same newline-delimited JSON protocol
over stdin/stdout, spoken to the JavaFX app via the same bridge. Uses FlowKit for
FCS / compensation / transforms / gating and matplotlib for rendering.

  Java  -> py (stdin) : {"id":N,"cmd":"...","args":{...}}
  py -> Java (stdout) : {"type":"progress|result|error|pong|ready", ...}

stdout is reserved exclusively for JSON: stray prints from libraries are sent to
stderr (sys.stdout is redirected to stderr; a private handle keeps the real
stdout for JSON only). EOF on stdin => parent gone => exit (orphan safety).
"""
import sys, os, io, json, argparse, traceback, time, glob, re, threading

# --- stdout hijack: do this FIRST ------------------------------------------
_JSON_OUT = sys.stdout                      # private, clean channel for JSON
sys.stdout = sys.stderr                     # stray prints -> stderr

def send(obj):
    _JSON_OUT.write(json.dumps(obj, default=str))
    _JSON_OUT.write("\n")
    _JSON_OUT.flush()

def send_ready():     send({"type": "ready", "pid": os.getpid(), "py": sys.version.split()[0]})
def send_pong(i):     send({"type": "pong", "id": i})
def send_progress(i, frac, msg=None): send({"type": "progress", "id": i, "frac": frac, "msg": msg})
def send_error(i, message, trace=None):
    send({"type": "error", "id": i, "message": str(message), "trace": trace or []})
def send_result(i, payload=None):
    d = {"type": "result", "id": i}
    if payload:
        d.update(payload)
    send(d)

# --- args / control dir -----------------------------------------------------
_ap = argparse.ArgumentParser()
_ap.add_argument("--control-dir", default=None)
_OPTS, _ = _ap.parse_known_args()
CONTROL_DIR = _OPTS.control_dir or os.path.join(os.environ.get("TEMP", "/tmp"), "sfctl")
os.makedirs(CONTROL_DIR, exist_ok=True)

def is_cancelled(i):
    return os.path.exists(os.path.join(CONTROL_DIR, "cancel_%s.flag" % i))

class Cancelled(Exception):
    pass

# --- global state (the engine's single mutable session) ---------------------
STATE = {
    "meta": [],          # per-file header metadata (fast): {name,file,pnn,pns,events}
    "files": [],         # parallel list of file paths
    "cache": {},         # lazily-built flowkit.Sample objects, keyed by file path
    "experiment": "Untitled Experiment",
    "gating": None,      # flowkit GatingStrategy (later phases)
    "comp_matrix": None, # {channels, matrix, fk_matrix} after extract_spillover/compute_spillover
    "comp_applied": False,
    "transforms": {},    # {channel: {method, cofactor}} after apply_transformation
}

def _basenames():
    return [os.path.basename(f) for f in STATE["files"]]

def _read_header(f):
    """Read ONLY the FCS TEXT segment (no event data) for fast listing."""
    import flowio
    fd = flowio.FlowData(f, only_text=True, ignore_offset_error=True)
    t = fd.text
    par = int(t.get("par", 0))
    pnn, pns = [], []
    for j in range(1, par + 1):
        pnn.append(t.get("p%dn" % j) or ("P%d" % j))
        pns.append(t.get("p%ds" % j) or "")
    return {"name": os.path.basename(f), "file": f, "pnn": pnn, "pns": pns,
            "events": int(t.get("tot", 0))}

# --- command registry -------------------------------------------------------
COMMANDS = {}
def command(name):
    def deco(fn):
        COMMANDS[name] = fn
        return fn
    return deco

# ---- protocol probes (parity with the R engine) ----------------------------
@command("ping")
def _ping(i, a): send_pong(i); return None

@command("echo")
def _echo(i, a): return {"echo": a}

@command("version")
def _version(i, a): return {"version": "Python " + sys.version.split()[0], "engine": "python"}

@command("sleep")
def _sleep(i, a):
    steps = int((a or {}).get("steps", 10)); step_ms = float((a or {}).get("step_ms", 200))
    for k in range(steps):
        if is_cancelled(i): raise Cancelled()
        time.sleep(step_ms / 1000.0)
        send_progress(i, (k + 1) / steps, "step %d/%d" % (k + 1, steps))
    return {"slept_ms": steps * step_ms}

@command("boom")
def _boom(i, a): raise RuntimeError("intentional failure for protocol test")

@command("noisy")
def _noisy(i, a):
    print("this print() must NOT reach the JSON stream")       # -> stderr
    sys.stderr.write("this stderr line is fine\n")
    return {"noisy": True}

@command("capabilities")
def _capabilities(i, a):
    import importlib.util
    pkgs = ["flowkit", "flowio", "flowutils", "numpy", "scipy", "pandas",
            "matplotlib", "datashader", "umap", "openTSNE", "phenograph",
            "flowsom", "cytonormpy", "sklearn"]
    have = {p: importlib.util.find_spec(p) is not None for p in pkgs}
    return {"packages": have}

# ---- setup: FCS import (FlowKit) -------------------------------------------
def _scatter(label):
    return bool(re.search(r"FSC|SSC|Time|Width|Event", label, re.I))

def _summary(skipped=0):
    metas = STATE["meta"]
    if not metas:
        return {"n_samples": 0, "samples": [], "channels": []}
    m0 = metas[0]
    pnn, pns = m0["pnn"], m0["pns"]
    channels = [{"channel": pnn[j], "marker": (pns[j] or ""), "scatter": _scatter(pnn[j])}
                for j in range(len(pnn))]
    out = [{"name": m["name"], "events": int(m["events"])} for m in metas]
    return {"experiment": STATE["experiment"], "n_samples": len(metas),
            "skipped": skipped, "samples": out, "channels": channels}

@command("load_fcs")
def _load_fcs(i, a):
    a = a or {}
    files = a.get("files") or []
    folder = a.get("folder")
    if not files:
        if not folder or not os.path.isdir(folder):
            raise ValueError("No folder or files provided.")
        pat = os.path.join(folder, "**", "*.fcs") if a.get("recursive") else os.path.join(folder, "*.fcs")
        files = glob.glob(pat, recursive=bool(a.get("recursive")))
    files = [f for f in files if os.path.isfile(f)]
    if not files:
        raise ValueError("No .fcs files found.")

    send_progress(i, 0.1, "Scanning %d FCS file(s)…" % len(files))
    metas, sigs = [], set()
    for k, f in enumerate(files):
        if is_cancelled(i): raise Cancelled()
        metas.append(_read_header(f))                       # header only — milliseconds
        sigs.add(tuple(metas[-1]["pnn"]))
        send_progress(i, 0.1 + 0.85 * (k + 1) / len(files), "Read %s" % metas[-1]["name"])
    if len(sigs) > 1:
        raise ValueError(
            "These FCS files have %d different channel layouts and cannot be combined "
            "into one experiment — load files that share the same panel." % len(sigs))

    STATE["meta"] = metas
    STATE["files"] = files
    STATE["cache"] = {}
    STATE["gating"] = None
    return _summary()

@command("get_state")
def _get_state(i, a): return _summary()

@command("set_experiment_name")
def _set_exp(i, a):
    STATE["experiment"] = str((a or {}).get("name", "Untitled Experiment"))
    return {"experiment": STATE["experiment"]}

@command("list_channels")
def _list_channels(i, a):
    # A query, not an operation — return empty before any data is loaded (no error spam at startup).
    if not STATE["meta"]:
        return {"channels": [], "samples": []}
    return {"channels": list(STATE["meta"][0]["pnn"]), "samples": _basenames()}

@command("reset")
def _reset(i, a):
    STATE["meta"] = []; STATE["files"] = []; STATE["cache"] = {}
    STATE["gating"] = None; STATE["comp_matrix"] = None
    STATE["comp_applied"] = False; STATE["transforms"] = {}
    return {"ok": True}

@command("get_events")
def _get_events(i, a):
    """Ship a sample's events to Java as a little-endian float32 binary blob
    (row-major, rows x cols) for native rendering + gating. Default: all channels,
    up to n events (subsampled if larger)."""
    import numpy as np
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    s = _sample_by_name(a.get("sample"))
    pnn = list(s.pnn_labels)
    chans = a.get("channels") or pnn
    chans = [c for c in chans if c in pnn]
    if not chans:
        raise ValueError("No valid channels requested.")
    src = a.get("source", "raw")
    ev = s.get_events(source=src)
    cols = [pnn.index(c) for c in chans]
    sub = np.asarray(ev[:, cols], dtype="<f4")
    n = sub.shape[0]
    cap = int(a.get("n", 500000))
    if n > cap:
        sub = sub[np.random.choice(n, cap, replace=False)]
    path = os.path.join(CONTROL_DIR, "sfev_%s_%d.bin" % (i, int(time.time() * 1000)))
    sub.tofile(path)  # native little-endian float32, row-major
    ranges = {c: [float(np.min(sub[:, j])), float(np.max(sub[:, j]))] for j, c in enumerate(chans)}
    return {"file": path, "channels": chans, "rows": int(sub.shape[0]),
            "cols": len(chans), "total": int(n), "ranges": ranges}

# ---- visualization: render a plot to PNG (matplotlib) ----------------------
def _sample_by_name(name):
    """Lazily build (and cache) the full flowkit.Sample for a file on first use.
    If a compensation matrix has been applied this session, auto-applies it so
    get_events(source='comp') works on any newly-loaded sample."""
    metas = STATE["meta"]
    if not metas:
        raise ValueError("Load data first.")
    idx = 0
    if name:
        names = [m["name"] for m in metas]
        if name in names:
            idx = names.index(name)
    f = metas[idx]["file"]
    s = STATE["cache"].get(f)
    if s is None:
        import flowkit as fk
        s = fk.Sample(f, ignore_offset_error=True)
        if STATE.get("comp_applied") and STATE.get("comp_matrix", {}) and \
                STATE["comp_matrix"].get("fk_matrix") is not None:
            try:
                s.apply_compensation(STATE["comp_matrix"]["fk_matrix"])
            except Exception:
                pass
        STATE["cache"][f] = s
    return s

@command("render_plot")
def _render_plot(i, a):
    import numpy as np
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.colors import LinearSegmentedColormap

    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    s = _sample_by_name(a.get("sample"))
    pnn = list(s.pnn_labels)
    xch = a.get("x")
    ych = a.get("y")
    if not xch or xch not in pnn:
        raise ValueError("X channel not found: %s" % xch)
    typ = a.get("type") or ("histogram" if not ych else "pseudocolor")
    if typ != "histogram" and (not ych or ych not in pnn):
        typ = "histogram"

    scale = float(a.get("scale", 1))
    w = int(a.get("width", 700)); h = int(a.get("height", 520))
    dpi = 100.0
    ev = s.get_events(source="raw")
    n = ev.shape[0]
    idx = np.random.choice(n, 20000, replace=False) if n > 20000 else np.arange(n)
    x = ev[idx, pnn.index(xch)]

    # FlowJo-style ramp: low density blue -> high red, empty bins white.
    cmap = LinearSegmentedColormap.from_list(
        "sf", ["#0000CC", "#00CCFF", "#00CC44", "#FFFF00", "#FF9900", "#FF0000"])

    fig = plt.figure(figsize=(w / dpi, h / dpi), dpi=dpi * scale, facecolor="white")
    ax = fig.add_axes([0.13, 0.13, 0.84, 0.84])
    ax.set_facecolor("white")
    ax.set_xlabel(xch)
    if typ == "histogram":
        ax.hist(x, bins=256, color="#3182BD", alpha=0.85)
        ax.set_ylabel("Count")
    else:
        y = ev[idx, pnn.index(ych)]
        ax.set_ylabel(ych)
        if typ == "dot":
            ax.scatter(x, y, s=2, c="#08306B", alpha=0.5, edgecolors="none")
        elif typ == "contour":
            ax.hist2d(x, y, bins=200, cmap=cmap, cmin=1)
            try:
                from scipy.stats import gaussian_kde
                sub = idx if len(idx) <= 4000 else np.random.choice(idx, 4000, replace=False)
                xs, ys = ev[sub, pnn.index(xch)], ev[sub, pnn.index(ych)]
                k = gaussian_kde(np.vstack([xs, ys]))
                gx, gy = np.mgrid[xs.min():xs.max():60j, ys.min():ys.max():60j]
                ax.contour(gx, gy, k(np.vstack([gx.ravel(), gy.ravel()])).reshape(gx.shape),
                           colors="#08519C", linewidths=0.6)
            except Exception:
                pass
        else:  # pseudocolor / density / zebra
            ax.hist2d(x, y, bins=220, cmap=cmap, cmin=1)

    # data ranges + plot-region pixel box (image coords: origin top-left).
    fig.canvas.draw()
    xr = ax.get_xlim(); yr = ax.get_ylim()
    ph = fig.get_size_inches()[1] * fig.dpi
    (l, b) = ax.transData.transform((xr[0], yr[0]))
    (r, t) = ax.transData.transform((xr[1], yr[1]))
    plotbox = [float(l), float(ph - t), float(r), float(ph - b)]  # [left, top, right, bottom]

    png_path = os.path.join(CONTROL_DIR, "sfplot_%s_%d.png" % (i, int(time.time() * 1000)))
    fig.savefig(png_path, dpi=fig.dpi, facecolor="white")
    plt.close(fig)

    sample_name = a.get("sample") or os.path.basename(STATE["files"][0])
    return {"png": png_path, "sample": sample_name,
            "x": xch, "y": ych, "type": typ,
            "width": int(w * scale), "height": int(h * scale),
            "xrange": [float(xr[0]), float(xr[1])],
            "yrange": [float(yr[0]), float(yr[1])],
            "plotbox_px": plotbox}

# ---- statistics: per-sample count + MFI (FlowKit) -------------------------
@command("compute_stats")
def _compute_stats(i, a):
    """Return per-sample statistics at the root ("All Events") level.
    Gate-level stats (child populations) are computed natively in Java from the
    WorkspaceModel gating tree + EventData; this command covers the Statistics module
    which needs a cross-sample table with median MFI per fluorescence channel."""
    import numpy as np
    if not STATE["meta"]:
        raise ValueError("Load data first.")

    fluor_chans = [m for m in STATE["meta"][0]["pnn"]
                   if not bool(__import__("re").search(r"FSC|SSC|Time|Width|Event", m, __import__("re").I))]
    rows = []
    total_files = len(STATE["files"])
    for k, meta in enumerate(STATE["meta"]):
        if is_cancelled(i): raise Cancelled()
        send_progress(i, (k + 0.5) / total_files, "Stats for %s" % meta["name"])
        s = _sample_by_name(meta["name"])
        ev = np.asarray(s.get_events(source="raw"))
        pnn = list(s.pnn_labels)
        n_events = int(ev.shape[0])
        mfi = {}
        for ch in fluor_chans:
            if ch in pnn:
                col = pnn.index(ch)
                vals = ev[:, col]
                mfi[ch] = float(np.median(vals))
        rows.append({
            "sample": meta["name"],
            "population": "All Events",
            "count": n_events,
            "percent_total": 100.0,
            "mfi": mfi,
        })
    return {"channels": fluor_chans, "rows": rows}


# ---- dim reduction: UMAP / t-SNE (umap-learn / openTSNE) ------------------
@command("run_dimredux")
def _run_dimredux(i, a):
    import numpy as np
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    method = str(a.get("method", "umap")).lower()
    n_events = int(a.get("n_events", 5000))
    s = _sample_by_name(a.get("sample"))
    pnn = list(s.pnn_labels)
    fluor = [c for c in pnn if not bool(__import__("re").search(r"FSC|SSC|Time|Width|Event", c, __import__("re").I))]
    if not fluor:
        raise ValueError("No fluorescence channels found for dim-reduction.")

    ev = np.asarray(s.get_events(source="raw"))
    n_total = ev.shape[0]
    idx = np.random.choice(n_total, min(n_events, n_total), replace=False)
    cols = [pnn.index(c) for c in fluor]
    X = ev[np.ix_(idx, cols)].astype("float32")
    # arcsinh cofactor 150 as a sensible default transform for flow data
    X = np.arcsinh(X / 150.0)

    send_progress(i, 0.2, "Running %s on %d events…" % (method.upper(), len(idx)))

    if method == "tsne":
        try:
            from openTSNE import TSNE
            emb = TSNE(perplexity=30, n_jobs=4, random_state=42).fit(X)
            coords = np.array(emb)
        except ImportError:
            from sklearn.manifold import TSNE as skTSNE
            coords = skTSNE(n_components=2, perplexity=30, random_state=42).fit_transform(X)
    else:
        try:
            import umap
            coords = umap.UMAP(n_components=2, random_state=42).fit_transform(X)
        except ImportError:
            raise ValueError("umap-learn is not installed. Run: pip install umap-learn")

    send_progress(i, 0.85, "Done.")
    coords_out = [[round(float(coords[r, 0]), 4), round(float(coords[r, 1]), 4)]
                  for r in range(len(coords))]
    return {"coords": coords_out, "method": method, "n": int(len(idx)), "n_features": len(fluor)}


# ---- clustering: FlowSOM / PhenoGraph -------------------------------------
@command("run_clustering")
def _run_clustering(i, a):
    import numpy as np
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    method = str(a.get("method", "flowsom")).lower()
    k = int(a.get("k", 10))
    n_events = int(a.get("n_events", 20000))
    s = _sample_by_name(a.get("sample"))
    pnn = list(s.pnn_labels)
    fluor = [c for c in pnn if not bool(__import__("re").search(r"FSC|SSC|Time|Width|Event", c, __import__("re").I))]
    if not fluor:
        raise ValueError("No fluorescence channels found for clustering.")

    ev = np.asarray(s.get_events(source="raw"))
    n_total = ev.shape[0]
    idx = np.random.choice(n_total, min(n_events, n_total), replace=False)
    cols = [pnn.index(c) for c in fluor]
    X = ev[np.ix_(idx, cols)].astype("float32")
    X = np.arcsinh(X / 150.0)

    send_progress(i, 0.2, "Clustering %d events with %s…" % (len(idx), method))

    actual = method   # what actually ran (so the title/legend never lies)
    if method == "phenograph":
        import phenograph
        labels, _graph, _Q = phenograph.cluster(X)
        labels = np.asarray(labels)
        n_clusters = int(labels.max() + 1)
    elif method == "flowsom":
        try:
            import anndata as ad
            import flowsom as fs
            fsom = fs.FlowSOM(ad.AnnData(X), n_clusters=k, cols_to_use=list(range(X.shape[1])), seed=42)
            raw = list(np.asarray(fsom.metacluster_labels).tolist())
            remap = {v: j for j, v in enumerate(sorted(set(raw)))}
            labels = np.array([remap[v] for v in raw], dtype=int)
            n_clusters = len(remap)
        except Exception as exc:
            send_progress(i, 0.5, "FlowSOM unavailable (%s) — using k-means." % type(exc).__name__)
            from sklearn.cluster import MiniBatchKMeans
            labels = MiniBatchKMeans(n_clusters=k, random_state=42, n_init=3).fit_predict(X)
            n_clusters = k; actual = "kmeans"
    else:
        from sklearn.cluster import MiniBatchKMeans
        labels = MiniBatchKMeans(n_clusters=k, random_state=42, n_init=3).fit_predict(X)
        n_clusters = k; actual = "kmeans"

    send_progress(i, 0.75, "Building heatmap…")

    # Median-expression heatmap (clusters × channels)
    cluster_medians = np.zeros((n_clusters, len(fluor)))
    cluster_counts = np.zeros(n_clusters, dtype=int)
    for c in range(n_clusters):
        mask = labels == c
        cluster_counts[c] = int(mask.sum())
        if mask.sum() > 0:
            cluster_medians[c] = np.median(X[mask], axis=0)

    clusters_out = [{"cluster": int(c), "count": int(cluster_counts[c]),
                     "percent": float(100.0 * cluster_counts[c] / max(1, len(idx)))}
                    for c in range(n_clusters)]
    medians_out = [[round(float(cluster_medians[c, f]), 4) for f in range(len(fluor))]
                   for c in range(n_clusters)]
    return {"method": actual, "k": n_clusters, "n": int(len(idx)),
            "clusters": clusters_out, "channels": list(fluor), "medians": medians_out}


# ---- workspace: FlowJo WSP import / export (FlowKit Session) ---------------
@command("save_wsp")
def _save_wsp(i, a):
    """Export the current samples as a FlowJo .wsp workspace via FlowKit Session."""
    import flowkit as fk
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    path = a.get("file")
    if not path:
        raise ValueError("'file' argument required.")
    session = fk.Session()
    group_name = STATE["experiment"] or "Experiment"
    sample_group = session.add_sample_group(group_name)
    for meta in STATE["meta"]:
        s = _sample_by_name(meta["name"])
        session.add_sample(s, group_name)
    session.export_wsp(path, group_name)
    return {"ok": True, "file": path, "samples": len(STATE["meta"])}


@command("load_wsp")
def _load_wsp(i, a):
    """Import a FlowJo .wsp workspace via FlowKit Session. Updates STATE with the session's
    sample file paths (they must exist on disk next to the .wsp file)."""
    import flowkit as fk
    a = a or {}
    path = a.get("file")
    if not path or not os.path.isfile(path):
        raise ValueError("WSP file not found: %s" % path)
    session = fk.Session(fcs_samples=path)
    files = []
    metas = []
    for group in session.sample_groups:
        for sample_id in session.get_group_sample_ids(group):
            s = session.get_sample(sample_id)
            if hasattr(s, 'original_filename') and s.original_filename:
                f = s.original_filename
            else:
                f = str(s.id)
            files.append(f)
            pnn = list(s.pnn_labels)
            metas.append({"name": os.path.basename(f), "file": f, "pnn": pnn,
                           "pns": list(getattr(s, 'pns_labels', [''] * len(pnn))),
                           "events": int(s.event_count)})
            STATE["cache"][f] = s
    STATE["files"] = files
    STATE["meta"] = metas
    STATE["experiment"] = os.path.splitext(os.path.basename(path))[0]
    return _summary()


# ---- compensation -----------------------------------------------------------

def _find_spill(text):
    """Case-insensitive lookup for $SPILL / $SPILLOVER in an FCS TEXT dict."""
    for key, val in text.items():
        k = key.lower().lstrip("$")
        if k in ("spill", "spillover") and val:
            return val
    return None

def _parse_spill(spill_str):
    """Parse FCS spillover string 'N,ch1,...,chN,v11,...,vNN' → (channels, matrix)."""
    parts = [p.strip() for p in spill_str.split(",")]
    n = int(parts[0])
    channels = parts[1:n + 1]
    vals = [float(p) for p in parts[n + 1:n + 1 + n * n]]
    matrix = [vals[r * n:(r + 1) * n] for r in range(n)]
    return channels, matrix

@command("extract_spillover")
def _extract_spillover(i, a):
    """Extract the embedded $SPILL / $SPILLOVER matrix from the first loaded FCS file
    (or the named sample if specified). Stores the result in STATE so apply_compensation
    can use it."""
    import flowio
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    name = a.get("sample")
    idx = 0
    if name:
        names = [m["name"] for m in STATE["meta"]]
        if name in names:
            idx = names.index(name)
    f = STATE["meta"][idx]["file"]
    fd = flowio.FlowData(f, only_text=True, ignore_offset_error=True)
    spill_str = _find_spill(fd.text)
    if not spill_str:
        raise ValueError(
            "No spillover matrix ($SPILL/$SPILLOVER) found in '%s'.\n"
            "Use Compute to generate one from single-colour controls, or load a file "
            "that was exported with an embedded spillover matrix." % STATE["meta"][idx]["name"])
    channels, matrix = _parse_spill(spill_str)
    STATE["comp_matrix"] = {"channels": channels, "matrix": matrix, "fk_matrix": None}
    return {"channels": channels, "matrix": matrix}

@command("compute_spillover")
def _compute_spillover(i, a):
    """Scan all loaded samples for an embedded spillover matrix, returning the first one found.
    This is a convenience wrapper: instruments often embed the same matrix in every FCS file,
    so searching all files is usually sufficient. Full single-colour peak-detection support
    is a future enhancement."""
    import flowio
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    total = len(STATE["meta"])
    for k, meta in enumerate(STATE["meta"]):
        if is_cancelled(i): raise Cancelled()
        send_progress(i, (k + 0.5) / total, "Scanning %s…" % meta["name"])
        fd = flowio.FlowData(meta["file"], only_text=True, ignore_offset_error=True)
        spill_str = _find_spill(fd.text)
        if spill_str:
            channels, matrix = _parse_spill(spill_str)
            STATE["comp_matrix"] = {"channels": channels, "matrix": matrix, "fk_matrix": None}
            return {"channels": channels, "matrix": matrix}
    raise ValueError(
        "No embedded spillover matrix found in any of the %d loaded file(s).\n"
        "To compute compensation from single-colour controls: load those FCS files "
        "into a separate session and use Extract, then manually enter the matrix." % total)

@command("apply_compensation")
def _apply_compensation(i, a):
    """Apply the spillover matrix to all loaded samples via FlowKit.
    If {channels, matrix} args are provided (e.g. after user edits in the UI), those
    override the stored matrix. Otherwise falls back to STATE["comp_matrix"].
    After this call, get_events with source='comp' returns compensated events."""
    import flowkit as fk
    import numpy as np
    a = a or {}
    if a.get("channels") and a.get("matrix"):
        channels = list(a["channels"])
        matrix = [list(row) for row in a["matrix"]]
        STATE["comp_matrix"] = {"channels": channels, "matrix": matrix, "fk_matrix": None}
    elif not STATE["comp_matrix"]:
        raise ValueError("No compensation matrix available — run Extract or Compute first.")
    else:
        channels = STATE["comp_matrix"]["channels"]
        matrix = STATE["comp_matrix"]["matrix"]

    # FlowKit 1.3.x Matrix signature: (spill_data_or_file, detectors, fluorochromes=None, ...).
    # The spillover is square over the detector channels (rows == cols == channels).
    fk_matrix = fk.Matrix(
        np.array(matrix, dtype=float),
        detectors=channels,
    )
    STATE["comp_matrix"]["fk_matrix"] = fk_matrix
    STATE["comp_applied"] = True

    total = len(STATE["meta"])
    applied = 0
    for k, meta in enumerate(STATE["meta"]):
        if is_cancelled(i): raise Cancelled()
        send_progress(i, (k + 0.5) / total, "Applying to %s…" % meta["name"])
        s = _sample_by_name(meta["name"])
        try:
            s.apply_compensation(fk_matrix)
            applied += 1
        except Exception as exc:
            send_progress(i, (k + 1) / total, "Warning: %s — %s" % (meta["name"], exc))

    return {"channels": channels, "applied": applied}


@command("comp_preview")
def _comp_preview(i, a):
    """Preview the effect of a spillover matrix on ONE channel pair WITHOUT committing it
    globally. Compensates the requested channels in-memory with numpy (comp = raw @ inv(S),
    matching FlowKit's detectors-as-rows convention) so the user can see the before/after of
    *edited* coefficients before clicking Apply. Channels outside the spillover set pass through
    unchanged.

    args: {sample, x, y, channels, matrix}  (channels/matrix optional → falls back to stored)
    Returns {x, y, raw_file, comp_file, rows, cols=2, total}
    """
    import numpy as np
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    s = _sample_by_name(a.get("sample"))
    pnn = list(s.pnn_labels)
    xch, ych = a.get("x"), a.get("y")
    if not xch or xch not in pnn or not ych or ych not in pnn:
        raise ValueError("Both x and y channels must be valid fluorescence channels.")

    channels = list(a.get("channels") or [])
    matrix = [list(r) for r in (a.get("matrix") or [])]
    if not channels or not matrix:
        cm = STATE.get("comp_matrix")
        if not cm:
            raise ValueError("No compensation matrix to preview — Extract or Compute first.")
        channels, matrix = cm["channels"], cm["matrix"]

    # restrict the spillover to channels actually present, preserving matrix order
    present = [c for c in channels if c in pnn]
    keep = [k for k, c in enumerate(channels) if c in pnn]
    S = np.array(matrix, dtype=float)[np.ix_(keep, keep)]
    spill_cols = [pnn.index(c) for c in present]

    raw_all = np.asarray(s.get_events(source="raw"), dtype=float)
    raw_spill = raw_all[:, spill_cols]
    try:
        comp_spill = raw_spill @ np.linalg.inv(S)
    except np.linalg.LinAlgError:
        raise ValueError("Spillover matrix is singular — check for a zero/duplicated row.")

    def comp_col(ch):
        # compensated value if ch is a spillover channel, else the unchanged raw column
        if ch in present:
            return comp_spill[:, present.index(ch)]
        return raw_all[:, pnn.index(ch)]

    raw_pair = np.column_stack([raw_all[:, pnn.index(xch)], raw_all[:, pnn.index(ych)]])
    comp_pair = np.column_stack([comp_col(xch), comp_col(ych)])

    n = raw_pair.shape[0]
    cap = int(a.get("n", 100000))
    if n > cap:
        idx = np.random.default_rng(0).choice(n, cap, replace=False)
        raw_pair = raw_pair[idx]
        comp_pair = comp_pair[idx]

    ts = int(time.time() * 1000)
    raw_path = os.path.join(CONTROL_DIR, "sfcpr_%s_%d.bin" % (i, ts))
    comp_path = os.path.join(CONTROL_DIR, "sfcpc_%s_%d.bin" % (i, ts))
    raw_pair.astype("<f4").tofile(raw_path)
    comp_pair.astype("<f4").tofile(comp_path)
    return {"x": xch, "y": ych, "raw_file": raw_path, "comp_file": comp_path,
            "rows": int(raw_pair.shape[0]), "cols": 2, "total": int(n)}


def _otsu_threshold(vals, bins=256):
    """Otsu's bimodal split on a 1-D array (expects a display space, e.g. arcsinh)."""
    import numpy as np
    finite = vals[np.isfinite(vals)]
    if finite.size < 10:
        return float(np.median(finite)) if finite.size else 0.0
    lo, hi = np.percentile(finite, [0.5, 99.5])
    if hi <= lo:
        return float(hi)
    counts, edges = np.histogram(finite, bins=bins, range=(lo, hi))
    counts = counts.astype(float)
    total = counts.sum()
    if total == 0:
        return float((lo + hi) / 2)
    mids = (edges[:-1] + edges[1:]) / 2
    w0 = np.cumsum(counts)
    w1 = total - w0
    sum_cum = np.cumsum(counts * mids)
    mu_total = sum_cum[-1]
    eps = 1e-12
    mu0 = sum_cum / (w0 + eps)
    mu1 = (mu_total - sum_cum) / (w1 + eps)
    between = w0 * w1 * (mu0 - mu1) ** 2
    return float(mids[int(np.argmax(between))])


def _scatter_cleanup_mask(ev, pnn, fsc, ssc, gate=None):
    """FlowJo-style size cleanup: keep the central scatter population (debris/saturation removed).
    Uses an explicit FSC/SSC rectangle {x_min,x_max,y_min,y_max} when the user has drawn one in the
    wizard; otherwise falls back to a central percentile box. Returns a boolean mask over rows."""
    import numpy as np
    mask = np.ones(ev.shape[0], dtype=bool)
    if gate:
        if fsc and fsc in pnn and "x_min" in gate:
            v = ev[:, pnn.index(fsc)]; mask &= (v >= gate["x_min"]) & (v <= gate["x_max"])
        if ssc and ssc in pnn and "y_min" in gate:
            v = ev[:, pnn.index(ssc)]; mask &= (v >= gate["y_min"]) & (v <= gate["y_max"])
        return mask
    for ch in (fsc, ssc):
        if ch and ch in pnn:
            v = ev[:, pnn.index(ch)]
            fin = v[np.isfinite(v)]
            if fin.size:
                lo, hi = np.percentile(fin, [5, 95])
                mask &= (v >= lo) & (v <= hi)
    return mask


@command("compute_spillover_from_controls")
def _compute_spillover_from_controls(i, a):
    """Compute a spillover matrix from single-colour controls + an (optional) universal
    negative, FlowJo-style (Bagwell & Adams 1993):

      1. size-cleanup gate (FSC/SSC percentile box) on every control,
      2. split each single-stain control's PRIMARY detector into positive/negative (Otsu in
         arcsinh space); require >100 positive events and >=2% of the gated parent,
      3. spillover[p][d] = median(positive_d) - negative_baseline_d  (unstained autofluorescence
         if a universal negative is supplied, else the control's own negative population),
      4. normalise each row so its primary detector = 1.0.

    args: {unstained?, controls?:[{sample,channel}], scatter?:{x,y}}
          (controls auto-assigned by brightest detector when omitted)
    Returns {channels, matrix, controls:[{channel,sample,threshold,threshold_t,n_pos,n_neg,
             pct_pos,ok,hist:{x,counts}}], scatter:{x,y}, unstained}
    """
    import numpy as np
    if not STATE["meta"]:
        raise ValueError("Load the control FCS files first.")
    a = a or {}

    pnn0 = list(STATE["meta"][0]["pnn"])
    fluor = [c for c in pnn0 if not _scatter(c)]
    if len(fluor) < 2:
        raise ValueError("Need at least 2 fluorescence detectors for compensation.")

    scatter = a.get("scatter") or {}
    fsc = scatter.get("x") or next((c for c in pnn0 if re.search(r"FSC.?A|FSC", c, re.I)), None)
    ssc = scatter.get("y") or next((c for c in pnn0 if re.search(r"SSC.?A|SSC", c, re.I)), None)
    sgate = scatter.get("gate")   # optional FSC/SSC rectangle drawn in the wizard (overrides the box)

    unstained = a.get("unstained")
    controls_in = a.get("controls")
    sample_names = [m["name"] for m in STATE["meta"]]

    def _brightest(name, used):
        """Detector with the highest gated median (FlowJo's auto-matching heuristic)."""
        s = _sample_by_name(name)
        ev = np.asarray(s.get_events(source="raw"), dtype=float)
        pnn = list(s.pnn_labels)
        mask = _scatter_cleanup_mask(ev, pnn, fsc, ssc, sgate)
        evg = ev[mask] if mask.any() else ev
        best, best_med = None, -np.inf
        for c in fluor:
            med = float(np.median(evg[:, pnn.index(c)]))
            if med > best_med and c not in used:
                best_med, best = med, c
        return best

    # default: every non-unstained sample is a single-stain control (auto-assigned)
    if controls_in is None:
        controls_in = [{"sample": nm} for nm in sample_names if not (unstained and nm == unstained)]

    # resolve each control's detector, auto-assigning by brightest where unspecified
    controls, used = [], set()
    for ctrl in controls_in:
        nm = ctrl.get("sample")
        if not nm or nm not in sample_names:
            continue
        ch = ctrl.get("channel") or _brightest(nm, used)
        if ch:
            used.add(ch)
            controls.append({"sample": nm, "channel": ch})

    if not controls:
        raise ValueError("No single-stain controls — assign controls or load single-stain files.")

    detectors = [c["channel"] for c in controls]

    # universal-negative autofluorescence baseline per detector
    baseline = {}
    if unstained and unstained in sample_names:
        su = _sample_by_name(unstained)
        evu = np.asarray(su.get_events(source="raw"), dtype=float)
        pnnu = list(su.pnn_labels)
        mu = _scatter_cleanup_mask(evu, pnnu, fsc, ssc, sgate)
        evug = evu[mu] if mu.any() else evu
        for d in detectors:
            baseline[d] = float(np.median(evug[:, pnnu.index(d)]))

    # per-control positive/negative threshold overrides (arcsinh space), keyed by sample name —
    # set when the user drags the split on a control's separation histogram.
    overrides = a.get("threshold_overrides") or {}

    n = len(detectors)
    matrix = [[1.0 if r == c else 0.0 for c in range(n)] for r in range(n)]
    reports = []

    for ri, ctrl in enumerate(controls):
        if is_cancelled(i):
            raise Cancelled()
        send_progress(i, (ri + 0.5) / n, "Gating %s…" % ctrl["sample"])
        name, primary = ctrl["sample"], ctrl["channel"]
        s = _sample_by_name(name)
        ev = np.asarray(s.get_events(source="raw"), dtype=float)
        pnn = list(s.pnn_labels)
        mask = _scatter_cleanup_mask(ev, pnn, fsc, ssc, sgate)
        evg = ev[mask] if mask.any() else ev
        prim = evg[:, pnn.index(primary)]

        prim_t = np.arcsinh(prim / 150.0)
        thr_t = float(overrides[name]) if name in overrides else _otsu_threshold(prim_t)
        thr = float(np.sinh(thr_t) * 150.0)
        pos_mask = prim >= thr
        neg_mask = ~pos_mask
        n_pos, n_neg = int(pos_mask.sum()), int(neg_mask.sum())
        pct_pos = 100.0 * n_pos / max(1, evg.shape[0])
        ok = n_pos > 100 and pct_pos >= 2.0

        def neg_med(d):
            if d in baseline:
                return baseline[d]
            col = evg[:, pnn.index(d)]
            return float(np.median(col[neg_mask])) if neg_mask.any() else 0.0

        pos = evg[pos_mask] if pos_mask.any() else evg
        signal = {d: float(np.median(pos[:, pnn.index(d)])) - neg_med(d) for d in detectors}
        denom = signal[primary] if abs(signal[primary]) > 1e-9 else 1.0
        for ci, d in enumerate(detectors):
            matrix[ri][ci] = round(signal[d] / denom, 5)
        matrix[ri][ri] = 1.0

        finite = prim_t[np.isfinite(prim_t)]
        lo, hi = (np.percentile(finite, [0.5, 99.5]) if finite.size else (0.0, 1.0))
        counts, edges = np.histogram(finite, bins=80, range=(lo, hi))
        xmid = 0.5 * (edges[:-1] + edges[1:])
        reports.append({
            "channel": primary, "sample": name,
            "threshold": thr, "threshold_t": thr_t,
            "n_pos": n_pos, "n_neg": n_neg, "pct_pos": round(pct_pos, 2), "ok": bool(ok),
            "hist": {"x": xmid.tolist(), "counts": counts.tolist()},
        })

    STATE["comp_matrix"] = {"channels": detectors, "matrix": matrix, "fk_matrix": None}
    return {"channels": detectors, "matrix": matrix, "controls": reports,
            "scatter": {"x": fsc, "y": ssc}, "unstained": unstained}


# ---- transformation ---------------------------------------------------------

@command("list_fluor_channels")
def _list_fluor_channels(i, a):
    """Return only the fluorescence channels (non-scatter, non-time) from the loaded panel."""
    if not STATE["meta"]:
        return {"channels": []}
    fluor = [c for c in STATE["meta"][0]["pnn"] if not _scatter(c)]
    return {"channels": fluor}

@command("apply_transformation")
def _apply_transformation(i, a):
    """Record the chosen transform for all (or selected) fluorescence channels.
    JavaFX/CytoPlot applies transforms natively for rendering; this command keeps
    the engine in sync so GatingML export and future source='xform' get_events work."""
    import numpy as np
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    method = str(a.get("method", "logicle")).lower()
    if method not in ("logicle", "arcsinh", "log"):
        raise ValueError("Unknown transform: %s.  Use logicle, arcsinh, or log." % method)
    cofactor = float(a.get("cofactor", 150)) if method == "arcsinh" else None
    all_fluor = [c for c in STATE["meta"][0]["pnn"] if not _scatter(c)]
    req = a.get("channels")
    channels = [c for c in (req if req else all_fluor) if c in STATE["meta"][0]["pnn"]]
    if not channels:
        raise ValueError("No valid channels selected for transformation.")

    # Apply via FlowKit transforms to each cached sample so source='xform' works.
    # FlowKit 1.3.x constructors take GatingML params only — no transform_id, no cofactor kwarg.
    try:
        import flowkit as fk
        if method == "arcsinh":
            xf = fk.transforms.AsinhTransform(param_t=10000, param_m=4.5, param_a=0)
        elif method == "log":
            xf = fk.transforms.LogTransform(param_t=262144, param_m=4.5)
        else:  # logicle
            xf = fk.transforms.LogicleTransform(param_t=262144, param_w=0.5,
                                                 param_m=4.5, param_a=0)
        for ch in channels:
            STATE["transforms"][ch] = {"method": method, "cofactor": cofactor,
                                       "fk_transform": xf}
    except Exception:
        for ch in channels:
            STATE["transforms"][ch] = {"method": method, "cofactor": cofactor}

    return {"channels": channels, "method": method,
            "cofactor": cofactor, "n": len(channels)}

@command("get_metadata")
def _get_metadata(i, a):
    """Return all FCS TEXT segment keywords for a sample as a flat {key: value} dict.
    Useful for the FCS keyword viewer in the UI and for GatingML annotation."""
    import flowio
    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    name = a.get("sample")
    idx = 0
    if name:
        names = [m["name"] for m in STATE["meta"]]
        if name in names:
            idx = names.index(name)
    f = STATE["meta"][idx]["file"]
    fd = flowio.FlowData(f, only_text=True, ignore_offset_error=True)
    keywords = {str(k): str(v) for k, v in fd.text.items()}
    return {"sample": STATE["meta"][idx]["name"], "keywords": keywords, "count": len(keywords)}

# ---- workspace save / load (.sfw is a self-contained JSON document) ----------
@command("save_workspace")
def _save_workspace(i, a):
    """Write the session to a .sfw JSON: experiment, sample file paths, compensation,
    transforms, and the gating trees (passed in by the Java side, which owns them)."""
    a = a or {}
    file = a.get("file")
    if not file:
        raise ValueError("file is required.")
    cm = STATE.get("comp_matrix")
    doc = {
        "version": 1,
        "experiment": STATE.get("experiment", "Untitled Experiment"),
        "files": list(STATE.get("files", [])),
        "comp_applied": bool(STATE.get("comp_applied")),
        "comp_matrix": ({"channels": cm["channels"], "matrix": cm["matrix"]} if cm else None),
        "transforms": {k: {"method": v.get("method"), "cofactor": v.get("cofactor")}
                       for k, v in STATE.get("transforms", {}).items()},
        "gates": a.get("gates", {}) or {},
        "audit_log": a.get("audit_log", []) or [],   # session analysis log (owned by the Java side)
    }
    with open(file, "w", encoding="utf-8") as fh:
        json.dump(doc, fh, indent=2, default=str)
    n_gates = sum(len(v) for v in doc["gates"].values()) if isinstance(doc["gates"], dict) else 0
    return {"file": file, "n_samples": len(doc["files"]), "n_gates": n_gates}

@command("load_workspace")
def _load_workspace(i, a):
    """Restore a .sfw document into the engine STATE and return a setup summary plus the
    saved gating trees (the Java side rebuilds its population tree from them)."""
    a = a or {}
    file = a.get("file")
    if not file or not os.path.isfile(file):
        raise ValueError("Workspace file not found: %s" % file)
    with open(file, "r", encoding="utf-8") as fh:
        doc = json.load(fh)

    files = [f for f in doc.get("files", []) if os.path.isfile(f)]
    missing = len(doc.get("files", [])) - len(files)
    STATE["experiment"] = doc.get("experiment", "Untitled Experiment")
    STATE["files"] = files
    STATE["meta"] = [_read_header(f) for f in files]
    STATE["cache"] = {}
    STATE["gating"] = None
    STATE["transforms"] = {k: {"method": v.get("method"), "cofactor": v.get("cofactor")}
                           for k, v in (doc.get("transforms") or {}).items()}

    cm = doc.get("comp_matrix")
    STATE["comp_matrix"] = None
    STATE["comp_applied"] = False
    if cm and cm.get("channels") and cm.get("matrix"):
        STATE["comp_matrix"] = {"channels": cm["channels"], "matrix": cm["matrix"], "fk_matrix": None}
        if doc.get("comp_applied"):
            try:    # rebuild the FlowKit matrix so source='comp' works immediately
                import flowkit as fk, numpy as np
                STATE["comp_matrix"]["fk_matrix"] = fk.Matrix(
                    np.array(cm["matrix"], dtype=float), detectors=cm["channels"])
                STATE["comp_applied"] = True
            except Exception:
                STATE["comp_applied"] = False

    out = _summary(skipped=missing)
    out["gates"] = doc.get("gates", {}) or {}
    out["audit_log"] = doc.get("audit_log", []) or []
    return out

@command("suggest_transform")
def _suggest_transform(i, a):
    """Analyse fluorescence channels from the first loaded sample and recommend a transform.
    Returns {suggestions: {channel: {recommend, reason, pct_negative}}, summary: {logicle_count, arcsinh_count}}."""
    import numpy as np
    if not STATE["meta"]:
        return {"suggestions": {}, "summary": {"logicle_count": 0, "arcsinh_count": 0}}
    s = _sample_by_name(None)   # first sample
    ev = np.asarray(s.get_events(source="raw"))
    pnn = list(s.pnn_labels)
    fluor = [c for c in pnn if not _scatter(c)]
    suggestions = {}
    for ch in fluor:
        col = pnn.index(ch)
        vals = ev[:, col]
        n = len(vals)
        if n == 0:
            continue
        pct_neg = float(100.0 * np.sum(vals < 0) / n)
        if pct_neg > 2.0:
            rec = "logicle"
            reason = "%.1f%% negative values" % pct_neg
        else:
            rec = "arcsinh"
            reason = "wide dynamic range — arcsinh/150 recommended"
        suggestions[ch] = {"recommend": rec, "reason": reason, "pct_negative": round(pct_neg, 1)}
    logicle_count = sum(1 for v in suggestions.values() if v["recommend"] == "logicle")
    return {"suggestions": suggestions,
            "summary": {"logicle_count": logicle_count,
                        "arcsinh_count": len(suggestions) - logicle_count}}


def _apply_gate_mask(ev, pnn, gate):
    """Return a boolean mask for events that pass one gate definition dict."""
    import numpy as np
    g_type = str(gate.get("type", "polygon")).lower()
    xc = gate.get("x_channel") or ""
    yc = gate.get("y_channel") or ""
    xs_g = [float(v) for v in gate.get("xs", [])]
    ys_g = [float(v) for v in gate.get("ys", [])]
    if not xc or xc not in pnn:
        return np.ones(len(ev), dtype=bool)
    xi = pnn.index(xc)
    ex = ev[:, xi]
    if g_type in ("range", "h_range"):
        if len(xs_g) < 2:
            return np.ones(len(ev), dtype=bool)
        return (ex >= xs_g[0]) & (ex <= xs_g[-1])
    if not yc or yc not in pnn:
        return np.ones(len(ev), dtype=bool)
    yi = pnn.index(yc)
    ey = ev[:, yi]
    if len(xs_g) < 3:
        return np.ones(len(ev), dtype=bool)
    from matplotlib.path import Path
    poly = np.array(list(zip(xs_g, ys_g)), dtype=float)
    return Path(poly).contains_points(np.column_stack([ex, ey]))


def _phase_boundaries(mu1, sig1, mu2, sig2, lo, hi):
    """G0/G1 ↔ S and S ↔ G2/M split points from the fit, kept ordered + inside the data range."""
    g1_hi = mu1 + 1.5 * sig1
    g2_lo = mu2 - 1.5 * sig2
    if not (g1_hi < g2_lo):                     # peaks overlap → split at the midpoint
        mid = 0.5 * (mu1 + mu2)
        g1_hi, g2_lo = mid - 1e-6, mid + 1e-6
    g1_hi = min(max(g1_hi, lo), hi)
    g2_lo = min(max(g2_lo, lo), hi)
    return {"lo": lo, "g1_hi": g1_hi, "g2_lo": g2_lo, "hi": hi}


def _sanitize_bounds(lb, ub, p0):
    """Make every lower bound strictly less than its upper bound and clamp the seed p0 into range.
    Curve-fit raises 'Each lower bound must be strictly less than each upper bound' when peak
    detection collapses a range (degenerate data / wrong channel); this keeps the fit runnable."""
    lb = [float(v) for v in lb]
    ub = [float(v) for v in ub]
    p0 = [float(v) for v in p0]
    for k in range(len(lb)):
        if not (ub[k] > lb[k]):
            ub[k] = lb[k] + max(abs(lb[k]) * 1e-3, 1e-6)
        if p0[k] < lb[k]: p0[k] = lb[k]
        if p0[k] > ub[k]: p0[k] = ub[k]
    return lb, ub, p0


# ---- analysis A1: cell cycle (Watson / Dean-Jett-Fox) ----------------------
@command("run_cell_cycle")
def _run_cell_cycle(i, a):
    """Fit a DNA-content histogram to a cell-cycle model and return phase fractions.

    Models:
      watson  — G0/G1 + G2/M are Gaussians with mean_G2M = 2*mean_G0G1 and
                EQUAL CV (sigma_G2M = 2*sigma_G0G1). S phase = broadened rectangle.
      djf     — Dean-Jett-Fox: same structure but sigma_G2M is a free parameter.

    Optional args:
      gate_polygons  — [{type, x_channel, y_channel, xs, ys}] chain to filter events.
      mu_g1_hint     — float; seed the G1 peak mean from a user-dragged anchor.
      mu_g2_hint     — float; seed via G2/2 when the user dragged the G2 anchor.

    Returns {phases, cvs, fit, curves, boundaries, n} — no PNG; Java renders natively.
    """
    import numpy as np
    from scipy.optimize import curve_fit
    from scipy.signal import find_peaks
    from scipy.special import erf

    a = a or {}
    model = str(a.get("model", "watson")).lower()
    mu_g1_hint = a.get("mu_g1_hint")
    mu_g2_hint = a.get("mu_g2_hint")
    gate_polygons = a.get("gate_polygons")  # list of gate dicts to apply sequentially

    # The UI can fit a GATED population by passing raw values directly, or let the engine
    # load the sample and optionally filter through gate_polygons.
    raw_values = a.get("values")
    if raw_values is not None:
        channel = a.get("channel", "DNA")
        vals = np.asarray(raw_values, dtype=float)
    else:
        if not STATE["meta"]:
            raise ValueError("Load data first.")
        s = _sample_by_name(a.get("sample"))
        pnn = list(s.pnn_labels)
        channel = a.get("channel")
        if not channel or channel not in pnn:
            dna = [c for c in pnn if re.search(r"PI|DAPI|7.?AAD|Hoechst|DNA|FxCycle|DRAQ", c, re.I)]
            channel = dna[0] if dna else None
            if channel is None:
                raise ValueError("Specify a DNA-content channel (PI / DAPI / 7-AAD / Hoechst).")
        ev = np.asarray(s.get_events(source="raw"))
        # Apply gate chain to select the correct sub-population
        if gate_polygons:
            mask = np.ones(len(ev), dtype=bool)
            for gate in gate_polygons:
                mask &= _apply_gate_mask(ev, pnn, gate)
            ev = ev[mask]
        vals = ev[:, pnn.index(channel)]

    vals = np.asarray(vals, dtype=float)
    vals = vals[np.isfinite(vals)]
    vals = vals[vals > 0]                       # DNA stains are linear, positive
    if vals.size < 100:
        raise ValueError("Too few events with positive %s signal for cell-cycle fitting." % channel)
    hi = np.percentile(vals, 99.5)              # trim doublets / debris tail
    vals = vals[vals <= hi]

    nbins = int(a.get("bins", 256))
    counts, edges = np.histogram(vals, bins=nbins)
    x = 0.5 * (edges[:-1] + edges[1:])
    y = counts.astype(float)
    dx = float(x[1] - x[0])

    # --- seed peak detection on a lightly-smoothed histogram ---
    ys_sm = np.convolve(y, np.ones(5) / 5, mode="same")
    peaks, props = find_peaks(ys_sm, prominence=ys_sm.max() * 0.05, distance=max(1, nbins // 40))
    if len(peaks) == 0:
        raise ValueError("No clear peak found in the %s histogram." % channel)
    g1_idx = peaks[int(np.argmax(props["prominences"]))]      # G1 = most prominent
    mu1_seed = float(x[g1_idx])
    # If the user dragged a peak anchor, use that as the seed instead of auto-detection
    if mu_g1_hint is not None:
        mu1_seed = float(mu_g1_hint)
    elif mu_g2_hint is not None:
        mu1_seed = float(mu_g2_hint) / 2.0
    g2_idx = None
    if len(peaks) > 1:
        cand = peaks[int(np.argmin(np.abs(x[peaks] - 2 * mu1_seed)))]
        if x[cand] >= mu1_seed * 1.4:
            g2_idx = cand
    a1_seed = float(y[g1_idx])
    a2_seed = float(y[g2_idx]) if g2_idx is not None else a1_seed * 0.15
    s1_seed = max(mu1_seed * 0.05, dx)
    as_seed = max(a1_seed * 0.05, 1.0)
    sqrt2 = np.sqrt(2.0)
    # When peak hints are provided, tighten the mu1 bounds to stay near the user's annotation
    hint_given = (mu_g1_hint is not None or mu_g2_hint is not None)
    mu1_lo_frac = 0.90 if hint_given else 0.6
    mu1_hi_frac = 1.10 if hint_given else 1.4

    def gauss(xx, amp, mu, sig):
        return amp * np.exp(-0.5 * ((xx - mu) / sig) ** 2)

    def sbox(xx, amp, m1, m2, sig):
        return 0.5 * amp * (erf((m2 - xx) / (sig * sqrt2)) - erf((m1 - xx) / (sig * sqrt2)))

    if model == "watson":
        def f(xx, a1, mu1, sig1, a2, a_s):
            return (gauss(xx, a1, mu1, sig1) + gauss(xx, a2, 2 * mu1, 2 * sig1)
                    + sbox(xx, a_s, mu1, 2 * mu1, sig1))
        p0 = [a1_seed, mu1_seed, s1_seed, a2_seed, as_seed]
        lb = [0, mu1_seed * mu1_lo_frac, dx, 0, 0]
        ub = [a1_seed * 3, mu1_seed * mu1_hi_frac, mu1_seed * 0.2, a1_seed * 3, a1_seed]
    else:  # dean-jett-fox: free G2 sigma
        def f(xx, a1, mu1, sig1, a2, sig2, a_s):
            return (gauss(xx, a1, mu1, sig1) + gauss(xx, a2, 2 * mu1, sig2)
                    + sbox(xx, a_s, mu1, 2 * mu1, sig1))
        p0 = [a1_seed, mu1_seed, s1_seed, a2_seed, s1_seed * 1.2, as_seed]
        lb = [0, mu1_seed * mu1_lo_frac, dx, 0, dx, 0]
        ub = [a1_seed * 3, mu1_seed * mu1_hi_frac, mu1_seed * 0.2, a1_seed * 3, mu1_seed * 0.3, a1_seed]

    lb, ub, p0 = _sanitize_bounds(lb, ub, p0)

    send_progress(i, 0.4, "Fitting %s model…" % model.upper())
    try:
        popt, _ = curve_fit(f, x, y, p0=p0, bounds=(lb, ub), maxfev=20000)
    except Exception as exc:
        raise ValueError("Cell-cycle fit did not converge: %s" % exc)

    if model == "watson":
        a1, mu1, sig1, a2, a_s = popt
        sig2 = 2 * sig1
    else:
        a1, mu1, sig1, a2, sig2, a_s = popt
    mu2 = 2 * mu1

    g1_c = gauss(x, a1, mu1, sig1)
    g2_c = gauss(x, a2, mu2, sig2)
    s_c = sbox(x, a_s, mu1, mu2, sig1)
    fit_c = g1_c + g2_c + s_c

    area_g1 = float(np.sum(g1_c) * dx)
    area_g2 = float(np.sum(g2_c) * dx)
    area_s = float(np.sum(s_c) * dx)
    total = area_g1 + area_g2 + area_s
    if total <= 0:
        raise ValueError("Cell-cycle fit produced no area — check the channel selection.")
    pct_g1 = 100.0 * area_g1 / total
    pct_s = 100.0 * area_s / total
    pct_g2 = 100.0 * area_g2 / total
    cv_g1 = 100.0 * sig1 / mu1
    cv_g2 = 100.0 * sig2 / mu2

    ss_res = float(np.sum((y - fit_c) ** 2))
    ss_tot = float(np.sum((y - np.mean(y)) ** 2))
    r2 = 1.0 - ss_res / ss_tot if ss_tot > 0 else 0.0
    rmse = float(np.sqrt(np.mean((y - fit_c) ** 2)))

    send_progress(i, 0.95, "Done.")
    return {"model": model, "channel": channel,
            "phases": {"G0G1": round(pct_g1, 2), "S": round(pct_s, 2), "G2M": round(pct_g2, 2)},
            "cvs": {"G0G1": round(cv_g1, 2), "G2M": round(cv_g2, 2)},
            "fit": {"mu_g1": round(float(mu1), 1), "mu_g2": round(float(mu2), 1),
                    "r2": round(r2, 4), "rmse": round(rmse, 2)},
            "boundaries": _phase_boundaries(float(mu1), float(sig1), float(mu2), float(sig2),
                                            float(vals.min()), float(vals.max())),
            "curves": {"x":     [round(float(v), 3) for v in x],
                       "hist":  [round(float(v), 2) for v in y],
                       "g1":    [round(float(v), 2) for v in g1_c],
                       "s":     [round(float(v), 2) for v in s_c],
                       "g2":    [round(float(v), 2) for v in g2_c],
                       "total": [round(float(v), 2) for v in fit_c]},
            "n": int(vals.size)}


# ---- analysis A2: proliferation index (dye-dilution) -----------------------
@command("run_proliferation")
def _run_proliferation(i, a):
    """Fit a dye-dilution histogram (CFSE / CTV / BrdU / Ki-67) with a sum of
    equal-sigma Gaussians whose means are spaced by one halving (log2) per
    generation. Generation 0 is the brightest (undivided parent).

    Reports the FlowJo-standard, precursor-corrected indices:
      P_g  = N_g / 2^g                       (precursor cohort that produced gen g)
      DI   = Σ(g·P_g) / Σ(P_g)               (Division Index, over ALL precursors)
      PI   = Σ_{g≥1}(g·P_g) / Σ_{g≥1}(P_g)   (Proliferation Index, divided cells only)
      %divided = Σ_{g≥1}(P_g) / Σ(P_g) · 100

    Returns {PI, DI, pct_divided, generations:[{gen,count,pct}], png, n}.
    """
    import numpy as np
    from scipy.optimize import curve_fit
    from scipy.signal import find_peaks

    if not STATE["meta"]:
        raise ValueError("Load data first.")
    a = a or {}
    s = _sample_by_name(a.get("sample"))
    pnn = list(s.pnn_labels)
    channel = a.get("channel")
    if not channel or channel not in pnn:
        raise ValueError("Specify a dye-dilution channel (CFSE / CTV / Ki-67 …).")
    n_peaks = int(a.get("n_peaks", 8))
    gate_polygons = a.get("gate_polygons")

    ev = np.asarray(s.get_events(source="raw"))
    if gate_polygons:
        mask = np.ones(len(ev), dtype=bool)
        for gate in gate_polygons:
            mask &= _apply_gate_mask(ev, pnn, gate)
        ev = ev[mask]
    col = pnn.index(channel)
    vals = ev[:, col]
    vals = vals[np.isfinite(vals) & (vals > 0)]     # log space → strictly positive
    if vals.size < 100:
        raise ValueError("Too few positive %s events for proliferation fitting." % channel)
    lo, hi = np.percentile(vals, [0.5, 99.7])
    vals = vals[(vals >= lo) & (vals <= hi)]
    xv = np.log10(vals)                              # generations are evenly spaced in log10

    nbins = int(a.get("bins", 256))
    counts, edges = np.histogram(xv, bins=nbins)
    x = 0.5 * (edges[:-1] + edges[1:])
    y = counts.astype(float)
    dx = float(x[1] - x[0])
    LOG2 = float(np.log10(2.0))                      # one division = -log10(2) in log10 space

    ys = np.convolve(y, np.ones(5) / 5, mode="same")
    peaks, props = find_peaks(ys, prominence=ys.max() * 0.03, distance=max(1, int(LOG2 / dx * 0.5)))
    if len(peaks) == 0:
        raise ValueError("No peaks found in the %s histogram." % channel)
    mu0_seed = float(x[peaks].max())                 # gen 0 = brightest (rightmost) peak
    sig_seed = max(LOG2 * 0.18, dx)
    # Cap generations to those whose peak CENTRE stays inside the data range — otherwise
    # the dimmest "off-screen" generation overfits the left boundary tail and inflates PI/DI.
    max_feasible = int((mu0_seed - x[0]) / LOG2) + 1
    n_peaks = max(2, min(n_peaks, max_feasible))

    def model(xx, mu0, delta, sig, *amps):
        out = np.zeros_like(xx)
        for g, ag in enumerate(amps):
            out = out + ag * np.exp(-0.5 * ((xx - (mu0 - g * delta)) / sig) ** 2)
        return out

    amp_seed = []
    for g in range(n_peaks):
        c = mu0_seed - g * LOG2
        j = int(np.clip(round((c - x[0]) / dx), 0, nbins - 1))
        amp_seed.append(max(y[j], 1.0))
    p0 = [mu0_seed, LOG2, sig_seed] + amp_seed
    lb = [mu0_seed - LOG2, LOG2 * 0.7, dx] + [0.0] * n_peaks
    ub = [mu0_seed + LOG2, LOG2 * 1.3, LOG2 * 0.6] + [y.max() * 1.5] * n_peaks
    lb, ub, p0 = _sanitize_bounds(lb, ub, p0)

    send_progress(i, 0.4, "Fitting %d generations…" % n_peaks)
    try:
        popt, _ = curve_fit(lambda xx, *p: model(xx, *p), x, y,
                            p0=p0, bounds=(lb, ub), maxfev=40000)
    except Exception as exc:
        raise ValueError("Proliferation fit did not converge: %s" % exc)

    mu0, delta, sig = popt[0], popt[1], popt[2]
    amps = np.array(popt[3:])
    sqrt2pi = np.sqrt(2.0 * np.pi)
    areas = amps * sig * sqrt2pi                      # Gaussian areas ∝ cell counts per generation
    # Keep generations CONTIGUOUSLY from gen 0, stopping at the first peak that drops below
    # 2% of the largest — generations are physically contiguous, so a real gap ends the series.
    thr = areas.max() * 0.02
    gens = []
    for g in range(n_peaks):
        if areas[g] >= thr:
            gens.append(g)
        else:
            break
    if not gens:
        raise ValueError("No generation peaks above threshold.")

    N = {g: float(areas[g]) for g in gens}
    P = {g: N[g] / (2 ** g) for g in gens}           # precursor cohorts
    sumP = sum(P.values())
    sumP_div = sum(P[g] for g in gens if g >= 1)
    DI = sum(g * P[g] for g in gens) / sumP if sumP > 0 else 0.0
    PI = (sum(g * P[g] for g in gens if g >= 1) / sumP_div) if sumP_div > 0 else 0.0
    pct_divided = 100.0 * sumP_div / sumP if sumP > 0 else 0.0

    total_cells = sum(N.values())
    generations = [{"gen": g, "count": int(round(N[g])),
                    "pct": round(100.0 * N[g] / total_cells, 2)} for g in gens]

    # Build per-generation curves for the interactive JavaFX chart (no matplotlib needed)
    fit_c = np.zeros_like(x)
    gen_curves = []
    for g in gens:
        gc = amps[g] * np.exp(-0.5 * ((x - (mu0 - g * delta)) / sig) ** 2)
        fit_c = fit_c + gc
        gen_curves.append({"gen": g, "y": gc.tolist()})

    return {"channel": channel,
            "PI": round(float(PI), 3), "DI": round(float(DI), 3),
            "pct_divided": round(float(pct_divided), 2),
            "generations": generations, "n": int(vals.size),
            "curves": {
                "x": x.tolist(),
                "hist": y.tolist(),
                "generations": gen_curves,
                "total": fit_c.tolist()
            }}


# ---- analysis A4: statistical comparison between groups --------------------
def _stars(p):
    return "***" if p < 0.001 else "**" if p < 0.01 else "*" if p < 0.05 else "ns"

def _dunn_posthoc(groups):
    """Dunn's test with Bonferroni correction (manual — no scikit_posthocs dependency).
    groups: dict {name: [values]}. Returns list of {pair, p, stars}."""
    import numpy as np
    from scipy.stats import norm
    names = list(groups.keys())
    data = [np.asarray(groups[n], float) for n in names]
    all_vals = np.concatenate(data)
    n_total = all_vals.size
    # rank all observations together (average ranks for ties)
    order = np.argsort(all_vals, kind="mergesort")
    ranks = np.empty(n_total, float)
    ranks[order] = np.arange(1, n_total + 1)
    # tie correction: average ranks within tie groups
    uniq, inv, cnt = np.unique(all_vals, return_inverse=True, return_counts=True)
    sums = np.zeros(uniq.size)
    np.add.at(sums, inv, ranks)
    avg = sums / cnt
    ranks = avg[inv]
    # mean rank per group
    sizes = [d.size for d in data]
    mean_ranks, offset = [], 0
    for sz in sizes:
        mean_ranks.append(np.mean(ranks[offset:offset + sz]))
        offset += sz
    # tie correction term for the variance
    tie_term = np.sum(cnt ** 3 - cnt)
    n_pairs = len(names) * (len(names) - 1) // 2
    out = []
    for a_ in range(len(names)):
        for b_ in range(a_ + 1, len(names)):
            na, nb = sizes[a_], sizes[b_]
            sigma = np.sqrt((n_total * (n_total + 1) / 12.0
                             - tie_term / (12.0 * (n_total - 1))) * (1.0 / na + 1.0 / nb))
            if sigma == 0:
                p = 1.0
            else:
                z = abs(mean_ranks[a_] - mean_ranks[b_]) / sigma
                p = 2.0 * (1.0 - norm.cdf(z))
            p_adj = min(1.0, p * n_pairs)            # Bonferroni
            out.append({"pair": "%s vs %s" % (names[a_], names[b_]),
                        "p": round(float(p_adj), 5), "stars": _stars(p_adj)})
    return out

@command("run_stats_comparison")
def _run_stats_comparison(i, a):
    """Compare a gate's frequency (or any per-sample metric) across groups, choosing the
    appropriate non-parametric test automatically:
      2 groups, unpaired → Mann-Whitney U
      2 groups, paired   → Wilcoxon signed-rank
      ≥3 groups          → Kruskal-Wallis + Dunn post-hoc (Bonferroni)

    args: {gate, groups:{name:[values]}, paired:bool}
    Returns {test, p_value, stars, posthoc:[...], png, groups:[{name,n,median}]}.
    """
    import numpy as np
    from scipy import stats
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    a = a or {}
    gate = a.get("gate", "population")
    paired = bool(a.get("paired", False))
    raw = a.get("groups") or {}
    groups = {k: [float(v) for v in vals] for k, vals in raw.items() if len(vals) >= 2}
    if len(groups) < 2:
        raise ValueError("Need at least 2 groups with ≥2 values each. "
                         "Assign samples to groups and make sure their %s frequencies are computed." % gate)

    names = list(groups.keys())
    data = [np.asarray(groups[n], float) for n in names]

    posthoc = []
    if len(groups) == 2:
        if paired:
            if data[0].size != data[1].size:
                raise ValueError("Paired test requires equal group sizes (%d vs %d)."
                                 % (data[0].size, data[1].size))
            stat, p = stats.wilcoxon(data[0], data[1])
            test = "Wilcoxon signed-rank (paired)"
        else:
            stat, p = stats.mannwhitneyu(data[0], data[1], alternative="two-sided")
            test = "Mann-Whitney U"
    else:
        stat, p = stats.kruskal(*data)
        test = "Kruskal-Wallis + Dunn (Bonferroni)"
        if p < 0.05:
            posthoc = _dunn_posthoc(groups)

    group_summ = [{"name": names[k], "n": int(data[k].size),
                   "median": round(float(np.median(data[k])), 4),
                   "values": [round(float(v), 4) for v in data[k].tolist()]}
                  for k in range(len(names))]
    return {"test": test, "p_value": round(float(p), 6),
            "stars": _stars(p), "posthoc": posthoc, "groups": group_summ, "gate": gate}


# ---- analysis A3: apoptosis (Annexin V / PI quadrant) ----------------------
@command("run_apoptosis")
def _run_apoptosis(i, a):
    """Apoptosis quadrant analysis for Annexin V + PI (or 7-AAD) assays.

    Detects thresholds at density valleys between live and dying populations
    using 1-D histogram valley-finding (Savitzky-Golay smoothed), then counts
    quadrant membership and produces a coloured scatter plot.

    args: {sample, annexin_channel, pi_channel}
    Returns {quadrants:{live, early_apoptotic, late_apoptotic, necrotic},
             thresholds:{annexin, pi}, n, png, annexin_channel, pi_channel}
    """
    import numpy as np
    from scipy.signal import find_peaks, savgol_filter
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    a = a or {}
    sample = a.get("sample") or (STATE["meta"][0]["name"] if STATE["meta"] else None)
    if not sample:
        raise ValueError("No sample specified.")
    annexin_ch = a.get("annexin_channel")
    pi_ch = a.get("pi_channel")
    if not annexin_ch or not pi_ch:
        raise ValueError("annexin_channel and pi_channel are required.")
    gate_polygons = a.get("gate_polygons")

    s = _sample_by_name(sample)
    events = s.as_dataframe(
        source="comp" if STATE.get("comp_applied") else "raw",
        col_multi_index=False)
    if annexin_ch not in events.columns:
        raise ValueError("Channel '%s' not found in sample '%s'." % (annexin_ch, sample))
    if pi_ch not in events.columns:
        raise ValueError("Channel '%s' not found in sample '%s'." % (pi_ch, sample))

    if gate_polygons:
        pnn = list(events.columns)
        ev_np = events.values
        mask = np.ones(len(ev_np), dtype=bool)
        for gate in gate_polygons:
            mask &= _apply_gate_mask(ev_np, pnn, gate)
        events = events[mask]

    ann_vals = events[annexin_ch].values.astype(float)
    pi_vals  = events[pi_ch].values.astype(float)
    n = len(ann_vals)

    def _valley_threshold(vals, bins=256):
        """1-D valley between the two dominant populations; falls back to 95th %ile."""
        finite = vals[np.isfinite(vals)]
        lo  = np.percentile(finite, 0.5)
        hi  = np.percentile(finite, 99.5)
        counts, edges = np.histogram(finite, bins=bins, range=(lo, hi))
        mids = (edges[:-1] + edges[1:]) / 2
        win = max(5, (bins // 10) * 2 + 1)   # odd window for savgol
        smooth = savgol_filter(counts.astype(float), win, 2)
        smooth = np.maximum(smooth, 0)
        peaks, _ = find_peaks(smooth, height=smooth.max() * 0.05, distance=bins // 8)
        if len(peaks) >= 2:
            p1, p2 = peaks[0], peaks[1]
            valley_idx = p1 + int(np.argmin(smooth[p1:p2 + 1]))
            return float(mids[valley_idx])
        return float(np.percentile(finite, 95))

    ann_thresh = _valley_threshold(ann_vals)
    pi_thresh  = _valley_threshold(pi_vals)

    ann_pos = ann_vals >= ann_thresh
    pi_pos  = pi_vals  >= pi_thresh

    n_live     = int(np.sum(~ann_pos & ~pi_pos))
    n_early    = int(np.sum( ann_pos & ~pi_pos))
    n_late     = int(np.sum( ann_pos &  pi_pos))
    n_necrotic = int(np.sum(~ann_pos &  pi_pos))

    q = {
        "live":             round(100.0 * n_live     / n, 2),
        "early_apoptotic":  round(100.0 * n_early    / n, 2),
        "late_apoptotic":   round(100.0 * n_late     / n, 2),
        "necrotic":         round(100.0 * n_necrotic / n, 2),
    }

    # scatter plot (subsample to 5 k for speed)
    sub = min(n, 5000)
    rng = np.random.default_rng(0)
    idx = rng.choice(n, sub, replace=False)
    ax_s = ann_vals[idx]
    py_s = pi_vals[idx]

    # colour by quadrant membership
    color_arr = np.where(
        ann_pos[idx] &  pi_pos[idx], "#E05555",    # late   → red
        np.where(
        ann_pos[idx] & ~pi_pos[idx], "#F5A623",    # early  → orange
        np.where(
        ~ann_pos[idx] &  pi_pos[idx], "#9B59B6",   # necrotic → purple
        "#4CAF50")))                                # live   → green

    fig, ax = plt.subplots(figsize=(6, 6), facecolor="white")
    ax.set_facecolor("white")
    ax.scatter(ax_s, py_s, s=1.5, c=color_arr, alpha=0.4, linewidths=0)
    ax.axvline(ann_thresh, color="#444", linewidth=1.2, linestyle="--")
    ax.axhline(pi_thresh,  color="#444", linewidth=1.2, linestyle="--")

    def _qlabel(txt, xa, ya):
        ax.text(xa, ya, txt, transform=ax.transAxes, fontsize=9,
                ha="center", va="center",
                bbox=dict(facecolor="white", edgecolor="#ccc",
                          alpha=0.75, boxstyle="round,pad=0.25"))
    _qlabel("Necrotic\n%.1f%%" % q["necrotic"],          0.13, 0.88)
    _qlabel("Late Apoptotic\n%.1f%%" % q["late_apoptotic"], 0.87, 0.88)
    _qlabel("Live\n%.1f%%" % q["live"],                   0.13, 0.12)
    _qlabel("Early Apoptotic\n%.1f%%" % q["early_apoptotic"], 0.87, 0.12)

    ax.set_xlabel(annexin_ch)
    ax.set_ylabel(pi_ch)
    ax.set_title("%s  —  Apoptosis  (n=%d)" % (sample, n))
    fig.tight_layout()
    png_path = os.path.join(CONTROL_DIR,
                            "sfap_%s_%d.png" % (i, int(time.time() * 1000)))
    fig.savefig(png_path, dpi=120, facecolor="white")
    plt.close(fig)

    return {
        "quadrants":        q,
        "thresholds":       {"annexin": round(ann_thresh, 4),
                             "pi":      round(pi_thresh,  4)},
        "n":                n,
        "png":              png_path,
        "annexin_channel":  annexin_ch,
        "pi_channel":       pi_ch,
        "events":           {"ann": ax_s.tolist(), "pi": py_s.tolist()},
    }


# ---- analysis A5: kinetic / time-course ------------------------------------
@command("run_kinetic")
def _run_kinetic(i, a):
    """Bin events by the Time channel, compute per-bin median MFI, group by label.

    args: {samples:[name], channel, time_channel="Time", bins=100,
           groups:{sample_name: group_label}}
    Returns {groups:[{name, times, mfi, sd}], png, channel, time_channel, n_samples}
    """
    import numpy as np
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    a = a or {}
    sample_names = a.get("samples") or [m["name"] for m in STATE["meta"]]
    channel       = a.get("channel")
    time_ch       = a.get("time_channel", "Time")
    bins_n        = int(a.get("bins", 100))
    grp_map       = a.get("groups") or {}
    gate_polygons = a.get("gate_polygons")

    if not channel:
        raise ValueError("channel is required.")

    per_sample = {}
    t_min, t_max = float("inf"), float("-inf")

    for name in sample_names:
        try:
            s = _sample_by_name(name)
            ev = s.as_dataframe(
                source="comp" if STATE.get("comp_applied") else "raw",
                col_multi_index=False)
        except Exception:
            continue
        if time_ch not in ev.columns or channel not in ev.columns:
            continue
        if gate_polygons:
            pnn = list(ev.columns)
            ev_np = ev.values
            mask = np.ones(len(ev_np), dtype=bool)
            for gate in gate_polygons:
                mask &= _apply_gate_mask(ev_np, pnn, gate)
            ev = ev[mask]
        t = ev[time_ch].values.astype(float)
        v = ev[channel].values.astype(float)
        t_min = min(t_min, float(np.nanmin(t)))
        t_max = max(t_max, float(np.nanmax(t)))
        per_sample[name] = (t, v)

    if not per_sample:
        raise ValueError(
            "No samples found with both '%s' and '%s' channels." % (time_ch, channel))

    edges = np.linspace(t_min, t_max, bins_n + 1)
    mids  = (edges[:-1] + edges[1:]) / 2

    grp_series = {}
    for name, (t, v) in per_sample.items():
        grp = grp_map.get(name, name)
        bidx = np.clip(np.digitize(t, edges) - 1, 0, bins_n - 1)
        row = [float(np.median(v[bidx == b])) if (bidx == b).any() else float("nan")
               for b in range(bins_n)]
        grp_series.setdefault(grp, []).append(row)

    group_results = []
    for gname, rows in grp_series.items():
        arr  = np.array(rows, dtype=float)
        mean = np.nanmean(arr, axis=0)
        sd   = np.nanstd(arr, axis=0)
        group_results.append({
            "name":  gname,
            "times": [round(float(x), 4) for x in mids],
            "mfi":   [round(float(x), 4) for x in mean],
            "sd":    [round(float(x), 4) for x in sd],
        })

    return {"groups": group_results,
            "channel": channel, "time_channel": time_ch,
            "bins": bins_n, "n_samples": len(per_sample)}


# ---- analysis A6: sample classifier (PCA + optional Random Forest) ---------
@command("run_classifier")
def _run_classifier(i, a):
    """PCA biplot from per-sample gate feature vectors; optional RF if group labels given.

    args: {features:{sample:{feat:value}}, group_labels:{sample:group}}
    Returns {method, components:[{x,y,label,group}], loadings:[{feat,pc1,pc2}],
             importance:[{feat,value}], top_loading_indices, n_samples, n_features, var_explained}
    """
    import numpy as np
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from sklearn.preprocessing import StandardScaler
    from sklearn.decomposition import PCA

    a = a or {}
    raw_feat = a.get("features") or {}
    grp_lbl  = a.get("group_labels") or {}

    if len(raw_feat) < 2:
        raise ValueError("Need at least 2 samples with gate feature vectors.")

    sample_names = list(raw_feat.keys())
    feat_names   = sorted({f for feats in raw_feat.values() for f in feats})
    if not feat_names:
        raise ValueError("Feature vectors are empty — gate at least one population first.")

    X = np.array([[float(raw_feat[s].get(f, 0.0)) for f in feat_names]
                  for s in sample_names], dtype=float)
    X_sc  = StandardScaler().fit_transform(X)
    n_comp = min(2, X_sc.shape[1], X_sc.shape[0] - 1)
    if n_comp < 1:
        raise ValueError("Need ≥ 2 features and ≥ 3 samples for PCA.")
    pca = PCA(n_components=n_comp)
    Z   = pca.fit_transform(X_sc)
    L   = pca.components_.T   # (n_feats, n_comp)

    method     = "PCA"
    importance = []
    unique_grps = sorted(set(grp_lbl.values()))
    if len(unique_grps) >= 2:
        from sklearn.ensemble import RandomForestClassifier
        y  = [grp_lbl.get(s, "?") for s in sample_names]
        rf = RandomForestClassifier(n_estimators=200, random_state=0)
        rf.fit(X_sc, y)
        top = sorted(zip(feat_names, rf.feature_importances_), key=lambda x: -x[1])[:10]
        importance = [{"feat": f, "value": round(float(v), 4)} for f, v in top]
        method = "PCA + Random Forest"

    ve = pca.explained_variance_ratio_
    mag  = np.sqrt(np.sum(L[:, :n_comp] ** 2, axis=1))
    top8 = np.argsort(mag)[-8:].tolist()

    comps_out = [{"x": round(float(Z[j, 0]), 4),
                  "y": round(float(Z[j, 1]) if n_comp > 1 else 0.0, 4),
                  "label": sample_names[j],
                  "group": grp_lbl.get(sample_names[j], "")}
                 for j in range(len(sample_names))]
    loads_out = [{"feat": feat_names[j],
                  "pc1":  round(float(L[j, 0]), 4),
                  "pc2":  round(float(L[j, 1]) if n_comp > 1 else 0.0, 4)}
                 for j in range(len(feat_names))]

    return {"method": method, "components": comps_out, "loadings": loads_out,
            "importance": importance, "top_loading_indices": top8,
            "n_samples": len(sample_names), "n_features": len(feat_names),
            "var_explained": [round(float(v), 4) for v in ve]}


# ---- export: GatingML 2.0 --------------------------------------------------
@command("save_gatingml")
def _save_gatingml(i, a):
    """Write gates as GatingML 2.0 XML.

    args: {file, gates:{sample:[{id,name,parent_id,type,x_channel,y_channel,xs,ys}]}}
    Returns {file, n_gates}
    """
    import xml.etree.ElementTree as ET

    a = a or {}
    file_path = a.get("file")
    if not file_path:
        raise ValueError("file path is required.")
    gate_map = a.get("gates") or {}

    GML = "http://www.isac-net.org/std/Gating-ML/v2.0/gating"
    DT  = "http://www.isac-net.org/std/Gating-ML/v2.0/datatypes"
    ET.register_namespace("gating",    GML)
    ET.register_namespace("data-type", DT)

    root_el  = ET.Element("{%s}Gating-ML" % GML)
    n_gates  = 0

    def _dim(parent_el, ch):
        d  = ET.SubElement(parent_el, "{%s}dimension" % GML)
        d.set("{%s}compensation-ref" % GML, "uncompensated")
        fd = ET.SubElement(d, "{%s}fcs-dimension" % DT)
        fd.set("{%s}name" % DT, ch)
        return d

    for _sample, gates in gate_map.items():
        for g in gates:
            gid    = str(g.get("id",   g.get("name", "gate%d" % n_gates)))
            gname  = str(g.get("name", gid))
            parent = g.get("parent_id")
            gtype  = str(g.get("type", "polygon"))
            x_ch   = str(g.get("x_channel", ""))
            y_ch   = str(g.get("y_channel", ""))
            xs     = [float(v) for v in (g.get("xs") or [])]
            ys     = [float(v) for v in (g.get("ys") or [])]

            if gtype == "interval" and xs:
                el = ET.SubElement(root_el, "{%s}RectangleGate" % GML)
                el.set("{%s}id" % GML, gid)
                el.set("{%s}eventsInside" % GML, "true")
                if gname:  el.set("{%s}name" % GML, gname)
                if parent: el.set("{%s}parent_id" % GML, parent)
                d = _dim(el, x_ch)
                d.set("{%s}min" % GML, str(xs[0]))
                d.set("{%s}max" % GML, str(xs[1] if len(xs) > 1 else xs[0]))

            elif gtype == "rectangle" and xs and ys:
                el = ET.SubElement(root_el, "{%s}RectangleGate" % GML)
                el.set("{%s}id" % GML, gid)
                el.set("{%s}eventsInside" % GML, "true")
                if gname:  el.set("{%s}name" % GML, gname)
                if parent: el.set("{%s}parent_id" % GML, parent)
                d = _dim(el, x_ch)
                d.set("{%s}min" % GML, str(min(xs)))
                d.set("{%s}max" % GML, str(max(xs)))
                d = _dim(el, y_ch)
                d.set("{%s}min" % GML, str(min(ys)))
                d.set("{%s}max" % GML, str(max(ys)))

            elif xs and ys:
                # polygon / ellipse (stored as sampled polygon vertices)
                el = ET.SubElement(root_el, "{%s}PolygonGate" % GML)
                el.set("{%s}id" % GML, gid)
                el.set("{%s}eventsInside" % GML, "true")
                if gname:  el.set("{%s}name" % GML, gname)
                if parent: el.set("{%s}parent_id" % GML, parent)
                _dim(el, x_ch)
                _dim(el, y_ch)
                for vx, vy in zip(xs, ys):
                    v  = ET.SubElement(el, "{%s}vertex" % GML)
                    cx = ET.SubElement(v, "{%s}coordinate" % GML)
                    cx.set("{%s}data-type" % DT, "double")
                    cx.set("{%s}value"     % DT, str(vx))
                    cy = ET.SubElement(v, "{%s}coordinate" % GML)
                    cy.set("{%s}data-type" % DT, "double")
                    cy.set("{%s}value"     % DT, str(vy))
            else:
                continue
            n_gates += 1

    try:
        ET.indent(root_el, space="  ")
    except AttributeError:
        pass  # ET.indent requires Python 3.9+
    tree = ET.ElementTree(root_el)
    tree.write(file_path, xml_declaration=True, encoding="UTF-8")
    return {"file": file_path, "n_gates": n_gates}


# ---- export: FCS 3.1 -------------------------------------------------------
@command("export_fcs")
def _export_fcs(i, a):
    """Export a sample's events to FCS 3.1 (LISTMODE FLOAT32, little-endian).

    args: {sample, file, compensated:bool}
    Returns {file, sample, n, channels, source}
    """
    import numpy as np

    a = a or {}
    sample     = a.get("sample") or (STATE["meta"][0]["name"] if STATE["meta"] else None)
    file_path  = a.get("file")
    compensated = bool(a.get("compensated", STATE.get("comp_applied", False)))

    if not sample:
        raise ValueError("No sample specified.")
    if not file_path:
        raise ValueError("file path is required.")

    s   = _sample_by_name(sample)
    src = "comp" if (compensated and STATE.get("comp_applied")) else "raw"
    ev  = s.as_dataframe(source=src, col_multi_index=False)

    channel_names = list(ev.columns)
    data = ev.values.astype(np.float32)
    n_events, n_chan = data.shape

    delim = b"\\"

    def _kv(key, val):
        return delim + key.encode("ascii") + delim + str(val).encode("ascii")

    pairs = [
        _kv("$BEGINANALYSIS", "0"),
        _kv("$ENDANALYSIS", "0"),
        _kv("$BYTEORD", "1,2,3,4"),
        _kv("$DATATYPE", "F"),
        _kv("$MODE", "L"),
        _kv("$NEXTDATA", "0"),
        _kv("$PAR", str(n_chan)),
        _kv("$TOT", str(n_events)),
    ]
    for p, ch in enumerate(channel_names, 1):
        pairs += [_kv("$P%dB" % p, "32"),
                  _kv("$P%dN" % p, ch),
                  _kv("$P%dE" % p, "0,0"),
                  _kv("$P%dR" % p, "262144")]

    text_body = b"".join(pairs)

    HEADER_SIZE = 256
    TEXT_START  = HEADER_SIZE
    DATA_BYTES  = n_events * n_chan * 4

    # Two-pass to get stable $BEGINDATA/$ENDDATA values.
    DATA_START = 0
    DATA_END   = 0
    for _pass in range(2):
        suffix = (_kv("$BEGINDATA", str(DATA_START))
                  + _kv("$ENDDATA", str(DATA_END)))
        full_text = text_body + suffix
        TEXT_END  = TEXT_START + len(full_text) - 1
        DATA_START = TEXT_END + 1
        DATA_END   = DATA_START + DATA_BYTES - 1

    header_str = "FCS3.1    %6d%6d%6d%6d%6d%6d" % (
        TEXT_START, TEXT_END, DATA_START, DATA_END, 0, 0)
    header = header_str.encode("ascii").ljust(HEADER_SIZE)[:HEADER_SIZE]

    with open(file_path, "wb") as f:
        f.write(header)
        f.write(full_text)
        f.write(data.flatten().tobytes())

    return {"file": file_path, "sample": sample,
            "n": n_events, "channels": n_chan, "source": src}


# ---- compensation residual diagnostic (differentiator #17) -----------------
@command("comp_residual")
def _comp_residual(i, a):
    """Pearson correlation heatmap of compensated fluorescence channels.

    After good compensation, fluorescence channels should be ~uncorrelated; a
    residual |r| > 0.2 flags over/under-compensation between that channel pair.

    args: {sample, threshold=0.2}
    Returns {channels, matrix, flagged:[{a,b,r,sign}], png, sample, n}
    """
    import numpy as np
    import matplotlib
    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    a = a or {}
    sample = a.get("sample") or (STATE["meta"][0]["name"] if STATE["meta"] else None)
    if not sample:
        raise ValueError("No sample specified.")
    threshold = float(a.get("threshold", 0.2))

    s = _sample_by_name(sample)
    ev = s.as_dataframe(
        source="comp" if STATE.get("comp_applied") else "raw",
        col_multi_index=False)
    fluor = [c for c in ev.columns if not _scatter(str(c))]
    # drop a Time-like channel if present
    fluor = [c for c in fluor if str(c).lower() != "time"]
    if len(fluor) < 2:
        raise ValueError("Need at least 2 fluorescence channels for a residual diagnostic.")

    X = ev[fluor].values.astype(float)
    n = X.shape[0]
    # subsample large files for speed; correlation is stable
    if n > 50000:
        rng = np.random.default_rng(0)
        X = X[rng.choice(n, 50000, replace=False)]

    R = np.corrcoef(X, rowvar=False)
    R = np.nan_to_num(R, nan=0.0)

    flagged = []
    k = len(fluor)
    for r_ in range(k):
        for c_ in range(r_ + 1, k):
            rv = float(R[r_, c_])
            if abs(rv) > threshold:
                flagged.append({"a": fluor[r_], "b": fluor[c_],
                                "r": round(rv, 3),
                                "sign": "under" if rv > 0 else "over"})
    flagged.sort(key=lambda f: -abs(f["r"]))

    fig, ax = plt.subplots(figsize=(max(5, k * 0.6), max(4.5, k * 0.6)), facecolor="white")
    im = ax.imshow(R, cmap="RdBu_r", vmin=-1, vmax=1)
    ax.set_xticks(range(k)); ax.set_yticks(range(k))
    ax.set_xticklabels(fluor, rotation=90, fontsize=7)
    ax.set_yticklabels(fluor, fontsize=7)
    # annotate cells above threshold
    for r_ in range(k):
        for c_ in range(k):
            if r_ != c_ and abs(R[r_, c_]) > threshold:
                ax.text(c_, r_, "%.2f" % R[r_, c_], ha="center", va="center",
                        fontsize=6, color="black")
    fig.colorbar(im, ax=ax, fraction=0.046, pad=0.04, label="Pearson r")
    ax.set_title("Compensation residual — %s\n%d pair(s) |r|>%.2f"
                 % (sample, len(flagged), threshold))
    fig.tight_layout()
    png_path = os.path.join(CONTROL_DIR,
                            "sfcr_%s_%d.png" % (i, int(time.time() * 1000)))
    fig.savefig(png_path, dpi=120, facecolor="white")
    plt.close(fig)

    return {"channels": fluor,
            "matrix": [[round(float(R[r_, c_]), 4) for c_ in range(k)] for r_ in range(k)],
            "flagged": flagged, "png": png_path, "sample": sample, "n": int(X.shape[0])}


# --- dispatch + main loop ---------------------------------------------------
def dispatch(req):
    i = req.get("id")
    cmd = req.get("cmd")
    if not cmd:
        send_error(i, "Request has no 'cmd'"); return
    fn = COMMANDS.get(cmd)
    if fn is None:
        send_error(i, "Unknown command: %s" % cmd); return
    try:
        payload = fn(i, req.get("args"))
        if payload is not None:
            send_result(i, payload)
    except Cancelled:
        send({"type": "cancelled", "id": i})
    except Exception as e:
        send_error(i, e, traceback.format_exc().splitlines()[-12:])
    finally:
        flag = os.path.join(CONTROL_DIR, "cancel_%s.flag" % i)
        if os.path.exists(flag):
            try: os.remove(flag)
            except OSError: pass

def _prewarm():
    """Import the heavy libs in the background so the FIRST get_events/load is fast.
    Without this the first FCS open pays the one-time flowkit/numpy import cost (~1-3s)."""
    try:
        import numpy            # noqa: F401
        import flowkit          # noqa: F401
    except Exception:
        pass

def main():
    send_ready()
    threading.Thread(target=_prewarm, name="prewarm", daemon=True).start()
    for line in sys.stdin:          # EOF -> parent gone -> exit (orphan safety)
        line = line.strip()
        if not line:
            continue
        if line == "__shutdown__":
            break
        try:
            req = json.loads(line)
        except Exception:
            send_error(None, "Malformed JSON request"); continue
        dispatch(req)

if __name__ == "__main__":
    main()
