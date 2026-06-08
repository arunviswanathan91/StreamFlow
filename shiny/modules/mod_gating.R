# StreamFLOW - mod_gating.R
# Interactive polygon gating via plotly click events

# ── UI ────────────────────────────────────────────────────────────────────────
gatingUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left panel
      column(4,
        box(
          title = "Gate Controls", width = NULL, solidHeader = TRUE,

          # Gate tree display
          tags$div(
            style = "background: #243447; border-radius: 4px; padding: 10px; margin-bottom: 10px; min-height: 60px;",
            tags$p(style = "font-size: 10px; color: #5A7A8A; margin: 0 0 4px 0;", "GATE HIERARCHY"),
            uiOutput(ns("gate_tree_ui"))
          ),

          selectInput(ns("parent_pop"),   "Gate From (Parent)", choices = c("root")),
          textInput(  ns("gate_name"),    "Gate Name",          value  = "Population1"),
          selectInput(ns("gate_type"),    "Gate Type",
                      choices = c("Polygon"   = "polygon",
                                  "Rectangle" = "rectangle",
                                  "Ellipse"   = "ellipse",
                                  "Threshold" = "threshold",
                                  "Quadrant"  = "quadrant"),
                      selected = "polygon"),
          uiOutput(ns("axis_selectors")),
          selectInput(ns("gate_sample"), "Gate On Sample",
                      choices = character(), multiple = FALSE),

          tags$hr(),
          checkboxInput(ns("show_gates"), "Show existing gates on plot", value = TRUE),

          fluidRow(
            column(6,
              actionButton(ns("draw_gate_btn"),
                           tagList(icon("pencil-alt"), " Draw Gate"),
                           class = "btn btn-primary btn-block btn-sm")
            ),
            column(6,
              actionButton(ns("save_gate_btn"),
                           tagList(icon("save"), " Save Gate"),
                           class = "btn btn-success btn-block btn-sm")
            )
          ),
          tags$div(style = "margin-top: 6px;",
            fluidRow(
              column(6,
                actionButton(ns("remove_gate_btn"),
                             tagList(icon("trash"), " Remove"),
                             class = "btn btn-danger btn-block btn-sm")
              ),
              column(6,
                actionButton(ns("apply_all_btn"),
                             tagList(icon("play"), " Apply All"),
                             class = "btn btn-default btn-block btn-sm")
              )
            )
          )
        ),

        # Population statistics mini-table
        box(
          title = "Population Statistics", width = NULL, solidHeader = TRUE,
          DTOutput(ns("gate_stats_table"))
        )
      ),

      # Right panel
      column(8,
        box(
          title = "Gating Plot", width = NULL, solidHeader = TRUE,
          tags$div(
            id    = ns("gating_instructions"),
            style = "font-size: 11px; color: #8899AA; margin-bottom: 8px; padding: 6px; background: #243447; border-radius: 4px;",
            icon("info-circle"),
            " Click on the plot to add vertices. Double-click to close the polygon and save gate."
          ),
          withSpinner(
            plotlyOutput(ns("gating_plot"), height = "480px"),
            color = "#00B4D8"
          ),
          uiOutput(ns("vertex_count_ui"))
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
gatingServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    vertices     = list(),   # list of x,y click points for current gate
    drawing      = FALSE,    # whether we are in draw mode
    gates        = list(),   # named list: gate_name -> list(gate, parent, x_ch, y_ch)
    gating_set   = NULL,
    gate_stats   = NULL
  )

  # ── Channel selectors ────────────────────────────────────────────────────
  output$axis_selectors <- renderUI({
    channels <- shared$channels
    req(channels)
    tagList(
      selectInput(ns("x_channel"), "X Axis Channel", choices = channels, selected = channels[1]),
      selectInput(ns("y_channel"), "Y Axis Channel", choices = channels,
                  selected = if (length(channels) > 1) channels[2] else channels[1])
    )
  })

  # Update sample selector
  observe({
    fs <- best_flowset()
    req(fs)
    updateSelectInput(session, "gate_sample",
                      choices  = sampleNames(fs),
                      selected = sampleNames(fs)[1])
  })

  # Update parent population choices
  observe({
    parents <- c("root", names(local$gates))
    updateSelectInput(session, "parent_pop", choices = parents, selected = "root")
  })

  # Helper: get best available flowset
  best_flowset <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Gate tree UI ─────────────────────────────────────────────────────────
  output$gate_tree_ui <- renderUI({
    gates <- local$gates
    if (length(gates) == 0) {
      return(tags$span(style = "color: #5A7A8A; font-size: 12px;", "No gates defined yet"))
    }

    make_tree <- function(parent, depth = 0) {
      children <- names(gates)[sapply(names(gates), function(g) gates[[g]]$parent == parent)]
      lapply(children, function(g) {
        stats <- local$gate_stats
        pct <- if (!is.null(stats) && g %in% stats$Population) {
          p <- stats$FreqOfParent[stats$Population == g]
          if (length(p) > 0) sprintf(" (%.1f%%)", p[1]) else ""
        } else ""
        tags$div(
          style = sprintf("margin-left: %dpx;", depth * 16),
          tags$span(style = "color: #2EC4B6; font-size: 12px;",
                    icon("caret-right"), " ", g, pct)
        )
      })
    }

    tagList(
      tags$span(style = "color: #00B4D8; font-size: 12px; font-weight: 600;",
                icon("circle"), " root"),
      make_tree("root", depth = 1)
    )
  })

  # ── Main gating plot ─────────────────────────────────────────────────────
  output$gating_plot <- renderPlotly({
    x_ch   <- input$x_channel
    y_ch   <- input$y_channel
    samp   <- input$gate_sample
    fs     <- best_flowset()

    req(x_ch, y_ch, samp, fs)
    req(x_ch %in% colnames(fs), y_ch %in% colnames(fs), samp %in% sampleNames(fs))

    tryCatch({
      ff   <- fs[[samp]]
      expr <- as.data.frame(exprs(ff))
      n    <- min(nrow(expr), 5000)
      idx  <- sample(nrow(expr), n)
      df   <- expr[idx, , drop = FALSE]

      # Colour by density
      dens <- tryCatch({
        d <- MASS::kde2d(df[[x_ch]], df[[y_ch]], n = 100)
        fields::interp.surface(d, cbind(df[[x_ch]], df[[y_ch]]))
      }, error = function(e) rep(1, nrow(df)))

      p <- plot_ly(
        data   = df,
        x      = ~get(x_ch),
        y      = ~get(y_ch),
        type   = "scatter",
        mode   = "markers",
        marker = list(
          size   = 3,
          opacity = 0.6,
          color  = dens,
          colorscale = list(
            list(0,   "#0D1B2A"),
            list(0.3, "#00B4D8"),
            list(0.7, "#2EC4B6"),
            list(1,   "#FFFFFF")
          ),
          showscale = FALSE
        ),
        hoverinfo = "none",
        source    = ns("gating_plot")
      ) %>%
        plotly_dark_layout(
          title = sprintf("%s — %s vs %s", samp, x_ch, y_ch),
          xlab  = x_ch,
          ylab  = y_ch
        )

      # Draw existing gates as shapes
      if (input$show_gates && length(local$gates) > 0) {
        shapes <- list()
        for (gname in names(local$gates)) {
          g <- local$gates[[gname]]
          if (!is.null(g$vertices) && g$x_ch == x_ch && g$y_ch == y_ch) {
            vx <- c(g$vertices$x, g$vertices$x[1])
            vy <- c(g$vertices$y, g$vertices$y[1])
            path_str <- paste0(
              "M ", vx[1], " ", vy[1],
              paste(sprintf(" L %g %g", vx[-1], vy[-1]), collapse = ""),
              " Z"
            )
            shapes <- c(shapes, list(list(
              type      = "path",
              path      = path_str,
              line      = list(color = "#2EC4B6", width = 1.5),
              fillcolor = "rgba(46,196,182,0.08)",
              layer     = "above"
            )))

            # Gate label
            cx <- mean(g$vertices$x)
            cy <- mean(g$vertices$y)
            p  <- p %>% add_annotations(
              x    = cx, y = cy, text = gname,
              showarrow = FALSE,
              font = list(color = "#2EC4B6", size = 11)
            )
          }
        }
        if (length(shapes) > 0) {
          p <- p %>% layout(shapes = shapes)
        }
      }

      # Draw current in-progress vertices
      if (local$drawing && length(local$vertices) > 0) {
        vx <- sapply(local$vertices, `[[`, "x")
        vy <- sapply(local$vertices, `[[`, "y")
        p  <- p %>%
          add_trace(
            x    = c(vx, vx[1]),
            y    = c(vy, vy[1]),
            type = "scatter",
            mode = "lines+markers",
            line = list(color = "#F39C12", dash = "dash", width = 1.5),
            marker = list(color = "#F39C12", size = 6),
            showlegend = FALSE,
            hoverinfo  = "none"
          )
      }

      p
    }, error = function(e) {
      plot_ly() %>%
        layout(
          paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
          annotations = list(list(
            text = paste("Plot error:", conditionMessage(e)),
            font = list(color = "#C0392B"), showarrow = FALSE,
            xref = "paper", yref = "paper", x = 0.5, y = 0.5
          ))
        )
    })
  })

  # ── Click events for polygon drawing ─────────────────────────────────────
  observeEvent(event_data("plotly_click", source = ns("gating_plot")), {
    if (!local$drawing) return()
    ev <- event_data("plotly_click", source = ns("gating_plot"))
    req(ev)
    local$vertices <- c(local$vertices, list(list(x = ev$x, y = ev$y)))
  })

  # Double-click closes polygon and auto-saves
  observeEvent(event_data("plotly_doubleclick", source = ns("gating_plot")), {
    if (!local$drawing || length(local$vertices) < 3) return()
    local$drawing <- FALSE
    save_current_gate()
  })

  # Rectangle gate via brush
  observeEvent(event_data("plotly_selected", source = ns("gating_plot")), {
    if (input$gate_type != "rectangle") return()
    ev <- event_data("plotly_selected", source = ns("gating_plot"))
    req(!is.null(ev), !is.null(ev$x), length(ev$x) >= 2)
    x_range <- range(ev$x)
    y_range <- range(ev$y)
    local$vertices <- list(
      list(x = x_range[1], y = y_range[1]),
      list(x = x_range[2], y = y_range[1]),
      list(x = x_range[2], y = y_range[2]),
      list(x = x_range[1], y = y_range[2])
    )
    local$drawing <- FALSE
    save_current_gate()
  })

  # Draw gate button
  observeEvent(input$draw_gate_btn, {
    local$vertices <- list()
    local$drawing  <- TRUE
    showNotification(
      "Click on the plot to add vertices. Double-click to close the polygon.",
      type = "message", duration = 5
    )
  })

  # ── Save gate ─────────────────────────────────────────────────────────────
  save_current_gate <- function() {
    req(length(local$vertices) >= 3)
    x_ch     <- input$x_channel
    y_ch     <- input$y_channel
    gname    <- trimws(input$gate_name)
    parent   <- input$parent_pop

    if (nchar(gname) == 0) gname <- paste0("Population", length(local$gates) + 1)
    if (gname %in% names(local$gates)) {
      gname <- paste0(gname, "_", length(local$gates) + 1)
    }

    vx <- sapply(local$vertices, `[[`, "x")
    vy <- sapply(local$vertices, `[[`, "y")
    mat <- matrix(c(vx, vy), ncol = 2,
                  dimnames = list(NULL, c(x_ch, y_ch)))

    gate_obj <- tryCatch(
      flowCore::polygonGate(filterId = gname, .gate = mat),
      error = function(e) {
        showNotification(paste("Gate creation error:", conditionMessage(e)), type = "error")
        NULL
      }
    )

    req(gate_obj)

    local$gates[[gname]] <- list(
      gate     = gate_obj,
      parent   = parent,
      x_ch     = x_ch,
      y_ch     = y_ch,
      vertices = list(x = vx, y = vy)
    )

    local$vertices <- list()
    shared$gate_list <- local$gates

    updateTextInput(session, "gate_name",
                    value = paste0("Population", length(local$gates) + 1))

    showNotification(paste("Gate saved:", gname), type = "message", duration = 3)
    compute_gate_stats()
  }

  observeEvent(input$save_gate_btn, {
    if (length(local$vertices) >= 3) {
      local$drawing <- FALSE
      save_current_gate()
    } else {
      showNotification("Draw a gate first (need at least 3 vertices).", type = "warning", duration = 3)
    }
  })

  # ── Remove gate ───────────────────────────────────────────────────────────
  observeEvent(input$remove_gate_btn, {
    sel_rows <- input$gate_stats_table_rows_selected
    if (!is.null(sel_rows) && !is.null(local$gate_stats)) {
      gname <- local$gate_stats$Population[sel_rows[1]]
      local$gates[[gname]] <- NULL
      shared$gate_list <- local$gates
      compute_gate_stats()
      showNotification(paste("Gate removed:", gname), type = "message", duration = 2)
    } else {
      showNotification("Select a gate in the statistics table first.", type = "warning", duration = 3)
    }
  })

  # ── Apply all gates ───────────────────────────────────────────────────────
  observeEvent(input$apply_all_btn, {
    fs <- best_flowset()
    req(fs, length(local$gates) > 0)
    shared$status <- "busy"

    withProgress(message = "Applying gates to all samples...", value = 0, {
      tryCatch({
        gs <- flowWorkspace::GatingSet(fs)
        incProgress(0.3)

        # Add gates in order (root first, then children)
        for (gname in names(local$gates)) {
          g <- local$gates[[gname]]
          parent_node <- if (g$parent == "root") "root" else g$parent

          flowWorkspace::gs_pop_add(gs, g$gate, parent = parent_node)
          incProgress(0.4 / length(local$gates))
        }

        flowWorkspace::recompute(gs)
        incProgress(0.3)

        local$gating_set  <- gs
        shared$gating_set <- gs
        compute_gate_stats()
        showNotification("All gates applied successfully.", type = "message", duration = 3)
      }, error = function(e) {
        showNotification(paste("GatingSet error:", conditionMessage(e)), type = "error", duration = 6)
      })
    })

    shared$status <- "idle"
  })

  # ── Compute gate statistics ───────────────────────────────────────────────
  compute_gate_stats <- function() {
    gs <- local$gating_set %||% shared$gating_set
    if (is.null(gs) || length(local$gates) == 0) {
      # Compute from raw gate objects for quick preview
      fs   <- best_flowset()
      if (is.null(fs)) return()
      rows <- lapply(names(local$gates), function(gname) {
        g    <- local$gates[[gname]]
        samp <- input$gate_sample %||% sampleNames(fs)[1]
        if (!samp %in% sampleNames(fs)) return(NULL)
        ff   <- fs[[samp]]
        result <- tryCatch(
          flowCore::filter(ff, g$gate),
          error = function(e) NULL
        )
        if (is.null(result)) return(NULL)
        n_total  <- nrow(exprs(ff))
        n_in     <- sum(result@subSet)
        data.frame(
          Population   = gname,
          Parent       = g$parent,
          Count        = n_in,
          FreqOfParent = if (n_total > 0) round(100 * n_in / n_total, 2) else 0,
          stringsAsFactors = FALSE
        )
      })
      rows <- Filter(Negate(is.null), rows)
      if (length(rows) > 0) local$gate_stats <- do.call(rbind, rows)
      return()
    }

    tryCatch({
      nodes <- flowWorkspace::gs_get_pop_paths(gs, path = "auto")
      nodes <- nodes[nodes != "root"]
      rows  <- lapply(nodes, function(node) {
        stats <- flowWorkspace::gs_pop_get_stats(gs, node, type = "percent")
        count <- flowWorkspace::gs_pop_get_stats(gs, node, type = "count")
        data.frame(
          Population   = node,
          Parent       = flowWorkspace::gs_pop_get_parent(gs, node),
          Count        = mean(count$count),
          FreqOfParent = round(mean(stats$percent) * 100, 2),
          stringsAsFactors = FALSE
        )
      })
      local$gate_stats <- do.call(rbind, rows)
    }, error = function(e) {
      message(sprintf("[mod_gating] Stats error: %s", conditionMessage(e)))
    })
  }

  output$gate_stats_table <- renderDT({
    df <- local$gate_stats
    if (is.null(df) || nrow(df) == 0) {
      return(datatable(
        data.frame(Population = character(), Parent = character(),
                   Count = integer(), `% of Parent` = numeric()),
        options = list(dom = "t"), rownames = FALSE
      ))
    }
    datatable(
      df,
      selection = "single",
      rownames  = FALSE,
      colnames  = c("Population", "Parent", "Count", "% of Parent"),
      options   = list(dom = "t", pageLength = 20, scrollX = TRUE),
      class     = "compact"
    ) %>%
      formatRound("FreqOfParent", 2) %>%
      formatCurrency("Count", currency = "", interval = 3, mark = ",", digits = 0)
  })

  output$vertex_count_ui <- renderUI({
    if (local$drawing) {
      tags$p(
        style = "font-size: 11px; color: #F39C12; margin-top: 4px;",
        icon("pencil-alt"),
        sprintf(" Drawing... %d vertices placed. Double-click to close.", length(local$vertices))
      )
    }
  })
}
