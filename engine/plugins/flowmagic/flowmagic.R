# FlowMagic auto-gating plugin for StreamFLOW.
#
# Invoked by engine/run_plugin.R as:  streamflow_run(request)
#
# IMPORTANT — how flowMagic actually works (verified against the vendored package's NAMESPACE and
# R/gating_wrapper.R, NOT assumed): flowMagic is a SUPERVISED, TEMPLATE-BASED auto-gater, not a
# one-shot "find gates in this 2-D plot" function. Its exported entry points are:
#
#   magic_manual_gating(gs_input, parent_node, new_child_node, samples_id, channel_x, channel_y, ...)
#   magic_template_train(gs_input, ...)     # learn a template from MANUALLY gated samples
#   magic_template_predict(gs_input, ...)   # apply that trained template to new samples
#
# All of them take a flowWorkspace GatingSet (`gs_input`), not a raw matrix. So this plugin exposes
# the PREDICT step: the user must first supply a trained template (produced by magic_template_train).
#
# request:
#   fcs_path      : absolute path to the sample's FCS file
#   template_path : path to a trained flowMagic template (.rds) produced by magic_template_train
#   parent_node   : gating-tree node the new gate hangs under (default "root")
#   x_channel     : detector name for the X axis
#   y_channel     : detector name for the Y axis
#
# return:
#   gates : list of { name, type = "polygon", x_channel, y_channel, xs = [...], ys = [...] }
#
# Vertices are returned in RAW DATA space, matching how StreamFLOW stores gates (CytoPlot.Gate.xs/ys),
# so the app can add them to the gating tree with no rescaling.

streamflow_run <- function(request) {
  fcs_path <- request$fcs_path
  template_path <- request$template_path
  x_ch <- request$x_channel
  y_ch <- request$y_channel
  parent_node <- if (is.null(request$parent_node)) "root" else request$parent_node

  if (is.null(fcs_path) || !file.exists(fcs_path)) stop("fcs_path not found: ", fcs_path)
  if (is.null(x_ch) || is.null(y_ch)) stop("x_channel and y_channel are required")
  if (is.null(template_path) || !file.exists(template_path)) {
    stop("flowMagic needs a trained template (.rds). Train one first with magic_template_train() ",
         "on manually gated samples, then pass its path as 'template_path'.")
  }

  suppressWarnings(suppressMessages({
    for (p in c("flowCore", "flowWorkspace", "flowMagic")) {
      if (!requireNamespace(p, quietly = TRUE)) stop(p, " is not installed")
    }
  }))

  # flowMagic operates on a GatingSet, so wrap the single FCS into one.
  ff <- flowCore::read.FCS(fcs_path, transformation = FALSE, truncate_max_range = FALSE)
  cn <- flowCore::colnames(ff)
  if (!(x_ch %in% cn)) stop("x_channel '", x_ch, "' not in FCS")
  if (!(y_ch %in% cn)) stop("y_channel '", y_ch, "' not in FCS")

  gs <- flowWorkspace::GatingSet(flowCore::flowSet(ff))
  template <- readRDS(template_path)

  pred <- flowMagic::magic_template_predict(
    gs_input = gs, template = template,
    channel_x = x_ch, channel_y = y_ch, parent_node = parent_node)

  # Convert the prediction into polygon gates using flowMagic's own exported converter.
  polys <- flowMagic::flowmagic_pred_to_poly_gates(pred)
  if (is.matrix(polys) || is.data.frame(polys)) polys <- list(polys)
  if (!is.list(polys)) stop("flowMagic returned an unexpected type: ", class(polys)[1])

  gates <- list()
  for (i in seq_along(polys)) {
    p <- as.matrix(polys[[i]])
    if (ncol(p) < 2 || nrow(p) < 3) next   # need a closed polygon
    gates[[length(gates) + 1L]] <- list(
      name = paste0("FlowMagic ", i),
      type = "polygon",
      x_channel = x_ch,
      y_channel = y_ch,
      xs = as.numeric(p[, 1]),
      ys = as.numeric(p[, 2])
    )
  }

  list(gates = gates, n_gates = length(gates))
}
