# StreamFLOW - server.R

server <- function(input, output, session) {

  # ── Pop-out window branch ────────────────────────────────────────────────
  # Electron opens focused single-sample windows via ?view=popout. These run as
  # separate Shiny sessions in the same R process and read shared app_state.
  qs <- shiny::parseQueryString(session$request$QUERY_STRING %||% "")
  if (identical(qs$view, "popout")) {
    popout_server(input, output, session, qs)
    return(invisible(NULL))
  }

  # ── Shared reactive state passed to all modules ──────────────────────────
  shared <- reactiveValues(
    fcs_folder       = NULL,
    raw_flowset      = NULL,   # original loaded flowSet (ncdfFlowSet from cyto_load)
    comp_flowset     = NULL,   # after cyto_compensate()
    trans_flowset    = NULL,   # after cyto_transform()
    gating_set       = NULL,   # GatingSet — central CytoExploreR object
    annotation       = NULL,   # data.frame: experiment details (cyto_details)
    channels         = NULL,   # character: user-selected analysis channels
    all_channels     = NULL,   # character: all raw channel names (immutable after load)
    fluor_channels   = NULL,   # character: fluorescent channels only (cyto_fluor_channels)
    markers          = NULL,   # character: marker names (cyto_markers)
    spillover_matrix = NULL,   # matrix: current spillover (cyto_spillover_extract)
    transforms       = list(), # cyto_transformer_* objects per channel
    gate_list        = list(), # named list of gate objects
    gating_template  = NULL,   # path to gatingTemplate CSV
    dim_result       = NULL,   # dimensionality reduction output
    stats_result     = NULL,   # statistics table (cyto_stats_compute result)
    experiment_name  = "Untitled Experiment",
    n_samples        = 0L,
    status           = "idle"  # "idle" | "busy" | "error"
  )

  # ── Keep cross-session app_state synced for pop-out windows ──────────────
  observe({
    app_state$flowset    <- shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
    app_state$gating_set <- shared$gating_set
    app_state$channels   <- shared$channels
  })

  # ── Module servers ────────────────────────────────────────────────────────
  callModule(setupServer,          "setup",          shared = shared)
  callModule(compensationServer,   "compensation",   shared = shared)
  callModule(transformationServer, "transformation", shared = shared)
  callModule(gatingServer,         "gating",         shared = shared)
  callModule(visualizationServer,  "visualization",  shared = shared)
  callModule(dimreduxServer,       "dimredux",       shared = shared)
  callModule(statisticsServer,     "statistics",     shared = shared)
  callModule(workspaceServer,      "workspace",      shared = shared)

  # ── Status bar outputs ────────────────────────────────────────────────────
  output$status_experiment <- renderUI({
    tags$span(
      style = "color: #00B4D8; font-weight: 600;",
      shared$experiment_name
    )
  })

  output$status_samples <- renderUI({
    n <- shared$n_samples
    color <- if (n > 0) "#2EC4B6" else "#5A7A8A"
    tags$span(style = paste0("color: ", color, "; font-weight: 600;"), n)
  })

  output$status_memory <- renderUI({
    mem_mb <- tryCatch({
      gc_info <- gc(verbose = FALSE)
      used_mb <- sum(gc_info[, "used"]) * 8 / 1024^2
      sprintf("%.0f MB", used_mb)
    }, error = function(e) "—")
    tags$span(style = "color: #8899AA;", mem_mb)
  })

  # Update status dot
  observe({
    status <- shared$status
    js_class <- switch(status,
      busy  = "status-dot",
      error = "status-dot",
      "status-dot idle"
    )
    js_color <- switch(status,
      busy  = "#F39C12",
      error = "#C0392B",
      "#2EC4B6"
    )
    shinyjs::runjs(sprintf(
      "document.getElementById('status-dot-main').className = '%s'; document.getElementById('status-dot-main').style.background = '%s';",
      js_class, js_color
    ))
  })

  # Auto-refresh memory every 30 seconds
  memory_timer <- reactiveTimer(30000)
  observe({
    memory_timer()
    output$status_memory  # trigger re-render
  })

  # ── Electron menu event handlers ─────────────────────────────────────────
  observeEvent(input$electron_menu_event, {
    evt <- input$electron_menu_event
    if (evt == "open_fcs_folder") {
      updateTabItems(session, "sidebar_menu", "setup")
      shinyjs::click("setup-load_fcs_btn")
    } else if (evt == "save_session") {
      shinyjs::click("workspace-save_ws")
    } else if (evt == "open_workspace") {
      shinyjs::click("workspace-open_ws")
    } else if (evt == "export_results") {
      updateTabItems(session, "sidebar_menu", "statistics")
      showNotification("Navigate to Statistics to export results.", type = "message", duration = 3)
    }
  })

  # ── Session cleanup ────────────────────────────────────────────────────────
  session$onSessionEnded(function() {
    message("[StreamFLOW] Session ended, cleaning up.")
    gc()
    stopApp()
  })
}
