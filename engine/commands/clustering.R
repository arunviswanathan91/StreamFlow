# StreamFLOW engine — Clustering (Phase 4)
# FlowSOM (self-organising map + metaclustering) or PhenoGraph (graph-based) on a
# sample's fluorescence events, returning cluster sizes + a median-expression
# heatmap (cluster × channel). Works on an in-memory matrix copy.

COMMANDS$run_clustering <- function(id, args) {
  .require_pkg("flowCore")
  fs <- .working_data()
  if (is.null(fs)) stop("Load data first.")
  method <- args$method %||% "flowsom"
  sample <- args$sample %||% flowCore::sampleNames(fs)[1]
  scale  <- as.numeric(args$scale %||% 1)
  k      <- as.integer(args$k %||% 10)
  n_evt  <- as.integer(args$n_events %||% 20000)

  allchans <- flowCore::colnames(fs)
  chans <- if (!is.null(args$channels) && length(args$channels) > 0)
    intersect(unlist(args$channels, use.names = FALSE), allchans)
  else allchans[!grepl("FSC|SSC|Time|^TIME$|Width|Event", allchans, ignore.case = TRUE)]
  if (length(chans) < 2) stop("Need at least two fluorescence channels.")

  send_progress(id, 0.1, "Extracting events…")
  e <- flowCore::exprs(fs[[sample]])[, chans, drop = FALSE]
  if (nrow(e) > n_evt) e <- e[sample.int(nrow(e), n_evt), , drop = FALSE]

  send_progress(id, 0.35, sprintf("Clustering (%s)…", method))
  labels <- switch(method,
    flowsom = {
      .require_pkg("FlowSOM")
      ff <- flowCore::flowFrame(e)
      fsom <- FlowSOM::FlowSOM(ff, colsToUse = colnames(e), nClus = k, seed = 42)
      as.integer(FlowSOM::GetMetaclusters(fsom))
    },
    phenograph = {
      .require_pkg("Rphenograph")
      rp <- Rphenograph::Rphenograph(e)
      as.integer(igraph::membership(rp[[2]]))
    },
    stop(sprintf("Unsupported method: %s (use flowsom or phenograph).", method))
  )

  send_progress(id, 0.8, "Summarising clusters…")
  cl  <- sort(unique(labels))
  med <- t(vapply(cl, function(c) apply(e[labels == c, , drop = FALSE], 2, stats::median),
                  numeric(ncol(e))))
  rownames(med) <- paste0("C", cl); colnames(med) <- colnames(e)

  pw <- as.integer(720 * scale); ph <- as.integer(560 * scale)
  png_path <- tempfile("sfclust_", fileext = ".png")
  grDevices::png(png_path, width = pw, height = ph, res = 72 * scale, bg = "#0D1B2A")
  on.exit(grDevices::dev.off(), add = TRUE)
  ramp <- grDevices::colorRampPalette(c("#0D1B2A", "#1B4965", "#00B4D8", "#FFD166"))
  graphics::par(bg = "#0D1B2A", col.axis = "#E0E0E0", col.main = "#00B4D8")
  stats::heatmap(med, Colv = NA, scale = "column", col = ramp(64),
                 margins = c(10, 6), main = sprintf("%s — %d clusters", toupper(method), length(cl)))

  clusters <- lapply(cl, function(c) list(cluster = c, count = sum(labels == c),
                                          percent = round(100 * sum(labels == c) / length(labels), 2)))
  list(png = png_path, method = method, k = length(cl), n = length(labels),
       channels = colnames(e), clusters = clusters, width = pw, height = ph)
}
