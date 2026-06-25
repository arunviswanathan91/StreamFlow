# StreamFLOW engine — Gating commands (Phase 4)
# Interactive gating round-trips drawn shapes (in DATA coordinates) into flowCore
# gates on a flowWorkspace GatingSet. The GatingSet is built on the most-processed
# data (transformed > compensated > raw) so gate coords match what the Graph
# Window renders. Gates apply to all samples (one gating tree); counts are
# reported for the sample being gated.

# Build the GatingSet lazily from the working data.
.ensure_gs <- function() {
  if (is.null(STATE$gating_set)) {
    fs <- .working_data()
    if (is.null(fs)) stop("Load data first.")
    STATE$gating_set <- flowWorkspace::GatingSet(fs)
    STATE$gate_list <- STATE$gate_list %||% list()
  }
  STATE$gating_set
}

.num <- function(v) as.numeric(unlist(v, use.names = FALSE))

# Build a flowCore gate from a drawn shape (coords in data space).
.build_gate <- function(type, name, xch, ych, coords) {
  switch(type,
    rectangle = {
      x <- .num(coords$x); y <- .num(coords$y)
      flowCore::rectangleGate(filterId = name,
        .gate = stats::setNames(list(range(x), range(y)), c(xch, ych)))
    },
    interval = {
      x <- .num(coords$x)
      flowCore::rectangleGate(filterId = name,
        .gate = stats::setNames(list(range(x)), xch))
    },
    polygon = {
      b <- cbind(.num(coords$x), .num(coords$y)); colnames(b) <- c(xch, ych)
      flowCore::polygonGate(filterId = name, boundaries = b)
    },
    ellipse = {
      x <- .num(coords$x); y <- .num(coords$y)
      cx <- mean(range(x)); cy <- mean(range(y))
      a  <- diff(range(x)) / 2; b <- diff(range(y)) / 2
      cov <- matrix(c(a^2, 0, 0, b^2), 2, 2, dimnames = list(c(xch, ych), c(xch, ych)))
      flowCore::ellipsoidGate(filterId = name,
        .gate = cov, mean = stats::setNames(c(cx, cy), c(xch, ych)), distance = 1)
    },
    stop(sprintf("Unsupported gate type: %s", type))
  )
}

.count <- function(gh, node) tryCatch(flowWorkspace::gh_pop_get_count(gh, node),
                                      error = function(e) NA_integer_)

COMMANDS$add_gate <- function(id, args) {
  .require_pkg("flowWorkspace"); .require_pkg("flowCore")
  gs <- .ensure_gs()
  name   <- args$name; if (is.null(name) || !nzchar(name)) stop("Gate needs a name.")
  parent <- args$parent %||% "root"
  type   <- args$type
  xch    <- args$x; ych <- args$y
  sample <- args$sample %||% flowCore::sampleNames(gs)[1]

  send_progress(id, 0.3, sprintf("Adding %s gate '%s'…", type, name))

  if (identical(type, "quadrant")) {
    x0 <- .num(coords_x <- args$coords$x)[1]; y0 <- .num(args$coords$y)[1]
    qg <- flowCore::quadGate(filterId = name,
            .gate = stats::setNames(c(x0, y0), c(xch, ych)))
    added <- flowWorkspace::gs_pop_add(gs, qg, parent = parent)
    flowWorkspace::recompute(gs)
    gh <- gs[[sample]]
    pops <- lapply(added, function(n) list(name = n, count = .count(gh, n)))
    STATE$gate_list[[name]] <- list(name = name, parent = parent, type = "quadrant",
                                    x = xch, y = ych, coords = list(x = x0, y = y0))
    return(list(quadrant = TRUE, populations = pops, x = xch, y = ych))
  }

  gate <- .build_gate(type, name, xch, ych, args$coords)
  flowWorkspace::gs_pop_add(gs, gate, parent = parent, name = name)
  send_progress(id, 0.7, "Recomputing…")
  flowWorkspace::recompute(gs)

  gh <- gs[[sample]]
  node <- if (parent == "root") paste0("/", name) else paste0(parent, "/", name)
  cnt  <- .count(gh, node)
  pcnt <- .count(gh, if (parent == "root") "root" else parent)
  STATE$gate_list[[name]] <- list(name = name, parent = parent, type = type,
                                  x = xch, y = ych, coords = args$coords)
  list(name = name, node = node, count = cnt, parent_count = pcnt,
       percent = if (is.finite(cnt) && is.finite(pcnt) && pcnt > 0) 100 * cnt / pcnt else NA)
}

# Gates whose axes match (x,y) — for drawing existing-gate overlays on a plot.
COMMANDS$list_gates <- function(id, args) {
  gs <- STATE$gating_set
  if (is.null(gs)) return(list(gates = list()))
  sample <- args$sample %||% flowCore::sampleNames(gs)[1]
  gh <- gs[[sample]]
  nodes <- setdiff(flowWorkspace::gs_get_pop_paths(gs, path = "auto"), "root")
  gates <- lapply(nodes, function(n) {
    key  <- basename(n)
    geom <- STATE$gate_list[[key]]
    list(name = n, key = key, count = .count(gh, n),
         type = geom$type, x = geom$x, y = geom$y,
         parent = geom$parent, coords = geom$coords)
  })
  list(gates = gates, sample = sample)
}

COMMANDS$remove_gate <- function(id, args) {
  gs <- STATE$gating_set
  if (is.null(gs)) stop("No gates to remove.")
  node <- args$node %||% args$name
  flowWorkspace::gs_pop_remove(gs, node)
  STATE$gate_list[[basename(node)]] <- NULL
  list(removed = node)
}

# Population stats for the gating tree (count, %parent, %total) for one sample.
COMMANDS$gate_tree_stats <- function(id, args) {
  gs <- STATE$gating_set
  if (is.null(gs)) return(list(stats = list()))
  sample <- args$sample %||% flowCore::sampleNames(gs)[1]
  gh <- gs[[sample]]
  total <- .count(gh, "root")
  nodes <- setdiff(flowWorkspace::gs_get_pop_paths(gs, path = "auto"), "root")
  st <- lapply(nodes, function(n) {
    cnt <- .count(gh, n)
    parent <- dirname(n); if (parent == "" || parent == ".") parent <- "root"
    pcnt <- .count(gh, parent)
    list(name = n, count = cnt,
         percent_parent = if (is.finite(pcnt) && pcnt > 0) 100 * cnt / pcnt else NA,
         percent_total  = if (is.finite(total) && total > 0) 100 * cnt / total else NA)
  })
  list(sample = sample, total = total, stats = st)
}
