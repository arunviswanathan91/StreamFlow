# StreamFLOW - server.R

server <- function(input, output, session) {

  # ── Shared reactive state passed to all modules ──────────────────────────
  shared <- reactiveValues(
    fcs_folder       = NULL,
    raw_flowset      = NULL,   # original loaded flowSet
    comp_flowset     = NULL,   # after compensation
    trans_flowset    = NULL,   # after transformation
    gating_set       = NULL,   # GatingSet object
    annotation       = NULL,   # data.frame: sample annotation
    channels         = NULL,   # character: selected channel names
    spillover_matrix = NULL,   # matrix: current spillover
    transforms       = list(), # list of transform objects per channel
    gate_list        = list(), # named list of gate objects
    dim_result       = NULL,   # dimensionality reduction output
    stats_result     = NULL,   # statistics table
    experiment_name  = "Untitled Experiment",
    n_samples        = 0L,
    status           = "idle"  # "idle" | "busy" | "error"
  )

  # ── Module servers ────────────────────────────────────────────────────────
  callModule(setupServer,          "setup",          shared = shared)
  callModule(compensationServer,   "compensation",   shared = shared)
  callModule(transformationServer, "transformation", shared = shared)
  callModule(gatingServer,         "gating",         shared = shared)
  callModule(visualizationServer,  "visualization",  shared = shared)
  callModule(dimreduxServer,       "dimredux",       shared = shared)
  callModule(statisticsServer,     "statistics",     shared = shared)

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
      showNotification("Session save not yet implemented in this version.", type = "message", duration = 3)
    } else if (evt == "export_results") {
      updateTabItems(session, "sidebar_menu", "statistics")
      showNotification("Navigate to Statistics to export results.", type = "message", duration = 3)
    }
  })

  # ── Session cleanup ────────────────────────────────────────────────────────
  session$onSessionEnded(function() {
    message("[StreamFLOW] Session ended, cleaning up.")
    gc()
  })
}
