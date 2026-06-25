# StreamFLOW engine — Setup commands (Phase 1)
# FCS import + sample/channel summary. Ported from shiny/modules/mod_setup.R
# (the read.FCSheader validation + read.flowSet load), minus the Shiny reactivity.
# flowCore is loaded lazily so the engine still starts without it.

.require_pkg <- function(pkg) {
  if (!requireNamespace(pkg, quietly = TRUE))
    stop(sprintf("R package '%s' is not installed in the engine library.", pkg))
}

# Build the sample + channel summary sent to the Java UI. Works on either a
# flowCore flowSet or a flowWorkspace cytoset (both support sampleNames/[[ and
# parameters() on the per-sample frame).
summarize_flowset <- function(fs, skipped = 0L) {
  sn     <- flowCore::sampleNames(fs)
  counts <- tryCatch(
    vapply(sn, function(s) as.integer(nrow(fs[[s]])), integer(1)),
    error = function(e) rep(NA_integer_, length(sn)))

  pars   <- flowCore::pData(flowCore::parameters(fs[[sn[1]]]))
  chan   <- as.character(pars$name)
  marker <- as.character(pars$desc)
  marker[is.na(marker)] <- ""
  # Scatter/Time vs fluorescence — same heuristic the Shiny app used.
  is_scatter <- grepl("FSC|SSC|^Time$", chan, ignore.case = TRUE)

  STATE$channels      <- chan
  STATE$markers       <- marker
  STATE$fluor_channels <- chan[!is_scatter]

  samples <- lapply(seq_along(sn), function(i)
    list(name = sn[i], events = counts[i]))
  channels <- lapply(seq_along(chan), function(i)
    list(channel = chan[i], marker = marker[i], scatter = is_scatter[i]))

  list(
    experiment = STATE$experiment_name %||% "Untitled Experiment",
    n_samples  = length(sn),
    skipped    = skipped,
    folder     = STATE$fcs_folder %||% "",
    samples    = samples,
    channels   = channels
  )
}

COMMANDS$load_fcs <- function(id, args) {
  .require_pkg("flowCore")
  .require_pkg("flowWorkspace")
  folder    <- args$folder
  files     <- args$files
  recursive <- isTRUE(args$recursive)

  if (is.null(files) || length(files) == 0) {
    if (is.null(folder) || !nzchar(folder)) stop("No folder or files provided.")
    if (!dir.exists(folder)) stop(sprintf("Folder does not exist: %s", folder))
    files <- list.files(folder, pattern = "\\.fcs$", recursive = recursive,
                        full.names = TRUE, ignore.case = TRUE)
  }
  files <- unlist(files, use.names = FALSE)
  files <- files[file.exists(files)]
  if (length(files) == 0) stop("No .fcs files found.")

  send_progress(id, 0.10, sprintf("Validating %d FCS header(s)…", length(files)))
  valid <- Filter(function(f) {
    if (is_cancelled(id)) stop(cancelledCondition(id))
    tryCatch({ flowCore::read.FCSheader(f); TRUE }, error = function(e) {
      message(sprintf("[setup] skipping unreadable FCS: %s — %s",
                      basename(f), conditionMessage(e))); FALSE })
  }, files)
  if (length(valid) == 0) stop("No valid FCS files (unreadable headers).")

  # Panel-consistency pre-check: read.flowSet requires an identical channel set
  # across files. Detect mismatched panels up front and report a clear message
  # (instead of flowCore's cryptic "invalid class flowSet object" error).
  send_progress(id, 0.25, "Checking channel layouts…")
  sigs <- vapply(valid, function(f) {
    h  <- flowCore::read.FCSheader(f)[[1]]
    np <- as.integer(h[["$PAR"]])
    paste(vapply(seq_len(np), function(i) h[[paste0("$P", i, "N")]], character(1)),
          collapse = "|")
  }, character(1))
  if (length(unique(sigs)) > 1) {
    tab <- sort(table(sigs), decreasing = TRUE)
    stop(sprintf(paste0("These FCS files have %d different channel layouts and ",
      "cannot be combined into one experiment — load files that share the ",
      "same panel. (Largest matching group: %d of %d files.)"),
      length(unique(sigs)), tab[[1]], length(valid)))
  }

  # Load as a flowWorkspace cytoset (HDF5-backed) — the same loader CytoExploreR
  # uses (load_cytoset_from_fcs), and what GatingSet()/compensation/transform/
  # gating in later phases operate on directly. Avoids a flowSet→cytoset convert.
  send_progress(id, 0.40, sprintf("Reading %d file(s)…", length(valid)))
  fs <- flowWorkspace::load_cytoset_from_fcs(files = normalizePath(valid),
                                             transformation = FALSE)

  STATE$raw_flowset    <- fs
  STATE$fcs_folder     <- folder %||% dirname(valid[1])
  # invalidate any downstream products from a previous dataset
  STATE$comp_flowset   <- NULL
  STATE$trans_flowset  <- NULL
  STATE$gating_set     <- NULL

  send_progress(id, 0.85, "Summarizing…")
  summarize_flowset(fs, skipped = length(files) - length(valid))
}

# Current dataset summary (e.g. after a workspace load, or for a reconnecting UI).
COMMANDS$get_state <- function(id, args) {
  fs <- STATE$raw_flowset
  if (is.null(fs)) return(list(n_samples = 0L, samples = list(), channels = list()))
  summarize_flowset(fs, skipped = 0L)
}

COMMANDS$set_experiment_name <- function(id, args) {
  STATE$experiment_name <- as.character(args$name %||% "Untitled Experiment")
  list(experiment = STATE$experiment_name)
}

COMMANDS$reset <- function(id, args) {
  for (k in ls(STATE)) if (k != "status") rm(list = k, envir = STATE)
  list(ok = TRUE)
}

# Which compute packages are available — lets the UI prompt for install and lets
# integration tests skip gracefully / smoke-test the whole stack. Covers the
# Phase 1 essentials, the CytoExploreR pipeline, and the optional extras.
COMMANDS$capabilities <- function(id, args) {
  pkgs <- c(
    # Phase 1 essentials
    "flowCore", "flowWorkspace", "jsonlite", "zip",
    # CytoExploreR pipeline (Phase 2-4 core)
    "CytoExploreR", "openCyto", "FlowSOM", "EmbedSOM", "superheat",
    "robustbase", "rhandsontable", "DataEditR", "HeatmapR", "CytoExploreRData",
    # optional extras (added capabilities)
    "flowAI", "flowStats", "ggcyto", "CytoNorm", "autospill", "Rphenograph"
  )
  flags <- stats::setNames(as.list(vapply(pkgs, function(p)
    requireNamespace(p, quietly = TRUE), logical(1))), pkgs)
  list(packages = flags)   # nested so they don't collide with protocol type/id
}
