# StreamFLOW - install_packages.R
# Installs all R dependencies into R Portable.
# Run via: Rscript scripts/install_packages.R

log_file <- tryCatch(
  file.path(dirname(normalizePath(sys.frame(1)$ofile)), "install_log.txt"),
  error = function(e) "install_log.txt"
)
log_file <- normalizePath(log_file, mustWork = FALSE)

CRAN_REPO <- "https://cran.r-project.org"

log_msg <- function(msg, type = "INFO") {
  ts  <- format(Sys.time(), "%Y-%m-%d %H:%M:%S")
  txt <- sprintf("[%s] [%s] %s", ts, type, msg)
  cat(txt, "\n")
  cat(txt, "\n", file = log_file, append = TRUE)
}

log_msg("StreamFLOW Package Installer starting")
log_msg(paste("R version:", R.Version()$version.string))
log_msg(paste("Library path:", paste(.libPaths(), collapse = "; ")))
log_msg(paste("Log file:", log_file))

# ── Helper ─────────────────────────────────────────────────────────────────
safe_install <- function(expr, pkg_name) {
  log_msg(paste("Installing:", pkg_name))
  tryCatch({
    expr
    log_msg(paste("OK:", pkg_name))
    TRUE
  }, error = function(e) {
    log_msg(paste("FAILED:", pkg_name, "-", conditionMessage(e)), "ERROR")
    FALSE
  }, warning = function(w) {
    log_msg(paste("WARN:", pkg_name, "-", conditionMessage(w)), "WARN")
    TRUE
  })
}

failures <- character()

# ── Step 1: BiocManager ────────────────────────────────────────────────────
ok <- safe_install(
  install.packages("BiocManager",
                   repos  = CRAN_REPO,
                   ask    = FALSE,
                   update = FALSE,
                   quiet  = FALSE),
  "BiocManager"
)
if (!ok) failures <- c(failures, "BiocManager")

# ── Step 2: Bioconductor core packages ────────────────────────────────────
bioc_pkgs <- c("cytolib", "flowCore", "flowWorkspace",
               "flowWorkspaceData", "openCyto", "FlowSOM")
ok <- safe_install(
  BiocManager::install(
    bioc_pkgs,
    ask    = FALSE,
    update = FALSE,
    quiet  = FALSE
  ),
  paste(bioc_pkgs, collapse = ", ")
)
if (!ok) failures <- c(failures, bioc_pkgs)

# ── Step 3: devtools ──────────────────────────────────────────────────────
ok <- safe_install(
  install.packages("devtools",
                   repos  = CRAN_REPO,
                   ask    = FALSE,
                   update = FALSE),
  "devtools"
)
if (!ok) failures <- c(failures, "devtools")

# ── Step 4: DillonHammill/openCyto (fork) ────────────────────────────────
ok <- safe_install(
  devtools::install_github("DillonHammill/openCyto", force = TRUE),
  "DillonHammill/openCyto"
)
if (!ok) failures <- c(failures, "DillonHammill/openCyto")

# ── Step 5: DataEditR ─────────────────────────────────────────────────────
ok <- safe_install(
  devtools::install_github("DillonHammill/DataEditR"),
  "DillonHammill/DataEditR"
)
if (!ok) failures <- c(failures, "DataEditR")

# ── Step 6: HeatmapR ─────────────────────────────────────────────────────
ok <- safe_install(
  devtools::install_github("DillonHammill/HeatmapR"),
  "DillonHammill/HeatmapR"
)
if (!ok) failures <- c(failures, "HeatmapR")

# ── Step 7: CytoExploreRData ──────────────────────────────────────────────
ok <- safe_install(
  devtools::install_github("DillonHammill/CytoExploreRData"),
  "DillonHammill/CytoExploreRData"
)
if (!ok) failures <- c(failures, "CytoExploreRData")

# ── Step 8: CytoExploreR ─────────────────────────────────────────────────
ok <- safe_install(
  devtools::install_github("DillonHammill/CytoExploreR"),
  "DillonHammill/CytoExploreR"
)
if (!ok) failures <- c(failures, "CytoExploreR")

# ── Step 9: CRAN packages ─────────────────────────────────────────────────
cran_pkgs <- c(
  "shiny", "shinydashboard", "shinyFiles", "shinyjs", "shinycssloaders",
  "DT", "plotly", "ggplot2", "dplyr", "tidyr", "colourpicker",
  "rhandsontable", "openxlsx", "umap", "Rtsne", "MASS", "fields",
  "zip", "jsonlite"
)

ok <- safe_install(
  install.packages(
    cran_pkgs,
    repos  = CRAN_REPO,
    ask    = FALSE,
    update = FALSE
  ),
  paste(cran_pkgs, collapse = ", ")
)
if (!ok) failures <- c(failures, cran_pkgs)

# ── Summary ───────────────────────────────────────────────────────────────
cat("\n")
if (length(failures) == 0) {
  log_msg("=== ALL PACKAGES INSTALLED SUCCESSFULLY ===")
  cat("  StreamFLOW is ready to launch!\n\n")
} else {
  log_msg(paste("=== COMPLETED WITH FAILURES ==="), "WARN")
  log_msg(paste("Failed packages:", paste(failures, collapse = ", ")), "ERROR")
  cat(sprintf("\n  %d package(s) failed. Check install_log.txt for details.\n\n",
              length(failures)))
}

log_msg(paste("Log saved to:", log_file))
