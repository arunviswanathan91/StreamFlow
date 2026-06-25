# StreamFLOW engine — Workspace commands (Phase 1)
# Save / load a complete session as a .sfw bundle. Ported almost verbatim from
# the pure helpers in shiny/modules/mod_workspace.R (shared -> STATE), keeping
# the zip-slip guard and the disk-backed-GatingSet temp-dir contract.
#
# A .sfw is a zip of:
#   gatingset/          flowWorkspace::save_gs() output (data + comp + transforms + gates)
#   gatingTemplate.csv  CytoExploreR gatingTemplate (if present)
#   meta.rds            channels/markers/spillover/transforms/gates/annotation/...

save_workspace <- function(state, file) {
  .require_pkg("flowWorkspace"); .require_pkg("zip")
  tmp <- file.path(tempdir(), paste0("sfw_save_", as.integer(Sys.time())))
  dir.create(tmp, recursive = TRUE, showWarnings = FALSE)
  on.exit(unlink(tmp, recursive = TRUE), add = TRUE)

  gs <- state$gating_set
  if (is.null(gs)) {
    fs <- state$trans_flowset %||% state$comp_flowset %||% state$raw_flowset
    if (!is.null(fs))
      gs <- tryCatch(flowWorkspace::GatingSet(fs), error = function(e) NULL)
  }
  if (!is.null(gs))
    tryCatch(flowWorkspace::save_gs(gs, file.path(tmp, "gatingset")),
             error = function(e) message("[workspace] save_gs failed: ", conditionMessage(e)))

  if (!is.null(state$gating_template) && file.exists(state$gating_template))
    file.copy(state$gating_template, file.path(tmp, "gatingTemplate.csv"))

  meta <- list(
    version          = "1.0",
    channels         = state$channels,
    all_channels     = state$all_channels,
    fluor_channels   = state$fluor_channels,
    markers          = state$markers,
    spillover_matrix = state$spillover_matrix,
    transforms       = state$transforms,
    gate_list        = state$gate_list,
    annotation       = state$annotation,
    experiment_name  = state$experiment_name,
    n_samples        = state$n_samples,
    fcs_folder       = state$fcs_folder
  )
  saveRDS(meta, file.path(tmp, "meta.rds"))

  if (file.exists(file)) unlink(file)
  zip::zipr(zipfile = file, files = list.files(tmp), root = tmp)
  file.exists(file)
}

load_workspace <- function(state, file) {
  .require_pkg("flowWorkspace")
  tmp <- file.path(tempdir(), paste0("sfw_load_", as.integer(Sys.time())))
  dir.create(tmp, recursive = TRUE, showWarnings = FALSE)
  # Keep the temp dir on success (load_gs returns a disk-backed GatingSet whose
  # HDF5 files live under it); remove it only on a failed load.
  loaded_ok <- FALSE
  on.exit(if (!loaded_ok) unlink(tmp, recursive = TRUE), add = TRUE)

  # Zip-slip guard: reject "../" or absolute paths in archive entries.
  entries <- utils::unzip(file, list = TRUE)$Name
  unsafe  <- entries[
    grepl("(^|[\\\\/])\\.\\.([\\\\/]|$)", entries) |
    grepl("^([A-Za-z]:|[\\\\/])", entries)
  ]
  if (length(unsafe) > 0)
    stop("Refusing to open workspace — archive contains unsafe paths: ",
         paste(unsafe, collapse = ", "))

  utils::unzip(file, exdir = tmp)

  meta_path <- file.path(tmp, "meta.rds")
  if (!file.exists(meta_path)) stop("Not a valid StreamFLOW workspace (meta.rds missing).")
  meta <- readRDS(meta_path)

  gs_dir <- file.path(tmp, "gatingset")
  if (dir.exists(gs_dir)) {
    gs <- tryCatch(flowWorkspace::load_gs(gs_dir), error = function(e) {
      message("[workspace] load_gs failed: ", conditionMessage(e)); NULL })
    if (!is.null(gs)) {
      state$gating_set  <- gs
      state$raw_flowset <- tryCatch(flowWorkspace::gs_pop_get_data(gs),
                                    error = function(e) NULL)
    }
  }

  tmpl <- file.path(tmp, "gatingTemplate.csv")
  if (file.exists(tmpl)) {
    persist <- file.path(tempdir(), "streamflow_gatingTemplate.csv")
    file.copy(tmpl, persist, overwrite = TRUE)
    state$gating_template <- persist
  }

  state$channels         <- meta$channels
  state$all_channels     <- meta$all_channels
  state$fluor_channels   <- meta$fluor_channels
  state$markers          <- meta$markers
  state$spillover_matrix <- meta$spillover_matrix
  state$transforms       <- meta$transforms %||% list()
  state$gate_list        <- meta$gate_list %||% list()
  state$annotation       <- meta$annotation
  state$experiment_name  <- meta$experiment_name %||% "Untitled Experiment"
  state$n_samples        <- meta$n_samples %||% 0L
  state$fcs_folder       <- meta$fcs_folder

  loaded_ok <- TRUE
  invisible(TRUE)
}

# ── Commands ─────────────────────────────────────────────────────────────────
COMMANDS$save_workspace <- function(id, args) {
  file <- args$file
  if (is.null(file) || !nzchar(file)) stop("No save path provided.")
  if (!dir.exists(dirname(file))) stop("Save location is not a valid folder.")
  if (is.null(STATE$raw_flowset) && is.null(STATE$gating_set))
    stop("Nothing to save — load FCS data first.")
  send_progress(id, 0.4, "Saving workspace…")
  ok <- save_workspace(STATE, file)
  list(saved = isTRUE(ok), file = file)
}

COMMANDS$load_workspace <- function(id, args) {
  file <- args$file
  if (is.null(file) || !nzchar(file)) stop("No workspace path provided.")
  if (!file.exists(file)) stop(sprintf("Workspace file not found: %s", file))
  send_progress(id, 0.4, "Loading workspace…")
  load_workspace(STATE, file)
  # Return the refreshed summary so the UI can repopulate.
  if (!is.null(STATE$raw_flowset)) summarize_flowset(STATE$raw_flowset, 0L)
  else list(n_samples = STATE$n_samples %||% 0L, samples = list(), channels = list())
}
