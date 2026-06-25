# StreamFLOW engine — Visualization commands (Phase 3)
# Renders a density scatter (2D) or histogram (1D) for a sample to a PNG and
# returns the data-coordinate extent + the plot-region pixel box. The coord
# metadata is what the Phase 4 gating canvas will use to map pixels <-> data, so
# the same render pipeline is reused there.

# Pick the most-processed available data: transformed > compensated > raw.
.working_data <- function() {
  STATE$trans_flowset %||% STATE$comp_flowset %||% STATE$raw_flowset
}

COMMANDS$render_plot <- function(id, args) {
  .require_pkg("flowCore")
  fs <- .working_data()
  if (is.null(fs)) stop("Load data first.")
  sn <- flowCore::sampleNames(fs)
  s  <- args$sample %||% sn[1]
  if (!s %in% sn) stop(sprintf("Unknown sample: %s", s))

  xch  <- args$x
  ych  <- args$y
  if (is.null(xch) || !nzchar(xch)) stop("An X channel is required.")
  type <- args$type %||% (if (is.null(ych) || !nzchar(ych)) "histogram" else "pseudocolor")
  # 2D types require a Y channel; fall back to histogram if missing.
  if (type != "histogram" && (is.null(ych) || !nzchar(ych))) type <- "histogram"

  scale  <- as.numeric(args$scale  %||% 1)         # JavaFX DPI scale (high-DPI)
  width  <- as.integer(args$width  %||% 800)
  height <- as.integer(args$height %||% 600)
  pw <- as.integer(width * scale); ph <- as.integer(height * scale)

  e <- flowCore::exprs(fs[[s]])
  if (!xch %in% colnames(e)) stop(sprintf("Channel not found: %s", xch))
  # Nested gating: restrict to a parent population's events so the user gates
  # within it (the GatingSet is built on this same working data, so indices align).
  parent <- args$parent
  if (!is.null(parent) && nzchar(parent) && parent != "root" && !is.null(STATE$gating_set)) {
    ind <- tryCatch(flowWorkspace::gh_pop_get_indices(STATE$gating_set[[s]], parent),
                    error = function(e2) NULL)
    if (!is.null(ind)) e <- e[ind, , drop = FALSE]
  }
  n <- nrow(e)
  if (n == 0) stop("No events to plot (empty parent population?).")
  idx <- if (n > 20000) sample.int(n, 20000) else seq_len(n)

  png_path <- tempfile("sfplot_", fileext = ".png")
  # White plot background with contrasting data colours (cytometry standard).
  grDevices::png(png_path, width = pw, height = ph, res = 72 * scale, bg = "white")
  on.exit(grDevices::dev.off(), add = TRUE)
  graphics::par(bg = "white", fg = "black", col.axis = "black",
                col.lab = "black", col.main = "black",
                mar = c(4.2, 4.2, 2, 1))

  # FlowJo-style density ramp: low = blue, high = red (white background = empty).
  ramp <- grDevices::colorRampPalette(
    c("#0000CC", "#00CCFF", "#00CC44", "#FFFF00", "#FF9900", "#FF0000"))
  smooth   <- !identical(args$smooth, FALSE)   # default TRUE
  outliers <- isTRUE(args$outliers)

  if (type == "histogram") {
    d <- stats::density(e[idx, xch])
    graphics::plot(d, main = "", xlab = xch, ylab = "Density", col = "#08519C", lwd = 2)
    graphics::polygon(d, col = "#3182BD55", border = NA)
  } else {
    if (is.null(ych) || !ych %in% colnames(e)) stop(sprintf("Channel not found: %s", ych))
    x <- e[idx, xch]; y <- e[idx, ych]
    switch(type,
      dot = graphics::plot(x, y, pch = ".", col = "#08306BAA", xlab = xch, ylab = ych),
      pseudocolor = if (smooth) {
        graphics::smoothScatter(x, y, nrpoints = 0, xlab = xch, ylab = ych, colramp = ramp)
      } else {
        graphics::plot(x, y, col = grDevices::densCols(x, y, colramp = ramp),
                       pch = 20, cex = 0.3, xlab = xch, ylab = ych)
      },
      contour = {
        k <- MASS::kde2d(x, y, n = 80)
        graphics::plot(x, y, type = "n", xlab = xch, ylab = ych)
        if (outliers) graphics::points(x, y, pch = ".", col = "#99999955")
        graphics::contour(k, add = TRUE, col = "#08519C", drawlabels = FALSE)
      },
      density = {
        k <- MASS::kde2d(x, y, n = 120)
        graphics::image(k, col = ramp(64), xlab = xch, ylab = ych)
      },
      zebra = {
        k <- MASS::kde2d(x, y, n = 90)
        graphics::image(k, col = ramp(48), xlab = xch, ylab = ych)
        graphics::contour(k, add = TRUE, col = "#333333", drawlabels = FALSE, nlevels = 12)
      },
      # default: smoothed pseudocolor
      graphics::smoothScatter(x, y, nrpoints = 0, xlab = xch, ylab = ych, colramp = ramp)
    )
  }

  # Capture data ranges + device pixel box of the plot region (before dev.off).
  usr  <- graphics::par("usr")   # x1, x2, y1, y2 in data coordinates
  left   <- graphics::grconvertX(usr[1], "user", "device")
  right  <- graphics::grconvertX(usr[2], "user", "device")
  top    <- graphics::grconvertY(usr[4], "user", "device")
  bottom <- graphics::grconvertY(usr[3], "user", "device")

  list(
    png        = png_path,
    sample     = s,
    x          = xch,
    y          = ych,
    type       = type,
    width      = pw,
    height     = ph,
    xrange     = c(usr[1], usr[2]),
    yrange     = c(usr[3], usr[4]),
    plotbox_px = c(left, top, right, bottom)
  )
}

# List channels available for plotting on the current data.
COMMANDS$list_channels <- function(id, args) {
  fs <- .working_data()
  if (is.null(fs)) stop("Load data first.")
  list(channels = flowCore::colnames(fs),
       samples  = flowCore::sampleNames(fs))
}
