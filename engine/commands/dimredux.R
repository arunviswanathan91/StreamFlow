# StreamFLOW engine — Dimensionality reduction (Phase 4)
# UMAP / t-SNE on a sample's fluorescence events, rendered to an embedding PNG.
# These are the long, formerly-UI-freezing jobs — they now run in the engine and
# the JavaFX UI stays responsive. Work on an in-memory matrix copy (never the
# disk-backed GatingSet) so a cancel/kill can't corrupt the workspace.

COMMANDS$run_dimredux <- function(id, args) {
  .require_pkg("flowCore")
  fs <- .working_data()
  if (is.null(fs)) stop("Load data first.")
  method <- args$method %||% "umap"
  sample <- args$sample %||% flowCore::sampleNames(fs)[1]
  scale  <- as.numeric(args$scale %||% 1)
  n_evt  <- as.integer(args$n_events %||% 5000)

  allchans <- flowCore::colnames(fs)
  chans <- if (!is.null(args$channels) && length(args$channels) > 0)
    intersect(unlist(args$channels, use.names = FALSE), allchans)
  else allchans[!grepl("FSC|SSC|Time|^TIME$|Width|Event", allchans, ignore.case = TRUE)]
  if (length(chans) < 2) stop("Need at least two fluorescence channels.")

  send_progress(id, 0.1, "Extracting events…")
  e <- flowCore::exprs(fs[[sample]])[, chans, drop = FALSE]
  if (nrow(e) > n_evt) e <- e[sample.int(nrow(e), n_evt), , drop = FALSE]

  send_progress(id, 0.35, sprintf("Running %s on %d events…", toupper(method), nrow(e)))
  emb <- switch(method,
    umap = { .require_pkg("umap"); umap::umap(e)$layout },
    tsne = { .require_pkg("Rtsne"); Rtsne::Rtsne(e, check_duplicates = FALSE,
                                                 perplexity = min(30, floor((nrow(e) - 1) / 3)))$Y },
    stop(sprintf("Unsupported method: %s (use umap or tsne).", method))
  )
  STATE$embedding <- list(method = method, coords = emb, sample = sample, channels = chans)

  send_progress(id, 0.85, "Rendering embedding…")
  pw <- as.integer(700 * scale); ph <- as.integer(560 * scale)
  png_path <- tempfile("sfmap_", fileext = ".png")
  grDevices::png(png_path, width = pw, height = ph, res = 72 * scale, bg = "#0D1B2A")
  on.exit(grDevices::dev.off(), add = TRUE)
  graphics::par(bg = "#0D1B2A", fg = "#B0C4D8", col.axis = "#B0C4D8",
                col.lab = "#E0E0E0", col.main = "#00B4D8", mar = c(4, 4, 2, 1))
  ramp <- grDevices::colorRampPalette(c("#0D1B2A", "#1B4965", "#00B4D8", "#FFD166"))
  lab <- toupper(method)
  graphics::smoothScatter(emb[, 1], emb[, 2], nrpoints = 0,
                          xlab = paste0(lab, "_1"), ylab = paste0(lab, "_2"), colramp = ramp)

  list(png = png_path, method = method, sample = sample, n = nrow(e),
       channels = chans, width = pw, height = ph)
}
