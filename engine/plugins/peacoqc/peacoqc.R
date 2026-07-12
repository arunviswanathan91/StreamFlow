# PeacoQC quality-control plugin for StreamFLOW.
#
# Invoked by engine/run_plugin.R as:  streamflow_run(request)
#
# Verified against the vendored package (NAMESPACE exports: PeacoQC, PeacoQCHeatmap, PlotPeacoQC,
# RemoveDoublets, RemoveMargins). The main entry is:
#
#   PeacoQC(ff, channels, ...)  ->  list(GoodCells = <logical vector>, FinalFF = <flowFrame>, ...)
#
# It flags events in unstable regions of the acquisition (flow-rate / signal-stability anomalies)
# using an IsolationTree + MAD-based peak detection over time bins.
#
# request:
#   fcs_path : absolute path to the sample's FCS file
#   channels : character vector of detectors to monitor (default: all fluorescence + scatter)
#   mad      : MAD threshold (PeacoQC default 6; lower = stricter)
#   it_limit : IsolationTree limit (PeacoQC default 0.6)
#
# return:
#   n_events, n_good, n_removed, pct_removed
#   kept_indices : 0-BASED row indices of the events PeacoQC kept. Returned as indices (not events)
#                  so the app applies them to its already-cached EventData with no re-transfer —
#                  the same contract the Python `subsample` command uses.
#
# NOTE: plotting / FCS writing / report generation are all disabled — this runs headless and returns
# data only. PeacoQC would otherwise write files into output_directory.

streamflow_run <- function(request) {
  fcs_path <- request$fcs_path
  if (is.null(fcs_path) || !file.exists(fcs_path)) stop("fcs_path not found: ", fcs_path)

  suppressWarnings(suppressMessages({
    for (p in c("flowCore", "PeacoQC")) {
      if (!requireNamespace(p, quietly = TRUE)) stop(p, " is not installed")
    }
  }))

  mad_val <- if (is.null(request$mad)) 6 else as.numeric(request$mad)
  it_limit <- if (is.null(request$it_limit)) 0.6 else as.numeric(request$it_limit)

  ff <- flowCore::read.FCS(fcs_path, transformation = FALSE, truncate_max_range = FALSE)
  cn <- flowCore::colnames(ff)

  # Default: monitor every channel except Time (PeacoQC uses Time as the binning axis).
  req_ch <- request$channels
  if (is.null(req_ch) || length(req_ch) == 0) {
    channels <- setdiff(cn, "Time")
  } else {
    channels <- intersect(unlist(req_ch), cn)
  }
  if (length(channels) == 0) stop("no valid channels to monitor")

  res <- PeacoQC::PeacoQC(
    ff = ff, channels = channels,
    MAD = mad_val, IT_limit = it_limit,
    plot = FALSE, save_fcs = FALSE, report = FALSE,
    output_directory = tempdir())

  good <- res$GoodCells
  if (is.null(good)) stop("PeacoQC returned no GoodCells vector")
  good <- as.logical(good)

  n_events <- length(good)
  n_good <- sum(good, na.rm = TRUE)
  kept0 <- which(good) - 1L   # R is 1-based; the app expects 0-based row indices

  list(
    n_events = n_events,
    n_good = n_good,
    n_removed = n_events - n_good,
    pct_removed = if (n_events > 0) round(100 * (n_events - n_good) / n_events, 2) else 0,
    channels = channels,
    kept_indices = as.integer(kept0)
  )
}
