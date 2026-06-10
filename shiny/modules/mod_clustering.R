# StreamFLOW - mod_clustering.R
# High-dimensional clustering with FlowSOM (the canonical R implementation,
# equivalent to FlowJo's FlowSOM plugin): self-organizing map + metaclustering,
# a marker x metacluster median heatmap, per-sample abundance, and a 2-channel
# scatter coloured by metacluster.
#
# FlowSOM is loaded lazily (requireNamespace) so the app still starts if the
# package is unavailable in a given environment.

# ── UI ──────────────────────────────────────────────────────────────────────────
clusteringUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      column(3,
        box(
          title = "FlowSOM Settings", width = NULL, solidHeader = TRUE,
          selectInput(ns("population"), "Population", choices = c("root")),
          uiOutput(ns("channel_ui")),
          fluidRow(
            column(6, numericInput(ns("xdim"), "SOM width",  value = 10, min = 4, max = 20)),
            column(6, numericInput(ns("ydim"), "SOM height", value = 10, min = 4, max = 20))
          ),
          numericInput(ns("k"), "Metaclusters", value = 12, min = 2, max = 40),
          numericInput(ns("subsample"), "Cells / sample", value = 5000,
                       min = 500, max = 100000, step = 500),
          numericInput(ns("seed"), "Random seed", value = 42, min = 1),
          actionButton(ns("run_btn"), tagList(icon("project-diagram"), " Run FlowSOM"),
                       class = "btn btn-primary btn-block"),
          tags$hr(),
          uiOutput(ns("status_ui"))
        )
      ),
      column(9,
        box(
          title = "Metacluster Heatmap (median expression)", width = NULL, solidHeader = TRUE,
          withSpinner(plotlyOutput(ns("heatmap"), height = "360px"), color = "#00B4D8")
        ),
        fluidRow(
          column(6,
            box(
              title = "Metacluster Map", width = NULL, solidHeader = TRUE,
              fluidRow(
                column(6, uiOutput(ns("map_x_ui"))),
                column(6, uiOutput(ns("map_y_ui")))
              ),
              withSpinner(plotlyOutput(ns("scatter"), height = "320px"), color = "#00B4D8")
            )
          ),
          column(6,
            box(
              title = "Abundance (% per sample)", width = NULL, solidHeader = TRUE,
              withSpinner(DTOutput(ns("abundance")), color = "#00B4D8")
            )
          )
        )
      )
    )
  )
}

# ── Server ──────────────────────────────────────────────────────────────────────
clusteringServer <- function(input, output, session, shared) {
  ns <- session$ns

  res <- reactiveValues(
    mc_per_cell = NULL,   # metacluster label per cell
    sample_vec  = NULL,   # sample name per cell
    data_mat    = NULL,   # cells x channels (used channels)
    chans       = NULL,
    heat        = NULL    # metacluster x channel medians
  )

  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # Population choices follow the GatingSet
  observe({
    gs <- shared$gating_set
    pops <- if (!is.null(gs))
      tryCatch(c("root", CytoExploreR::cyto_nodes(gs)), error = function(e) "root")
    else "root"
    updateSelectInput(session, "population", choices = pops, selected = "root")
  })

  output$channel_ui <- renderUI({
    chans <- shared$fluor_channels %||% shared$channels %||% character()
    if (length(chans) == 0) return(tags$p(style = "color:#F39C12;", "Load data first."))
    checkboxGroupInput(ns("channels"), "Clustering channels",
                       choices = chans,
                       selected = chans[seq_len(min(8, length(chans)))])
  })

  # Pull a population's data as a flowSet (GatingSet-aware)
  pop_flowset <- function() {
    gs  <- shared$gating_set
    pop <- input$population %||% "root"
    if (!is.null(gs)) {
      cs <- tryCatch(flowWorkspace::gs_pop_get_data(gs, pop),
                     error = function(e) tryCatch(flowWorkspace::gs_pop_get_data(gs),
                                                  error = function(e2) NULL))
      if (!is.null(cs))
        return(tryCatch(flowWorkspace::cytoset_to_flowSet(cs),
                        error = function(e) cs))
    }
    best_fs()
  }

  observeEvent(input$run_btn, {
    if (!requireNamespace("FlowSOM", quietly = TRUE)) {
      showNotification("FlowSOM package is not installed in this build.",
                       type = "error", duration = 6)
      return()
    }
    fs    <- pop_flowset()
    chans <- input$channels
    if (is.null(fs) || is.null(chans) || length(chans) < 2) {
      showNotification("Select a population and at least two channels.",
                       type = "warning", duration = 4)
      return()
    }
    shared$status <- "busy"

    withProgress(message = "Running FlowSOM…", value = 0.1, {
      tryCatch({
        ids  <- sampleNames(fs)
        nsub <- input$subsample %||% 5000
        parts <- lapply(ids, function(s) {
          e <- exprs(fs[[s]])
          keep <- intersect(chans, colnames(e))
          e <- e[, keep, drop = FALSE]
          if (nrow(e) > nsub) e <- e[sample(nrow(e), nsub), , drop = FALSE]
          list(mat = e, samp = rep(s, nrow(e)))
        })
        mat <- do.call(rbind, lapply(parts, `[[`, "mat"))
        samp_vec <- unlist(lapply(parts, `[[`, "samp"))
        incProgress(0.3, message = "Building SOM…")

        ff   <- flowCore::flowFrame(as.matrix(mat))
        cols <- colnames(mat)
        fsom <- FlowSOM::FlowSOM(
          ff, colsToUse = cols, nClus = input$k %||% 12,
          xdim = input$xdim %||% 10, ydim = input$ydim %||% 10,
          scale = FALSE, seed = input$seed %||% 42)
        incProgress(0.4, message = "Metaclustering…")

        mc <- as.factor(FlowSOM::GetMetaclusters(fsom))

        # metacluster x channel medians
        heat <- sapply(cols, function(ch)
          tapply(mat[, ch], mc, median, na.rm = TRUE))
        heat <- t(heat)  # channels x metaclusters

        res$mc_per_cell <- mc
        res$sample_vec  <- samp_vec
        res$data_mat    <- mat
        res$chans       <- cols
        res$heat        <- heat
        incProgress(0.2)
        showNotification(sprintf("FlowSOM done: %d cells, %d metaclusters.",
                                 nrow(mat), nlevels(mc)),
                         type = "message", duration = 4)
      }, error = function(e)
        showNotification(paste("FlowSOM error:", conditionMessage(e)),
                         type = "error", duration = 6))
    })
    shared$status <- "idle"
  })

  output$status_ui <- renderUI({
    if (is.null(res$mc_per_cell))
      tags$p(style = "color:#5A7A8A;font-size:12px;", "No clustering run yet.")
    else
      tags$p(style = "color:#2EC4B6;font-size:12px;",
             sprintf("%d cells · %d metaclusters",
                     length(res$mc_per_cell), nlevels(res$mc_per_cell)))
  })

  # ── Heatmap ─────────────────────────────────────────────────────────────────
  output$heatmap <- renderPlotly({
    h <- res$heat
    req(h)
    # per-channel min-max scale for visual comparability
    hs <- t(apply(h, 1, function(r) {
      rng <- range(r, na.rm = TRUE)
      if (diff(rng) == 0) rep(0.5, length(r)) else (r - rng[1]) / diff(rng)
    }))
    plot_ly(
      x = paste0("MC", colnames(h)), y = rownames(h), z = hs,
      type = "heatmap",
      colorscale = list(list(0, "#081420"), list(0.5, "#00B4D8"), list(1, "#FFFFFF")),
      hovertemplate = "MC %{x}<br>%{y}<br>scaled %{z:.2f}<extra></extra>"
    ) %>%
      layout(paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
             xaxis = list(color = "#E0E0E0"), yaxis = list(color = "#E0E0E0"),
             margin = list(l = 90, b = 50))
  })

  # ── Scatter map controls ────────────────────────────────────────────────────
  output$map_x_ui <- renderUI({
    req(res$chans)
    selectInput(ns("map_x"), "X", choices = res$chans, selected = res$chans[1])
  })
  output$map_y_ui <- renderUI({
    req(res$chans)
    selectInput(ns("map_y"), "Y", choices = res$chans,
                selected = if (length(res$chans) > 1) res$chans[2] else res$chans[1])
  })

  output$scatter <- renderPlotly({
    req(res$data_mat, res$mc_per_cell, input$map_x, input$map_y)
    df <- as.data.frame(res$data_mat)
    df$MC <- res$mc_per_cell
    n <- min(nrow(df), 30000)
    df <- df[sample(nrow(df), n), , drop = FALSE]
    plot_ly(df, x = ~get(input$map_x), y = ~get(input$map_y),
            color = ~MC, type = "scattergl", mode = "markers",
            marker = list(size = 3, opacity = 0.6)) %>%
      layout(paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
             xaxis = list(title = input$map_x, color = "#E0E0E0", gridcolor = "#243447"),
             yaxis = list(title = input$map_y, color = "#E0E0E0", gridcolor = "#243447"),
             legend = list(font = list(color = "#E0E0E0")))
  })

  # ── Abundance table ─────────────────────────────────────────────────────────
  output$abundance <- renderDT({
    req(res$mc_per_cell, res$sample_vec)
    tab <- table(Sample = res$sample_vec, Metacluster = res$mc_per_cell)
    pct <- round(100 * prop.table(tab, margin = 1), 2)
    df  <- as.data.frame.matrix(pct)
    df  <- cbind(Sample = rownames(df), df)
    datatable(df, rownames = FALSE, selection = "none",
              options = list(dom = "tp", pageLength = 10, scrollX = TRUE),
              class = "compact")
  })
}
