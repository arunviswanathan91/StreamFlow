# StreamFLOW engine — offline installer from engine/vendor/
# =============================================================================
# Installs the CytoExploreR analysis pipeline (Phase 2+) and optional extras
# from the locally-vendored package sources, with NO network dependency
# (the machine's Avast HTTPS scanning breaks CRAN/Bioc/GitHub downloads).
#
#   Rscript engine/install_from_vendor.R
#
# Idempotent: skips packages already installed. Installs in dependency order.
# Anything still missing is reported at the end with where to get it.
# =============================================================================

# Resolve engine/vendor relative to this script.
fa <- grep("^--file=", commandArgs(trailingOnly = FALSE), value = TRUE)
engine_dir <- if (length(fa)) dirname(normalizePath(sub("^--file=", "", fa[1]))) else getwd()
VENDOR <- file.path(engine_dir, "vendor")
# Resolve transitive deps from BOTH CRAN and Bioconductor (network now open via
# the Avast exclusion). Vendored packages build from local source; their missing
# deps (changepoint, fda, ks, robustbase deps, …) are pulled from these repos.
if (requireNamespace("BiocManager", quietly = TRUE)) {
  options(repos = BiocManager::repositories())
} else {
  options(repos = c(CRAN = "https://cloud.r-project.org"))
}

if (!requireNamespace("remotes", quietly = TRUE))
  stop("remotes is required (install.packages('remotes')) but appears missing.")

note <- function(...) cat(sprintf(...), "\n")
have <- function(p) isTRUE(requireNamespace(p, quietly = TRUE))

# Find a vendored source dir for a package (handles -master/-devel/-main suffixes).
vendor_dir <- function(pkg) {
  hits <- list.dirs(VENDOR, recursive = FALSE)
  base <- basename(hits)
  i <- which(tolower(base) == tolower(pkg) |
             grepl(paste0("^", tolower(pkg), "[-_](master|devel|main)$"), tolower(base)))
  if (length(i)) hits[i[1]] else NA_character_
}

install_vendor <- function(pkg, deps = FALSE) {
  if (have(pkg)) { note("skip  %s (already installed)", pkg); return(invisible(TRUE)) }
  d <- vendor_dir(pkg)
  if (is.na(d)) { note("MISS  %s (not in vendor/)", pkg); return(invisible(FALSE)) }
  note("build %s  <- %s", pkg, basename(d))
  ok <- tryCatch({
    remotes::install_local(d, dependencies = deps, upgrade = "never", quiet = FALSE)
    have(pkg)
  }, error = function(e) { note("FAIL  %s: %s", pkg, conditionMessage(e)); FALSE })
  invisible(ok)
}

# Dependency order: leaves first, CytoExploreR last. Compiled deps (cytolib,
# flowCore, flowWorkspace, openCyto, FlowSOM, Rcpp, Rtsne, FNN) are assumed
# already installed as Bioc/CRAN binaries — we do NOT rebuild them from the
# vendored *dev* sources (needs Rtools and risks breaking the working stack).
PIPE_DEPS <- c("robustbase", "EmbedSOM", "superheat",  # CytoExploreR imports (now in vendor)
               "rhandsontable", "flowAI",               # CytoExploreR imports (in vendor)
               "DataEditR", "HeatmapR", "CytoExploreRData")
for (p in PIPE_DEPS) install_vendor(p, deps = TRUE)

# Core: CytoExploreR (Phase 2-4 compensation/transform/gating/stats/plots).
install_vendor("CytoExploreR", deps = TRUE)

# Optional extras (new functionality) — installed only if present in vendor.
EXTRAS <- c(
  "flowStats",     # normalization / warping
  "ggcyto",        # ggplot2 cytometry plotting
  "CytoNorm",      # batch-effect normalization (NEW)
  "autospill",     # automated spillover / compensation (NEW)
  "Rphenograph"    # PhenoGraph clustering (NEW, alongside FlowSOM)
)
for (p in EXTRAS) install_vendor(p, deps = TRUE)

# ── Report ───────────────────────────────────────────────────────────────────
note("\n=== Install report ===")
report <- c("CytoExploreR", PIPE_DEPS, EXTRAS)
for (p in unique(report)) note("%-18s %s", p, if (have(p)) "OK" else "MISSING")
note("\nCytoExploreR loadable: %s",
     tryCatch({loadNamespace("CytoExploreR"); "YES"},
              error = function(e) paste("NO -", conditionMessage(e))))
