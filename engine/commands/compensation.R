# StreamFLOW engine — Compensation commands (Phase 2)
# Spillover matrix: extract embedded ($SPILLOVER), compute from single-colour
# controls, set/edit, and apply. Uses flowCore primitives (headless) — the
# CytoExploreR spillover_compute helper is interactive and cannot run here.

# Convert a spillover matrix to a JSON-friendly payload.
matrix_payload <- function(mat) {
  ch <- colnames(mat)
  list(channels = ch,
       matrix   = lapply(seq_len(nrow(mat)), function(i) as.numeric(mat[i, ])))
}

# Rebuild a matrix from a {channels, matrix} payload (edited/uploaded by the UI).
parse_matrix <- function(args) {
  ch  <- unlist(args$channels, use.names = FALSE)
  rows <- args$matrix
  m <- do.call(rbind, lapply(rows, function(r) as.numeric(unlist(r))))
  dimnames(m) <- list(ch, ch)
  m
}

# Coerce the working data (cytoset) to a flowSet for flowCore::spillover.
.as_flowset <- function(cs) {
  if (methods::is(cs, "flowSet")) return(cs)
  flowWorkspace::cytoset_to_flowSet(cs)
}

.find_channel <- function(chans, patterns) {
  for (p in patterns) {
    hit <- chans[grepl(p, chans, ignore.case = TRUE)]
    if (length(hit)) return(hit[1])
  }
  NA_character_
}

# Extract the spillover matrix embedded in an FCS file's keywords ($SPILLOVER).
COMMANDS$extract_spillover <- function(id, args) {
  .require_pkg("flowCore")
  cs <- STATE$raw_flowset
  if (is.null(cs)) stop("Load FCS data first.")
  sn <- flowCore::sampleNames(cs)
  s  <- args$sample %||% sn[1]
  mat <- tryCatch(flowCore::spillover(cs[[s]])$SPILL, error = function(e) NULL)
  if (is.null(mat)) mat <- tryCatch(flowCore::keyword(cs[[s]], "$SPILLOVER")[[1]],
                                    error = function(e) NULL)
  if (is.null(mat)) stop(sprintf("No embedded spillover matrix in '%s'.", s))
  STATE$spillover_matrix <- mat
  matrix_payload(mat)
}

# Compute a spillover matrix from single-colour controls + an unstained control.
# Classic deterministic peak-ratio method (headless, no LAPACK/ggplot):
#   * each stained control is auto-assigned to the channel it is brightest in
#     (relative to the unstained background) — robust, no filename parsing;
#   * its spillover row = background-subtracted median of the positive population
#     in every channel, normalised so the primary channel = 1.
# This is the WF1 path. A future autospill integration can add iterative
# regression refinement (Roca et al.) on top of this initialisation.
COMMANDS$compute_spillover <- function(id, args) {
  .require_pkg("flowCore")
  cs <- STATE$raw_flowset
  if (is.null(cs)) stop("Load single-colour control files first.")
  sn <- flowCore::sampleNames(cs)
  if (length(sn) < 2) stop("Need an unstained control plus single-colour controls.")

  uns <- args$unstained %||% sn[grepl("unstain|^uns|autof", sn, ignore.case = TRUE)][1]
  if (is.na(uns) || !nzchar(uns)) stop("No unstained control found — specify one.")
  stained <- setdiff(sn, uns)

  chans <- flowCore::colnames(cs)
  fluor <- chans[!grepl("FSC|SSC|Time|^TIME$|Event|Width", chans, ignore.case = TRUE)]
  if (length(fluor) == 0) stop("No fluorescence channels detected.")

  send_progress(id, 0.2, "Measuring unstained background…")
  med_of <- function(s, sub = NULL) {
    e <- flowCore::exprs(cs[[s]])[, fluor, drop = FALSE]
    if (!is.null(sub)) e <- e[sub, , drop = FALSE]
    apply(e, 2, stats::median)
  }
  bg <- med_of(uns)

  M <- diag(length(fluor)); dimnames(M) <- list(fluor, fluor)
  used <- character(0)
  for (i in seq_along(stained)) {
    s <- stained[i]
    send_progress(id, 0.2 + 0.7 * i / length(stained), sprintf("Control: %s", s))
    e <- flowCore::exprs(cs[[s]])[, fluor, drop = FALSE]
    # primary channel = brightest above background; skip channels already taken
    cand <- apply(e, 2, stats::median) - bg
    cand[fluor %in% used] <- -Inf
    P <- fluor[which.max(cand)]
    used <- c(used, P)
    # positive population = top quartile in the primary channel
    thr <- stats::quantile(e[, P], 0.75, na.rm = TRUE)
    pos <- e[e[, P] >= thr, , drop = FALSE]
    row <- apply(pos, 2, stats::median) - bg
    denom <- row[[P]]
    if (!is.finite(denom) || denom <= 0) next      # leave identity row
    row <- row / denom
    row[!is.finite(row) | row < 0] <- 0
    M[P, ] <- row
  }
  diag(M) <- 1
  STATE$spillover_matrix <- M
  c(matrix_payload(M), list(unstained = uns))
}

# Store a user-provided / edited matrix.
COMMANDS$set_spillover <- function(id, args) {
  mat <- parse_matrix(args)
  STATE$spillover_matrix <- mat
  list(ok = TRUE, channels = colnames(mat))
}

# Apply the current (or supplied) spillover matrix to the data.
COMMANDS$apply_compensation <- function(id, args) {
  .require_pkg("flowCore")
  cs <- STATE$raw_flowset
  if (is.null(cs)) stop("Load FCS data first.")
  mat <- if (!is.null(args$matrix)) parse_matrix(args) else STATE$spillover_matrix
  if (is.null(mat)) stop("No spillover matrix — compute or extract one first.")
  send_progress(id, 0.4, "Applying compensation…")
  STATE$comp_flowset <- flowCore::compensate(cs, mat)
  STATE$spillover_matrix <- mat
  # downstream products are now stale
  STATE$trans_flowset <- NULL
  STATE$gating_set <- NULL
  list(applied = TRUE, channels = colnames(mat))
}
