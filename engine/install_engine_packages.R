# StreamFLOW engine — R package installer (Java rewrite)
# Installs ONLY what the headless compute engine needs (no Shiny/DT/plotly UI
# packages). Correct ordering: CRAN deps (incl. cli/remotes for GitHub) FIRST,
# then Bioconductor core, then the DillonHammill GitHub forks. Excludes
# flowWorkspaceData (example data only; its annotation mirror 404s).
#
#   Rscript engine/install_engine_packages.R
#
# Re-runnable: skips packages already present.

CRAN <- "https://cloud.r-project.org"
options(repos = c(CRAN = CRAN))

ok <- TRUE
note <- function(...) cat(sprintf(...), "\n")

have <- function(p) requireNamespace(p, quietly = TRUE)

install_cran <- function(pkgs) {
  need <- pkgs[!vapply(pkgs, have, logical(1))]
  if (length(need) == 0) { note("CRAN: all present (%s)", paste(pkgs, collapse=", ")); return(invisible()) }
  note("CRAN: installing %s", paste(need, collapse = ", "))
  install.packages(need, repos = CRAN, quiet = FALSE)
}

# 1) CRAN — tools (cli/remotes/devtools) + compute libs the engine uses.
install_cran(c(
  "cli", "remotes", "BiocManager",
  "ggplot2", "dplyr", "tidyr",
  "umap", "Rtsne", "MASS", "fields",
  "openxlsx", "zip", "jsonlite"
))

# 2) Bioconductor core (no flowWorkspaceData — example data only, mirror 404s).
bioc <- c("cytolib", "flowCore", "flowWorkspace", "openCyto", "FlowSOM")
need_bioc <- bioc[!vapply(bioc, have, logical(1))]
if (length(need_bioc)) {
  note("Bioc: installing %s", paste(need_bioc, collapse = ", "))
  BiocManager::install(need_bioc, update = FALSE, ask = FALSE)
} else note("Bioc: all present")

# 3) GitHub forks (DillonHammill) — needed from Phase 2 on (CytoExploreR etc.).
#    remotes is now installed; run after Bioc so its deps resolve.
forks <- c(
  "DillonHammill/openCyto",
  "DillonHammill/DataEditR",
  "DillonHammill/HeatmapR",
  "DillonHammill/CytoExploreRData",
  "DillonHammill/CytoExploreR"
)
if (have("remotes")) {
  for (f in forks) {
    pkg <- sub(".*/", "", f)
    if (have(pkg)) { note("GitHub: %s present", pkg); next }
    note("GitHub: installing %s", f)
    tryCatch(remotes::install_github(f, upgrade = "never", quiet = FALSE),
             error = function(e) { note("GitHub FAILED %s: %s", f, conditionMessage(e)) })
  }
}

# 4) Report (honest — checks actual availability, unlike the old script).
note("\n=== Engine package status ===")
check <- c(bioc, "CytoExploreR", "remotes", "ggplot2", "umap", "Rtsne",
           "openxlsx", "zip", "jsonlite")
for (p in unique(check)) note("%-16s %s", p, if (have(p)) "OK" else "MISSING")
