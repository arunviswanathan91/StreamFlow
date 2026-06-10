# StreamFLOW - mod_workspace.R
# Save / load a complete analysis session as a single .sfw bundle so an
# experiment can be archived and shared (FlowJo workspace equivalent).
#
# A .sfw file is a zip containing:
#   gatingset/          – flowWorkspace::save_gs() output (data + comp + transforms + gates)
#   gatingTemplate.csv  – the CytoExploreR gatingTemplate (if present)
#   meta.rds            – channels, markers, spillover, transformers, gate geometry,
#                         annotation, experiment name (everything in `shared`)

# ── Hidden trigger buttons (driven by the File menu via shinyjs) ───────────────
workspaceUI <- function(id) {
  ns <- NS(id)
  tags$div(
    style = "display:none;",
    shinySaveButton(ns("save_ws"), "Save Workspace", "Save StreamFLOW Workspace",
                    filename = "experiment", filetype = list(sfw = "sfw")),
    shinyFilesButton(ns("open_ws"), "Open Workspace", "Open StreamFLOW Workspace",
                     multiple = FALSE)
  )
}

# ── Serialization helpers ──────────────────────────────────────────────────────
save_workspace <- function(shared, file) {
  tmp <- file.path(tempdir(), paste0("sfw_save_", as.integer(Sys.time())))
  dir.create(tmp, recursive = TRUE, showWarnings = FALSE)
  on.exit(unlink(tmp, recursive = TRUE), add = TRUE)

  # Persist the canonical data container as a GatingSet (preserves comp/transform/gates)
  gs <- shared$gating_set
  if (is.null(gs)) {
    fs <- shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
    if (!is.null(fs))
      gs <- tryCatch(flowWorkspace::GatingSet(fs), error = function(e) NULL)
  }
  if (!is.null(gs))
    tryCatch(flowWorkspace::save_gs(gs, file.path(tmp, "gatingset")),
             error = function(e) message("[workspace] save_gs failed: ", conditionMessage(e)))

  if (!is.null(shared$gating_template) && file.exists(shared$gating_template))
    file.copy(shared$gating_template, file.path(tmp, "gatingTemplate.csv"))

  meta <- list(
    version          = "1.0",
    channels         = shared$channels,
    all_channels     = shared$all_channels,
    fluor_channels   = shared$fluor_channels,
    markers          = shared$markers,
    spillover_matrix = shared$spillover_matrix,
    transforms       = shared$transforms,
    gate_list        = shared$gate_list,
    annotation       = shared$annotation,
    experiment_name  = shared$experiment_name,
    n_samples        = shared$n_samples,
    fcs_folder       = shared$fcs_folder
  )
  saveRDS(meta, file.path(tmp, "meta.rds"))

  # Zip with the `zip` package (pure C, reliable on bundled R-Portable; base
  # utils::zip would need an external zip executable that may not be present).
  if (file.exists(file)) unlink(file)
  zip::zipr(zipfile = file, files = list.files(tmp), root = tmp)
  file.exists(file)
}

load_workspace <- function(shared, file) {
  tmp <- file.path(tempdir(), paste0("sfw_load_", as.integer(Sys.time())))
  dir.create(tmp, recursive = TRUE, showWarnings = FALSE)
  utils::unzip(file, exdir = tmp)

  meta_path <- file.path(tmp, "meta.rds")
  if (!file.exists(meta_path)) stop("Not a valid StreamFLOW workspace (meta.rds missing).")
  meta <- readRDS(meta_path)

  gs_dir <- file.path(tmp, "gatingset")
  if (dir.exists(gs_dir)) {
    gs <- tryCatch(flowWorkspace::load_gs(gs_dir), error = function(e) {
      message("[workspace] load_gs failed: ", conditionMessage(e)); NULL })
    if (!is.null(gs)) {
      shared$gating_set  <- gs
      shared$raw_flowset <- tryCatch(flowWorkspace::gs_pop_get_data(gs),
                                     error = function(e) NULL)
    }
  }

  tmpl <- file.path(tmp, "gatingTemplate.csv")
  if (file.exists(tmpl)) {
    persist <- file.path(tempdir(), "streamflow_gatingTemplate.csv")
    file.copy(tmpl, persist, overwrite = TRUE)
    shared$gating_template <- persist
  }

  shared$channels         <- meta$channels
  shared$all_channels     <- meta$all_channels
  shared$fluor_channels   <- meta$fluor_channels
  shared$markers          <- meta$markers
  shared$spillover_matrix <- meta$spillover_matrix
  shared$transforms       <- meta$transforms %||% list()
  shared$gate_list        <- meta$gate_list %||% list()
  shared$annotation       <- meta$annotation
  shared$experiment_name  <- meta$experiment_name %||% "Untitled Experiment"
  shared$n_samples        <- meta$n_samples %||% 0L
  shared$fcs_folder       <- meta$fcs_folder
  invisible(TRUE)
}

# ── Server ─────────────────────────────────────────────────────────────────────
workspaceServer <- function(input, output, session, shared) {
  volumes <- resolve_volumes()
  shinyFileSave(input, "save_ws", roots = volumes, session = session,
                filetypes = c("sfw"))
  shinyFileChoose(input, "open_ws", roots = volumes, session = session,
                  filetypes = c("sfw"))

  observeEvent(input$save_ws, {
    req(is.list(input$save_ws))
    fi <- parseSavePath(volumes, input$save_ws)
    req(nrow(fi) > 0)
    path <- as.character(fi$datapath)
    shared$status <- "busy"
    withProgress(message = "Saving workspace…", value = 0.4, {
      ok <- tryCatch(save_workspace(shared, path), error = function(e) {
        showNotification(paste("Workspace save failed:", conditionMessage(e)),
                         type = "error", duration = 6); FALSE })
      if (isTRUE(ok))
        showNotification(paste("Workspace saved:", basename(path)),
                         type = "message", duration = 4)
    })
    shared$status <- "idle"
  })

  observeEvent(input$open_ws, {
    req(is.list(input$open_ws))
    fi <- parseFilePaths(volumes, input$open_ws)
    req(nrow(fi) > 0)
    path <- as.character(fi$datapath)
    shared$status <- "busy"
    withProgress(message = "Loading workspace…", value = 0.4, {
      ok <- tryCatch({ load_workspace(shared, path); TRUE }, error = function(e) {
        showNotification(paste("Workspace load failed:", conditionMessage(e)),
                         type = "error", duration = 6); FALSE })
      if (isTRUE(ok))
        showNotification(paste("Workspace loaded:", basename(path)),
                         type = "message", duration = 4)
    })
    shared$status <- "idle"
  })
}
