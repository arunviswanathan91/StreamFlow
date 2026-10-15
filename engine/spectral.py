"""Spectral unmixing maths for StreamFLOW. Pure NumPy/SciPy/scikit-learn - no engine state.

Clean-room from published methods only (StreamFLOW is Apache-2.0; nothing here is adapted from
the LGPL Honeychrome checkout):

  * Ordinary / weighted least squares unmixing - Novo, Gregori & Rajwa, Cytometry A 2013:
        X = E . Wt,   W = (M Omega^-1 M^T)^-1 M Omega^-1
    with M the (fluorophores x detectors) signature matrix and Omega = diag(per-detector mean),
    the Poisson-noise weighting. OLS is the Omega = I special case.
  * Per-cell autofluorescence - an unstained-derived library of AF spectra; each event is unmixed
    against every candidate AF signature and keeps the one with the lowest residual.
  * Signature QC - cosine similarity matrix and a collinearity index from the inverse Gram matrix.

Conventions: events are (N x D) raw detector values; signatures are rows, each scaled to peak 1.
Anything feeding a reported number is seeded (see `seed` arguments), per the engine invariant.
"""
import numpy as np

_EPS = 1e-12


def _otsu(v, bins=256):
    """Otsu threshold of a 1-D array, or None when the data are (near) constant."""
    v = np.asarray(v, dtype=float)
    v = v[np.isfinite(v)]
    if v.size < 2 or np.ptp(v) <= 0:
        return None
    hist, edges = np.histogram(v, bins=bins)
    p = hist / hist.sum()
    centres = (edges[:-1] + edges[1:]) / 2.0
    w0 = np.cumsum(p)
    m = np.cumsum(p * centres)
    mt = m[-1]
    denom = w0 * (1.0 - w0)
    denom[denom <= 0] = np.nan
    between = (mt * w0 - m) ** 2 / denom
    if np.all(np.isnan(between)):
        return None
    return float(centres[int(np.nanargmax(between))])


def saturation_mask(events, ceiling=None, frac=0.999):
    """True for events NOT pinned at the top of the detector range in any detector.

    Pinned events bias positive medians low (the true signal is clipped) and distort every
    signature that includes them, so they are dropped before signatures are built. `ceiling` is
    the per-detector range maximum (e.g. $PnR); when omitted the observed column maximum is used.
    """
    e = np.asarray(events, dtype=float)
    top = np.max(e, axis=0) if ceiling is None else np.asarray(ceiling, dtype=float)
    return ~np.any(e >= frac * top, axis=1)


def peak_detector(events):
    """Index of the detector that best characterises a single-stain control.

    First principal component of the (uncentred) events, argmax of its absolute loading - robust
    to a dim control whose brightest *median* detector is dominated by autofluorescence.
    """
    e = np.asarray(events, dtype=float)
    if e.shape[0] < 2:
        return int(np.argmax(np.median(e, axis=0)))
    # uncentred covariance: the dominant direction is the control's spectrum itself
    _, _, vt = np.linalg.svd(e - np.median(e, axis=0), full_matrices=False)
    return int(np.argmax(np.abs(vt[0])))


def build_signature(events, negative=None, min_pos=50, saturation=None):
    """Signature of one single-stain control.

    events:   (N x D) raw values of the control.
    negative: optional (D,) autofluorescence baseline (e.g. unstained median) subtracted from the
              positive median; when omitted the control's own negative population is used.
    Returns {signature (D,), peak, n_pos, n_neg, threshold, ok, reason}. `signature` is scaled so
    its peak detector is 1.0.
    """
    e = np.asarray(events, dtype=float)
    out = {"ok": False, "reason": "", "n_pos": 0, "n_neg": 0, "peak": None,
           "threshold": None, "signature": None, "n_saturated": 0}
    if e.ndim != 2 or e.shape[0] < min_pos * 2:
        out["reason"] = "too few events (%d)" % (e.shape[0] if e.ndim == 2 else 0)
        return out
    if saturation is not None:
        keep = saturation_mask(e, saturation)
        out["n_saturated"] = int((~keep).sum())
        if keep.sum() >= min_pos * 2:
            e = e[keep]
    pk = peak_detector(e)
    out["peak"] = pk
    thr = _otsu(np.arcsinh(e[:, pk] / 150.0))
    if thr is None:
        out["reason"] = "no bimodal split on the peak detector"
        return out
    thr_raw = float(np.sinh(thr) * 150.0)
    out["threshold"] = thr_raw
    pos = e[:, pk] > thr_raw
    out["n_pos"], out["n_neg"] = int(pos.sum()), int((~pos).sum())
    if out["n_pos"] < min_pos:
        out["reason"] = "only %d positive events" % out["n_pos"]
        return out
    base = np.median(e[~pos], axis=0) if negative is None else np.asarray(negative, dtype=float)
    if negative is None and out["n_neg"] < min_pos:
        base = np.zeros(e.shape[1])
    sig = np.median(e[pos], axis=0) - base
    sig = np.clip(sig, 0.0, None)  # a spectrum has no negative lobes; noise only
    peak_val = sig[pk]
    if peak_val <= 0:
        out["reason"] = "positive population is not brighter than the negative"
        return out
    out["signature"] = sig / peak_val
    out["ok"] = True
    return out


def _span_residual(rows, v, rel_tol=1e-9):
    """Norm of `v` after removing its projection onto span(rows), by modified Gram-Schmidt with
    re-orthogonalisation. Rank-tolerant: a row that is (numerically) in the span of earlier rows adds
    nothing to the basis, so a *dependent* set is handled instead of silently producing garbage."""
    basis = []
    for r in rows:
        w = np.array(r, dtype=float)
        n0 = np.linalg.norm(w)
        for _ in range(2):
            for q in basis:
                w -= (q @ w) * q
        n1 = np.linalg.norm(w)
        if n0 > 0 and n1 > rel_tol * n0:
            basis.append(w / n1)
    w = np.array(v, dtype=float)
    for _ in range(2):
        for q in basis:
            w -= (q @ w) * q
    return float(np.linalg.norm(w))


def signature_qc(M, rel_tol=1e-9):
    """Per-pair cosine similarity and a per-fluorophore collinearity index.

    M is (F x D). The collinearity index of row i is ||m_i|| / dist(m_i, span of the other rows),
    which equals sqrt((G^-1)_ii) * ||m_i|| for the Gram matrix G = M M^T but is computed without
    inverting G. It is 1.0 for an orthogonal set and grows without bound as a fluorophore becomes
    explainable by the others (unmixing then amplifies noise by that factor); it is `inf` when the
    fluorophore lies in the span of the others to within `rel_tol` - a dependent set cannot be unmixed.
    (Earlier this inverted G directly: for a dependent set numpy returned rounding garbage and the
    clip to >= 0 turned it into 0.0, i.e. "not collinear" - the opposite of the truth.)
    """
    M = np.asarray(M, dtype=float)
    norms = np.linalg.norm(M, axis=1, keepdims=True)
    unit = M / np.maximum(norms, _EPS)
    cos = unit @ unit.T
    coll = np.empty(M.shape[0])
    for i in range(M.shape[0]):
        others = np.delete(M, i, axis=0)
        res = _span_residual(others, M[i], rel_tol)
        coll[i] = np.inf if res <= rel_tol * norms[i, 0] or norms[i, 0] == 0 else norms[i, 0] / res
    return {"cosine": cos, "collinearity": coll,
            "condition": float(np.linalg.cond(M)) if M.size else float("nan")}


def unmixing_matrix(M, weights=None):
    """W (F x D) such that abundances = events @ W.T.

    weights None -> OLS. weights (D,) -> WLS with Omega = diag(weights), i.e.
    W = (M Omega^-1 M^T)^-1 M Omega^-1. Weights are floored so a silent detector cannot dominate.
    """
    M = np.asarray(M, dtype=float)
    if weights is None:
        gram = M @ M.T
        return np.linalg.solve(gram, M)
    w = np.maximum(np.asarray(weights, dtype=float), _EPS)
    inv_w = 1.0 / w
    mw = M * inv_w[None, :]            # M Omega^-1
    return np.linalg.solve(mw @ M.T, mw)


def poisson_weights(events, floor=1.0):
    """Per-detector mean signal of the events: the Poisson variance estimate used as Omega."""
    e = np.asarray(events, dtype=float)
    return np.maximum(np.mean(np.clip(e, 0, None), axis=0), floor)


def unmix(events, M, method="wls", weights=None):
    """Unmix (N x D) events against M (F x D). Returns (abundances (N x F), W (F x D))."""
    e = np.asarray(events, dtype=float)
    m = str(method).lower()
    if m == "ols":
        W = unmixing_matrix(M, None)
    elif m == "wls":
        W = unmixing_matrix(M, poisson_weights(e) if weights is None else weights)
    else:
        raise ValueError("Unknown unmixing method %r (use 'ols' or 'wls')." % method)
    return e @ W.T, W


def extract_af_library(unstained, k=4, seed=0, min_events=200):
    """Cluster an unstained sample's spectra into up to `k` autofluorescence signatures.

    Spectra are normalised to unit L2 norm first so clusters group by *shape* (which is what
    unmixing sees), not by brightness. Returns (library (K x D) peak-scaled, counts (K,)).
    Seeded: identical input and seed give identical output.
    """
    from sklearn.cluster import KMeans
    e = np.clip(np.asarray(unstained, dtype=float), 0, None)
    norms = np.linalg.norm(e, axis=1)
    e = e[norms > 0]
    if e.shape[0] < min_events:
        raise ValueError("Unstained sample has only %d usable events (need >= %d) for an "
                         "autofluorescence library." % (e.shape[0], min_events))
    unit = e / np.linalg.norm(e, axis=1, keepdims=True)
    k = int(max(1, min(k, e.shape[0] // 50)))
    km = KMeans(n_clusters=k, n_init=10, random_state=int(seed)).fit(unit)
    lib, counts = [], []
    for c in range(k):
        grp = e[km.labels_ == c]
        sig = np.median(grp, axis=0)
        lib.append(sig / max(sig.max(), _EPS))
        counts.append(int(grp.shape[0]))
    order = np.argsort(counts)[::-1]
    return np.asarray(lib)[order], np.asarray(counts)[order]


def unmix_with_af(events, M, af_library, method="wls", weights=None, chunk=50000):
    """Per-cell autofluorescence: unmix each event against M plus every candidate AF signature
    and keep the candidate with the lowest residual.

    Returns (abundances (N x (F+1)) - last column is the AF abundance, af_index (N,), residual (N,)).
    """
    e = np.asarray(events, dtype=float)
    lib = np.asarray(af_library, dtype=float)
    w = poisson_weights(e) if (weights is None and str(method).lower() == "wls") else weights
    n, f = e.shape[0], M.shape[0]
    out = np.zeros((n, f + 1))
    best_idx = np.zeros(n, dtype=np.int32)
    best_res = np.full(n, np.inf)
    for k in range(lib.shape[0]):
        Mk = np.vstack([M, lib[k]])
        Wk = unmixing_matrix(Mk, None if str(method).lower() == "ols" else w)
        for s in range(0, n, chunk):
            blk = e[s:s + chunk]
            ab = blk @ Wk.T
            res = np.sum((blk - ab @ Mk) ** 2, axis=1)
            better = res < best_res[s:s + chunk]
            sl = slice(s, s + blk.shape[0])
            out[sl][better] = ab[better]
            best_idx[sl][better] = k
            best_res[sl][better] = res[better]
    return out, best_idx, best_res


def residuals(events, M, abundances):
    """Per-event root-sum-square residual: how much of each event the model failed to explain."""
    e = np.asarray(events, dtype=float)
    return np.sqrt(np.sum((e - np.asarray(abundances) @ np.asarray(M)) ** 2, axis=1))


def write_fcs_f32(path, names, data):
    """Write (N x C) float32 data as a spec-conformant FCS 3.1 LISTMODE file (little-endian):
    8-character header offset fields and a closing delimiter after the last TEXT value."""
    data = np.asarray(data, dtype="<f4")
    n_events, n_chan = data.shape
    d = b"/"
    for nm in names:
        if "/" in nm:
            raise ValueError("Channel name %r contains the FCS delimiter '/'." % nm)

    def kv(key, val):
        return key.encode("ascii") + d + str(val).encode("ascii", "replace") + d

    pairs = [kv("$BYTEORD", "1,2,3,4"), kv("$DATATYPE", "F"), kv("$MODE", "L"),
             kv("$NEXTDATA", 0), kv("$PAR", n_chan), kv("$TOT", n_events)]
    for p, ch in enumerate(names, 1):
        pairs += [kv("$P%dB" % p, 32), kv("$P%dN" % p, ch), kv("$P%dE" % p, "0,0"),
                  kv("$P%dR" % p, 262144)]
    body = b"".join(pairs)
    data_bytes = n_events * n_chan * 4
    t_start = 256                       # TEXT begins after the 58-byte header block
    d_start = d_end = 0
    for _ in range(3):                  # offsets are self-referential; iterate to a fixed point
        text = d + body + kv("$BEGINDATA", d_start) + kv("$ENDDATA", d_end)
        t_end = t_start + len(text) - 1
        d_start, d_end = t_end + 1, t_end + data_bytes
    fits = d_end <= 99999999
    hdr = "FCS3.1    %8d%8d%8d%8d%8d%8d" % (t_start, t_end, d_start if fits else 0,
                                            d_end if fits else 0, 0, 0)
    with open(path, "wb") as f:
        f.write(hdr.encode("ascii").ljust(t_start))
        f.write(text)
        f.write(data.tobytes())
