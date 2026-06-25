# StreamFLOW engine — Transformation commands (Phase 2)
# Per-channel fluorescence transforms (logicle / arcsinh) applied to the
# compensated data. Uses flowCore primitives (estimateLogicle, arcsinhTransform,
# transform) — headless and stable.

.fluor_channels <- function(fs) {
  chans <- flowCore::colnames(fs)
  chans[!grepl("FSC|SSC|Time|^TIME$|Width|Event", chans, ignore.case = TRUE)]
}

COMMANDS$list_fluor_channels <- function(id, args) {
  fs <- STATE$comp_flowset %||% STATE$raw_flowset
  if (is.null(fs)) stop("Load data first.")
  list(channels = .fluor_channels(fs))
}

# Apply a transform to the (compensated, if available) data -> STATE$trans_flowset.
COMMANDS$apply_transformation <- function(id, args) {
  .require_pkg("flowCore")
  fs <- STATE$comp_flowset %||% STATE$raw_flowset
  if (is.null(fs)) stop("Load (and ideally compensate) data first.")
  method <- args$method %||% "logicle"

  chans <- if (!is.null(args$channels) && length(args$channels) > 0)
    unlist(args$channels, use.names = FALSE) else .fluor_channels(fs)
  chans <- intersect(chans, flowCore::colnames(fs))
  if (length(chans) == 0) stop("No valid fluorescence channels to transform.")

  send_progress(id, 0.3, sprintf("Estimating %s transform…", method))
  tl <- switch(method,
    logicle = flowCore::estimateLogicle(fs[[flowCore::sampleNames(fs)[1]]], channels = chans),
    arcsinh = {
      cf <- as.numeric(args$cofactor %||% 150)
      flowCore::transformList(chans, flowCore::arcsinhTransform(a = 0, b = 1 / cf, c = 0))
    },
    log = flowCore::transformList(chans, flowCore::logTransform()),
    stop(sprintf("Unsupported transform method: '%s' (use logicle, arcsinh, or log).", method))
  )

  send_progress(id, 0.6, "Applying transform…")
  STATE$trans_flowset <- flowCore::transform(fs, tl)
  STATE$transforms <- list(method = method, channels = chans,
                           cofactor = if (method == "arcsinh") as.numeric(args$cofactor %||% 150) else NULL)
  STATE$gating_set <- NULL   # gating must be redone on transformed data
  list(applied = TRUE, method = method, channels = chans)
}
