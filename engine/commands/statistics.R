# StreamFLOW engine — Statistics commands (Phase 3c)
# Per-sample × per-population statistics: event count, % of total, and MFI
# (median fluorescence intensity) per fluorescence channel. Uses the GatingSet
# populations when gates exist, else whole-sample (root) stats.

COMMANDS$compute_stats <- function(id, args) {
  .require_pkg("flowWorkspace"); .require_pkg("flowCore")
  gs <- STATE$gating_set
  fs <- .working_data()
  if (is.null(gs) && is.null(fs)) stop("Load data first.")

  src      <- if (!is.null(gs)) gs else fs
  samples  <- flowCore::sampleNames(src)
  allchans <- flowCore::colnames(src)
  chans <- if (!is.null(args$channels) && length(args$channels) > 0)
    intersect(unlist(args$channels, use.names = FALSE), allchans)
  else allchans[!grepl("FSC|SSC|Time|^TIME$|Width|Event", allchans, ignore.case = TRUE)]

  nodes <- if (!is.null(gs)) flowWorkspace::gs_get_pop_paths(gs, path = "auto") else "root"
  if (!"root" %in% nodes) nodes <- c("root", nodes)

  rows <- list()
  for (s in samples) {
    if (!is.null(gs)) {
      gh    <- gs[[s]]
      total <- tryCatch(flowWorkspace::gh_pop_get_count(gh, "root"), error = function(e) NA_integer_)
    } else {
      total <- nrow(fs[[s]])
    }
    for (n in nodes) {
      e <- tryCatch({
        if (!is.null(gs)) flowCore::exprs(flowWorkspace::gh_pop_get_data(gs[[s]], n))
        else flowCore::exprs(fs[[s]])
      }, error = function(e) NULL)
      if (is.null(e)) next
      cnt <- nrow(e)
      mfi <- if (cnt > 0)
        stats::setNames(as.list(apply(e[, chans, drop = FALSE], 2, stats::median)), chans)
      else stats::setNames(as.list(rep(NA_real_, length(chans))), chans)
      rows[[length(rows) + 1]] <- list(
        sample        = s,
        population    = n,
        count         = cnt,
        percent_total = if (is.finite(total) && total > 0) round(100 * cnt / total, 3) else NA,
        mfi           = mfi
      )
    }
  }
  list(channels = chans, rows = rows)
}
