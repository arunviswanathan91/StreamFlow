# StreamFLOW - mod_gating.R
# FlowJo-style interactive gating built on plotly events + flowCore/flowWorkspace.
#
# Gate types:
#   Polygon     – click vertices, double-click to close          (2D)
#   Rectangle   – brush a box                                    (2D)
#   Ellipse     – brush a box, ellipse inscribed inside it       (2D)
#   Quadrant    – single click sets the X/Y crosshair → 4 pops   (2D)
#   Interval    – brush an X-range on the histogram              (1D)
#   Boolean     – combine existing populations with AND/OR/NOT
#
# Extra UX (toward FlowJo parity):
#   - 1D / 2D plot toggle (histogram <-> scatter) without leaving the page
#   - Copy / Paste a gate onto another parent/sample
#   - Apply to all samples (a GatingSet gate already spans every file)
#   - Inline MFI / statistics via CytoExploreR::cyto_stats_compute
#
# CytoExploreR / flowWorkspace API used:
#   cyto_nodes(), cyto_names(), cyto_stats_compute(),
#   cyto_gatingTemplate_generate(), cyto_gatingTemplate_apply(),
#   cyto_gate_remove(), cyto_plot_gating_tree(),
#   flowWorkspace::GatingSet/gs_pop_add/recompute/gs_pop_get_stats/gs_pop_get_parent,
#   flowCore::polygonGate/rectangleGate/ellipsoidGate/quadGate/booleanFilter

# ── Constants ──────────────────────────────────────────────────────────────────
.GATE_TYPES_2D <- c("Polygon"   = "polygon",
                    "Rectangle" = "rectangle",
                    "Ellipse"   = "ellipse",
                    "Quadrant"  = "quadrant",
                    "Boundary"  = "boundary",
                    "Threshold" = "threshold",
                    "Boolean"   = "boolean")
.GATE_TYPES_1D <- c("Interval"  = "interval",
                    "Boundary"  = "boundary",
                    "Threshold" = "threshold",
                    "Boolean"   = "boolean")

# ── UI ──────────────────────────────────────────────────────────────────────────
gatingUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # ── Left panel ─────────────────────────────────────────────────────────
      column(4,
        box(
          title = "Gate Hierarchy", width = NULL, solidHeader = TRUE,
          tags$div(
            style = "background:#0D1B2A;border-radius:4px;padding:10px;min-height:80px;margin-bottom:10px;overflow-y:auto;max-height:220px;",
            uiOutput(ns("gate_tree_ui"))
          ),
          fluidRow(
            column(6,
              actionButton(ns("refresh_tree_btn"),
                           tagList(icon("sync"), " Refresh Tree"),
                           class = "btn btn-default btn-block btn-sm")
            ),
            column(6,
              downloadButton(ns("dl_template"),
                             "Export Template",
                             class = "btn btn-default btn-block btn-sm")
            )
          )
        ),

        box(
          title = "Gate Controls", width = NULL, solidHeader = TRUE,
          # Plot dimension toggle (FlowJo-style histogram <-> scatter)
          radioButtons(ns("plot_dim"), "Plot Mode",
                       choices  = c("2D Scatter" = "2d", "1D Histogram" = "1d"),
                       selected = "2d", inline = TRUE),
          selectInput(ns("parent_pop"), "Gate From (Parent)", choices = c("root")),
          textInput(ns("gate_name"),   "Gate Name",  value = "Population1"),
          selectInput(ns("gate_type"), "Gate Type", choices = .GATE_TYPES_2D,
                      selected = "polygon"),
          uiOutput(ns("axis_selectors_ui")),
          selectInput(ns("gate_sample"), "Gate On Sample", choices = character()),
          uiOutput(ns("boolean_ui")),
          tags$hr(),
          checkboxInput(ns("show_gates"), "Show existing gates on plot", value = TRUE),
          fluidRow(
            column(6,
              actionButton(ns("draw_btn"),
                           tagList(icon("pencil-alt"), " Start Drawing"),
                           class = "btn btn-primary btn-block btn-sm")
            ),
            column(6,
              actionButton(ns("save_btn"),
                           tagList(icon("save"), " Save Gate"),
                           class = "btn btn-success btn-block btn-sm")
            )
          ),
          tags$div(style = "margin-top:6px;",
            fluidRow(
              column(4,
                actionButton(ns("copy_btn"),
                             tagList(icon("copy"), " Copy"),
                             class = "btn btn-default btn-block btn-sm")
              ),
              column(4,
                actionButton(ns("paste_btn"),
                             tagList(icon("paste"), " Paste"),
                             class = "btn btn-default btn-block btn-sm")
              ),
              column(4,
                actionButton(ns("remove_btn"),
                             tagList(icon("trash"), " Remove"),
                             class = "btn btn-danger btn-block btn-sm")
              )
            )
          ),
          tags$div(style = "margin-top:6px;",
            actionButton(ns("apply_all_btn"),
                         tagList(icon("play"), " Apply to All Samples"),
                         class = "btn btn-warning btn-block btn-sm")
          ),
          tags$hr(),
          fluidRow(
            column(6,
              actionButton(ns("import_template_btn"),
                           tagList(icon("file-import"), " Import Template"),
                           class = "btn btn-default btn-block btn-sm")
            ),
            column(6,
              actionButton(ns("template_file_btn"),
                           label   = tagList(icon("folder-open"), " Browse"),
                           class   = "btn btn-default btn-block btn-sm",
                           onclick = sprintf(
                             "streamflowPickFile('%s', 'Select gatingTemplate CSV', [{name: 'CSV files', extensions: ['csv']}])",
                             ns("template_file_picked")
                           ))
            )
          )
        )
      ),

      # ── Right panel ────────────────────────────────────────────────────────
      column(8,
        box(
          title = "Gating Plot", width = NULL, solidHeader = TRUE,
          tags$div(style = "display:flex;justify-content:flex-end;margin-bottom:4px;",
            actionButton(ns("popout_btn"),
                         tagList(icon("external-link-alt"), " Pop-out Sample"),
                         class = "btn btn-default btn-sm")),
          uiOutput(ns("drawing_instructions_ui")),
          withSpinner(plotlyOutput(ns("gating_plot"), height = "430px"), color = "#00B4D8"),
          uiOutput(ns("vertex_status_ui")),
          tags$script(HTML(sprintf(
            "$(document).on('contextmenu', '#%s', function(e){ e.preventDefault();
               if(window.Shiny) Shiny.setInputValue('%s', Date.now(), {priority:'event'}); });",
            ns("gating_plot"), ns("popout_request"))))
        ),

        fluidRow(
          column(7,
            box(
              title = "Population Statistics", width = NULL, solidHeader = TRUE,
              uiOutput(ns("stat_channel_ui")),
              withSpinner(DTOutput(ns("gate_stats_table")), color = "#00B4D8")
            )
          ),
          column(5,
            box(
              title = "Gating Tree", width = NULL, solidHeader = TRUE,
              uiOutput(ns("gating_tree_sample_ui")),
              withSpinner(
                htmlOutput(ns("cyto_gating_tree")),
                color = "#00B4D8"
              ),
              tags$p(style = "font-size:11px;color:#5A7A8A;",
                     "Interactive D3 tree — click nodes to explore.")
            )
          )
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
gatingServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    vertices      = list(),
    drawing       = FALSE,
    gates         = list(),   # name -> list(type, gate, parent, x_ch, y_ch, geom)
    gating_set    = NULL,
    gate_stats    = NULL,
    template_path = NULL,
    clipboard     = NULL
  )

  # ── Best available flowset ─────────────────────────────────────────────────
  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Gate type choices track plot dimension ─────────────────────────────────
  observeEvent(input$plot_dim, {
    choices <- if (input$plot_dim == "1d") .GATE_TYPES_1D else .GATE_TYPES_2D
    sel <- if (input$gate_type %in% choices) input$gate_type else choices[[1]]
    updateSelectInput(session, "gate_type", choices = choices, selected = sel)
    local$drawing  <- FALSE
    local$vertices <- list()
  }, ignoreInit = TRUE)

  # ── Axis channel selectors (Y hidden in 1D) ────────────────────────────────
  output$axis_selectors_ui <- renderUI({
    channels <- shared$channels %||% character()
    one_d    <- isTRUE(input$plot_dim == "1d")
    tagList(
      selectInput(ns("x_ch"), if (one_d) "Channel" else "X Channel",
                  choices = channels, selected = channels[1]),
      if (!one_d)
        selectInput(ns("y_ch"), "Y Channel", choices = channels,
                    selected = if (length(channels) > 1) channels[2] else channels[1])
    )
  })

  # ── Boolean gate builder (only for type = boolean) ─────────────────────────
  output$boolean_ui <- renderUI({
    req(input$gate_type == "boolean")
    pops <- names(local$gates)
    if (length(pops) < 1)
      return(tags$p(style = "color:#F39C12;font-size:11px;",
                    "Create at least one gate before building a boolean gate."))
    tagList(
      selectInput(ns("bool_op"), "Operator",
                  choices = c("AND (&)" = "&", "OR (|)" = "|", "NOT (!)" = "!")),
      selectInput(ns("bool_pops"), "Populations", choices = pops,
                  multiple = TRUE)
    )
  })

  # ── Sample selector ────────────────────────────────────────────────────────
  observe({
    fs <- best_fs()
    req(fs)
    ids    <- sampleNames(fs)
    labels <- tryCatch(CytoExploreR::cyto_names(fs), error = function(e) ids)
    if (length(labels) != length(ids)) labels <- ids
    choices <- setNames(ids, labels)
    updateSelectInput(session, "gate_sample", choices = choices, selected = ids[1])
  })

  # ── Parent population choices ──────────────────────────────────────────────
  observe({
    gs <- local$gating_set
    parents <- if (!is.null(gs)) {
      tryCatch(c("root", CytoExploreR::cyto_nodes(gs)), error = function(e) c("root"))
    } else {
      c("root", names(local$gates))
    }
    updateSelectInput(session, "parent_pop", choices = parents,
                      selected = isolate(input$parent_pop) %||% "root")
  })

  # ── MFI channel selector ───────────────────────────────────────────────────
  output$stat_channel_ui <- renderUI({
    channels <- shared$fluor_channels %||% shared$channels %||% character()
    if (length(channels) == 0) return(NULL)
    selectInput(ns("stat_channels"), "MFI channels",
                choices = channels,
                selected = channels[seq_len(min(3, length(channels)))],
                multiple = TRUE)
  })

  # ── Gate tree sidebar display ──────────────────────────────────────────────
  output$gate_tree_ui <- renderUI({
    gates <- local$gates
    gs    <- local$gating_set

    node_names <- if (!is.null(gs)) {
      tryCatch(CytoExploreR::cyto_nodes(gs), error = function(e) names(gates))
    } else {
      names(gates)
    }

    if (length(node_names) == 0 && length(gates) == 0)
      return(tags$span(style = "color:#5A7A8A;font-size:12px;", "No gates yet"))

    all_names <- if (length(node_names) > 0) node_names else names(gates)

    tagList(
      tags$div(style = "color:#00B4D8;font-size:12px;font-weight:600;margin-bottom:4px;",
               icon("circle"), " root"),
      lapply(all_names, function(gname) {
        stats <- local$gate_stats
        pct <- if (!is.null(stats) && "Population" %in% names(stats)) {
          p <- stats$FreqOfParent[stats$Population == gname]
          if (length(p) > 0) sprintf(" %.1f%%", p[1]) else ""
        } else ""
        parent_info <- if (!is.null(gates[[gname]]$parent))
          paste0(" [", gates[[gname]]$parent, "]") else ""
        type_tag <- if (!is.null(gates[[gname]]$type))
          paste0(" · ", gates[[gname]]$type) else ""
        tags$div(style = "margin-left:14px;padding:2px 0;",
          tags$span(style = "color:#2EC4B6;font-size:12px;",
                    icon("caret-right"), " ", gname,
                    tags$span(style = "color:#5A7A8A;font-size:11px;",
                              pct, parent_info, type_tag))
        )
      })
    )
  })

  # ── Sample data helper (subsampled for responsive plotting) ────────────────
  sample_df <- reactive({
    fs   <- best_fs()
    samp <- input$gate_sample
    req(fs, samp, samp %in% sampleNames(fs))
    ff   <- fs[[samp]]
    expr <- as.data.frame(exprs(ff))
    n    <- min(nrow(expr), 15000)
    expr[sample(nrow(expr), n), , drop = FALSE]
  })

  # ── Main gating plot ───────────────────────────────────────────────────────
  output$gating_plot <- renderPlotly({
    one_d <- isTRUE(input$plot_dim == "1d")
    x_ch  <- input$x_ch
    samp  <- input$gate_sample
    fs    <- best_fs()
    req(x_ch, samp, fs, x_ch %in% colnames(fs), samp %in% sampleNames(fs))

    if (one_d) return(render_hist(fs, samp, x_ch))

    y_ch <- input$y_ch
    req(y_ch, y_ch %in% colnames(fs))
    render_scatter(samp, x_ch, y_ch)
  })

  # ── 2D scatter renderer ────────────────────────────────────────────────────
  render_scatter <- function(samp, x_ch, y_ch) {
    tryCatch({
      df <- sample_df()
      dens <- tryCatch({
        d <- MASS::kde2d(df[[x_ch]], df[[y_ch]], n = 100)
        fields::interp.surface(d, cbind(df[[x_ch]], df[[y_ch]]))
      }, error = function(e) rep(1, nrow(df)))
      dens[!is.finite(dens)] <- 0

      p <- plot_ly(
        data = df, x = ~get(x_ch), y = ~get(y_ch),
        type = "scattergl", mode = "markers",
        marker = list(
          size = 3, opacity = 0.6, color = dens,
          colorscale = list(
            list(0,   "#081420"), list(0.3, "#00B4D8"),
            list(0.7, "#2EC4B6"), list(1,   "#FFFFFF")
          ),
          showscale = FALSE
        ),
        hoverinfo = "none", source = ns("gating_plot")
      ) %>%
        plotly_dark_layout(
          title = sprintf("%s — %s vs %s", samp, x_ch, y_ch),
          xlab  = x_ch, ylab = y_ch
        ) %>%
        config(doubleClick = FALSE, displayModeBar = TRUE,
               modeBarButtonsToRemove = list("lasso2d"))

      # Saved gates
      if (isTRUE(input$show_gates) && length(local$gates) > 0) {
        gl <- build_gate_shapes(x_ch, y_ch)
        if (length(gl$shapes) > 0)
          p <- p %>% layout(shapes = gl$shapes, annotations = gl$anns)
      }

      # In-progress polygon
      if (local$drawing && length(local$vertices) > 0) {
        vx <- sapply(local$vertices, `[[`, "x")
        vy <- sapply(local$vertices, `[[`, "y")
        p  <- p %>% add_trace(
          x = c(vx, vx[1]), y = c(vy, vy[1]),
          type = "scatter", mode = "lines+markers",
          line   = list(color = "#F39C12", dash = "dash", width = 1.5),
          marker = list(color = "#F39C12", size = 6),
          showlegend = FALSE, hoverinfo = "none"
        )
      }
      p
    }, error = function(e) plot_error(e))
  }

  # ── 1D histogram renderer ──────────────────────────────────────────────────
  render_hist <- function(fs, samp, x_ch) {
    tryCatch({
      df  <- sample_df()
      xv  <- df[[x_ch]]
      xv  <- xv[is.finite(xv)]
      d   <- density(xv, n = 512)

      p <- plot_ly(
        x = d$x, y = d$y, type = "scatter", mode = "lines",
        fill = "tozeroy", line = list(color = "#00B4D8", width = 1.5),
        fillcolor = "rgba(0,180,216,0.25)",
        hoverinfo = "none", source = ns("gating_plot")
      ) %>%
        plotly_dark_layout(
          title = sprintf("%s — %s", samp, x_ch),
          xlab  = x_ch, ylab = "Density"
        ) %>%
        config(doubleClick = FALSE, displayModeBar = TRUE,
               modeBarButtonsToRemove = list("lasso2d"))

      # Saved interval gates on this channel
      if (isTRUE(input$show_gates) && length(local$gates) > 0) {
        shapes <- list(); anns <- list()
        ymax <- max(d$y)
        for (gname in names(local$gates)) {
          g <- local$gates[[gname]]
          if (!identical(g$x_ch, x_ch)) next
          if (identical(g$type, "interval")) {
            shapes <- c(shapes, list(list(
              type = "rect", x0 = g$geom$lo, x1 = g$geom$hi,
              y0 = 0, y1 = ymax, yref = "y",
              line = list(color = "#2EC4B6", width = 1.5),
              fillcolor = "rgba(46,196,182,0.10)", layer = "above"
            )))
            anns <- c(anns, list(list(
              x = mean(c(g$geom$lo, g$geom$hi)), y = ymax * 0.95,
              text = gname, showarrow = FALSE,
              font = list(color = "#2EC4B6", size = 11)
            )))
          } else if (g$type %in% c("boundary", "threshold")) {
            xmin <- min(d$x); xmax <- max(d$x)
            rx0 <- if (g$type == "boundary") xmin else g$geom$x0
            rx1 <- if (g$type == "boundary") g$geom$x0 else xmax
            shapes <- c(shapes, list(
              list(type = "rect", x0 = rx0, x1 = rx1, y0 = 0, y1 = ymax, yref = "y",
                   line = list(width = 0),
                   fillcolor = "rgba(46,196,182,0.10)", layer = "below"),
              list(type = "line", x0 = g$geom$x0, x1 = g$geom$x0,
                   y0 = 0, y1 = ymax, yref = "y",
                   line = list(color = "#2EC4B6", width = 1.5, dash = "dash"))))
            anns <- c(anns, list(list(
              x = g$geom$x0, y = ymax * 0.95, text = gname, showarrow = FALSE,
              font = list(color = "#2EC4B6", size = 11))))
          }
        }
        if (length(shapes) > 0)
          p <- p %>% layout(shapes = shapes, annotations = anns)
      }
      p
    }, error = function(e) plot_error(e))
  }

  plot_error <- function(e) {
    plot_ly() %>%
      layout(paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
             annotations = list(list(
               text = paste("Plot error:", conditionMessage(e)),
               font = list(color = "#C0392B"), showarrow = FALSE,
               xref = "paper", yref = "paper", x = 0.5, y = 0.5)))
  }

  # ── Build plotly shapes for saved 2D gates ─────────────────────────────────
  build_gate_shapes <- function(x_ch, y_ch) {
    shapes <- list(); anns <- list()
    for (gname in names(local$gates)) {
      g <- local$gates[[gname]]
      if (is.null(g$x_ch) || g$x_ch != x_ch) next
      if (identical(g$type, "interval")) next  # 1D only

      if (identical(g$type, "ellipse")) {
        ge <- g$geom
        shapes <- c(shapes, list(list(
          type = "circle", xref = "x", yref = "y",
          x0 = ge$cx - ge$rx, x1 = ge$cx + ge$rx,
          y0 = ge$cy - ge$ry, y1 = ge$cy + ge$ry,
          line = list(color = "#2EC4B6", width = 1.5),
          fillcolor = "rgba(46,196,182,0.06)", layer = "above"
        )))
        anns <- c(anns, list(list(
          x = ge$cx, y = ge$cy, text = gname, showarrow = FALSE,
          font = list(color = "#2EC4B6", size = 11))))
      } else if (identical(g$type, "quadrant")) {
        if (is.null(g$y_ch) || g$y_ch != y_ch) next
        shapes <- c(shapes,
          list(list(type = "line", x0 = g$geom$x0, x1 = g$geom$x0,
                    yref = "paper", y0 = 0, y1 = 1,
                    line = list(color = "#F39C12", width = 1.2, dash = "dot"))),
          list(list(type = "line", y0 = g$geom$y0, y1 = g$geom$y0,
                    xref = "paper", x0 = 0, x1 = 1,
                    line = list(color = "#F39C12", width = 1.2, dash = "dot"))))
        anns <- c(anns, list(list(
          x = g$geom$x0, y = g$geom$y0, text = gname, showarrow = FALSE,
          font = list(color = "#F39C12", size = 11))))
      } else if (g$type %in% c("boundary", "threshold")) {
        # vertical limit on X; horizontal limit on Y when this is the Y channel
        shapes <- c(shapes, list(list(
          type = "line", x0 = g$geom$x0, x1 = g$geom$x0,
          yref = "paper", y0 = 0, y1 = 1,
          line = list(color = "#2EC4B6", width = 1.5, dash = "dash"))))
        if (!is.null(g$y_ch) && g$y_ch == y_ch && !is.null(g$geom$y0))
          shapes <- c(shapes, list(list(
            type = "line", y0 = g$geom$y0, y1 = g$geom$y0,
            xref = "paper", x0 = 0, x1 = 1,
            line = list(color = "#2EC4B6", width = 1.5, dash = "dash"))))
        anns <- c(anns, list(list(
          x = g$geom$x0, y = g$geom$y0 %||% 0, text = gname, showarrow = FALSE,
          font = list(color = "#2EC4B6", size = 11))))
      } else if (!is.null(g$geom$x) && (is.null(g$y_ch) || g$y_ch == y_ch)) {
        # polygon / rectangle (stored as vertices)
        vx <- c(g$geom$x, g$geom$x[1]); vy <- c(g$geom$y, g$geom$y[1])
        path_str <- paste0(
          "M ", vx[1], " ", vy[1],
          paste(sprintf(" L %g %g", vx[-1], vy[-1]), collapse = ""), " Z")
        shapes <- c(shapes, list(list(
          type = "path", path = path_str,
          line = list(color = "#2EC4B6", width = 1.5),
          fillcolor = "rgba(46,196,182,0.06)", layer = "above")))
        anns <- c(anns, list(list(
          x = mean(g$geom$x), y = mean(g$geom$y), text = gname,
          showarrow = FALSE, font = list(color = "#2EC4B6", size = 11))))
      }
    }
    list(shapes = shapes, anns = anns)
  }

  # ── Plotly click → polygon vertex or quadrant crosshair ────────────────────
  observeEvent(event_data("plotly_click", source = ns("gating_plot")), {
    if (!local$drawing) return()
    ev <- event_data("plotly_click", source = ns("gating_plot"))
    req(ev)
    gt <- input$gate_type
    if (gt == "quadrant") {
      local$drawing <- FALSE
      commit_quadrant(ev$x, ev$y)
    } else if (gt %in% c("boundary", "threshold")) {
      local$drawing <- FALSE
      commit_corner(gt, ev$x, ev$y)
    } else if (gt == "polygon") {
      local$vertices <- c(local$vertices, list(list(x = ev$x, y = ev$y)))
    }
  })

  # ── Double-click → close polygon ──────────────────────────────────────────
  observeEvent(event_data("plotly_doubleclick", source = ns("gating_plot")), {
    if (!local$drawing || input$gate_type != "polygon" ||
        length(local$vertices) < 3) return()
    local$drawing <- FALSE
    commit_polygon()
  })

  # ── Brush → rectangle / ellipse (2D) or interval (1D) ──────────────────────
  observeEvent(event_data("plotly_selected", source = ns("gating_plot")), {
    ev <- event_data("plotly_selected", source = ns("gating_plot"))
    req(!is.null(ev$x), length(ev$x) >= 2)
    gt <- input$gate_type

    if (input$plot_dim == "1d" && gt == "interval") {
      local$drawing <- FALSE
      commit_interval(range(ev$x))
    } else if (gt == "rectangle") {
      local$drawing <- FALSE
      commit_rectangle(range(ev$x), range(ev$y))
    } else if (gt == "ellipse") {
      local$drawing <- FALSE
      commit_ellipse(range(ev$x), range(ev$y))
    }
  })

  # ── Start drawing ──────────────────────────────────────────────────────────
  observeEvent(input$draw_btn, {
    local$vertices <- list()
    local$drawing  <- TRUE
    msg <- switch(input$gate_type,
      polygon   = "Click to add vertices, double-click to close the polygon.",
      rectangle = "Brush a box on the plot to define the rectangle.",
      ellipse   = "Brush a box — the ellipse is inscribed inside it.",
      quadrant  = "Click once to place the quadrant crosshair.",
      interval  = "Brush an X-range on the histogram.",
      boundary  = "Click once — events below the point (lower-left) are gated.",
      threshold = "Click once — events above the point (upper-right) are gated.",
      boolean   = "Boolean gates are built from existing populations — use Save Gate.",
      "Draw on the plot.")
    showNotification(msg, type = "message", duration = 6)
  })

  # ── Gate name helper ───────────────────────────────────────────────────────
  unique_name <- function(base) {
    gname <- trimws(base)
    if (nchar(gname) == 0) gname <- paste0("Population", length(local$gates) + 1)
    while (gname %in% names(local$gates))
      gname <- paste0(gname, "_", length(local$gates) + 1)
    gname
  }

  store_gate <- function(gname, type, gate, x_ch, y_ch, geom) {
    local$gates[[gname]] <- list(type = type, gate = gate, parent = input$parent_pop %||% "root",
                                 x_ch = x_ch, y_ch = y_ch, geom = geom)
    local$vertices   <- list()
    shared$gate_list <- local$gates
    updateTextInput(session, "gate_name",
                    value = paste0("Population", length(local$gates) + 1))
    showNotification(paste("Gate saved:", gname), type = "message", duration = 3)
    compute_gate_stats()
  }

  # ── Commit: polygon ────────────────────────────────────────────────────────
  commit_polygon <- function() {
    req(length(local$vertices) >= 3)
    x_ch <- input$x_ch; y_ch <- input$y_ch
    gname <- unique_name(input$gate_name)
    vx <- sapply(local$vertices, `[[`, "x"); vy <- sapply(local$vertices, `[[`, "y")
    mat <- matrix(c(vx, vy), ncol = 2, dimnames = list(NULL, c(x_ch, y_ch)))
    gate <- tryCatch(flowCore::polygonGate(filterId = gname, .gate = mat),
                     error = function(e) { gate_err(e); NULL })
    req(gate)
    store_gate(gname, "polygon", gate, x_ch, y_ch, list(x = vx, y = vy))
  }

  # ── Commit: rectangle ──────────────────────────────────────────────────────
  commit_rectangle <- function(xr, yr) {
    x_ch <- input$x_ch; y_ch <- input$y_ch
    gname <- unique_name(input$gate_name)
    vx <- c(xr[1], xr[2], xr[2], xr[1]); vy <- c(yr[1], yr[1], yr[2], yr[2])
    gate <- tryCatch({
      args <- list(setNames(xr, NULL), setNames(yr, NULL))
      names(args) <- c(x_ch, y_ch)
      do.call(flowCore::rectangleGate, c(args, list(filterId = gname)))
    }, error = function(e) { gate_err(e); NULL })
    req(gate)
    store_gate(gname, "rectangle", gate, x_ch, y_ch, list(x = vx, y = vy))
  }

  # ── Commit: ellipse (inscribed in brushed box) ─────────────────────────────
  commit_ellipse <- function(xr, yr) {
    x_ch <- input$x_ch; y_ch <- input$y_ch
    gname <- unique_name(input$gate_name)
    cx <- mean(xr); cy <- mean(yr)
    rx <- diff(xr) / 2; ry <- diff(yr) / 2
    req(rx > 0, ry > 0)
    gate <- tryCatch({
      cov  <- matrix(c(rx^2, 0, 0, ry^2), nrow = 2,
                     dimnames = list(c(x_ch, y_ch), c(x_ch, y_ch)))
      mu   <- setNames(c(cx, cy), c(x_ch, y_ch))
      flowCore::ellipsoidGate(filterId = gname, .gate = cov, mean = mu, distance = 1)
    }, error = function(e) { gate_err(e); NULL })
    req(gate)
    store_gate(gname, "ellipse", gate, x_ch, y_ch,
               list(cx = cx, cy = cy, rx = rx, ry = ry))
  }

  # ── Commit: boundary / threshold (one click, Inf-bounded rectangle) ────────
  # boundary  = events BELOW the point (-Inf .. x0, -Inf .. y0)
  # threshold = events ABOVE the point (x0 .. Inf, y0 .. Inf)
  commit_corner <- function(kind, x0, y0) {
    one_d <- isTRUE(input$plot_dim == "1d")
    x_ch  <- input$x_ch
    y_ch  <- if (one_d) NULL else input$y_ch
    gname <- unique_name(input$gate_name)
    xr <- if (kind == "boundary") c(-Inf, x0) else c(x0, Inf)
    gate <- tryCatch({
      if (one_d) {
        args <- setNames(list(xr), x_ch)
      } else {
        yr <- if (kind == "boundary") c(-Inf, y0) else c(y0, Inf)
        args <- setNames(list(xr, yr), c(x_ch, y_ch))
      }
      do.call(flowCore::rectangleGate, c(args, list(filterId = gname)))
    }, error = function(e) { gate_err(e); NULL })
    req(gate)
    store_gate(gname, kind, gate, x_ch, y_ch,
               list(x0 = x0, y0 = if (one_d) NULL else y0))
  }

  # ── Commit: quadrant (4 populations from one crosshair) ────────────────────
  commit_quadrant <- function(x0, y0) {
    x_ch <- input$x_ch; y_ch <- input$y_ch
    gname <- unique_name(input$gate_name)
    gate <- tryCatch({
      args <- setNames(list(x0, y0), c(x_ch, y_ch))
      do.call(flowCore::quadGate, c(args, list(filterId = gname)))
    }, error = function(e) { gate_err(e); NULL })
    req(gate)
    store_gate(gname, "quadrant", gate, x_ch, y_ch, list(x0 = x0, y0 = y0))
  }

  # ── Commit: interval (1D) ──────────────────────────────────────────────────
  commit_interval <- function(xr) {
    x_ch <- input$x_ch
    gname <- unique_name(input$gate_name)
    gate <- tryCatch({
      args <- setNames(list(setNames(xr, NULL)), x_ch)
      do.call(flowCore::rectangleGate, c(args, list(filterId = gname)))
    }, error = function(e) { gate_err(e); NULL })
    req(gate)
    store_gate(gname, "interval", gate, x_ch, NULL, list(lo = xr[1], hi = xr[2]))
  }

  # ── Commit: boolean ────────────────────────────────────────────────────────
  commit_boolean <- function() {
    pops <- input$bool_pops; op <- input$bool_op
    if (length(pops) == 0) {
      showNotification("Select at least one population for the boolean gate.",
                       type = "warning", duration = 3)
      return()
    }
    gname <- unique_name(input$gate_name)
    expr_str <- if (op == "!") paste0("!", pops[1])
                else paste(pops, collapse = op)
    gate <- tryCatch(flowCore::char2booleanFilter(expr_str, filterId = gname),
                     error = function(e) {
                       tryCatch(eval(parse(text = sprintf(
                         "flowCore::booleanFilter(%s, filterId='%s')", expr_str, gname))),
                         error = function(e2) { gate_err(e2); NULL })
                     })
    req(gate)
    store_gate(gname, "boolean", gate, NULL, NULL, list(expr = expr_str))
  }

  gate_err <- function(e)
    showNotification(paste("Gate error:", conditionMessage(e)),
                     type = "error", duration = 5)

  # ── Save button dispatches by type ─────────────────────────────────────────
  observeEvent(input$save_btn, {
    gt <- input$gate_type
    if (gt == "boolean") { commit_boolean(); return() }
    if (gt == "polygon") {
      if (length(local$vertices) >= 3) { local$drawing <- FALSE; commit_polygon() }
      else showNotification("Draw a polygon first (≥ 3 vertices).",
                            type = "warning", duration = 3)
      return()
    }
    showNotification("Use 'Start Drawing', then draw on the plot to save this gate type.",
                     type = "message", duration = 4)
  })

  # ── Copy / Paste gate ──────────────────────────────────────────────────────
  selected_gate_name <- function() {
    sel <- input$gate_stats_table_rows_selected
    if (!is.null(sel) && !is.null(local$gate_stats))
      local$gate_stats$Population[sel[1]] else NULL
  }

  observeEvent(input$copy_btn, {
    gname <- selected_gate_name()
    if (is.null(gname) || is.null(local$gates[[gname]])) {
      showNotification("Select a gate in the statistics table to copy.",
                       type = "warning", duration = 3)
      return()
    }
    local$clipboard <- local$gates[[gname]]
    showNotification(paste("Copied gate:", gname), type = "message", duration = 2)
  })

  observeEvent(input$paste_btn, {
    if (is.null(local$clipboard)) {
      showNotification("Nothing to paste — copy a gate first.",
                       type = "warning", duration = 3)
      return()
    }
    g     <- local$clipboard
    gname <- unique_name(paste0(input$gate_name))
    # Re-tag the gate object's filterId so it is unique in the GatingSet
    new_gate <- g$gate
    if (!is.null(new_gate) && "filterId" %in% methods::slotNames(new_gate))
      try(new_gate@filterId <- gname, silent = TRUE)
    local$gates[[gname]] <- list(type = g$type, gate = new_gate,
                                 parent = input$parent_pop %||% "root",
                                 x_ch = g$x_ch, y_ch = g$y_ch, geom = g$geom)
    shared$gate_list <- local$gates
    showNotification(paste("Pasted gate as:", gname), type = "message", duration = 3)
    compute_gate_stats()
  })

  # ── Remove gate ────────────────────────────────────────────────────────────
  observeEvent(input$remove_btn, {
    gname <- selected_gate_name()
    req(gname)
    gs <- local$gating_set
    if (!is.null(gs)) {
      tryCatch(
        safe_cyto(CytoExploreR::cyto_gate_remove(gs, alias = gname,
                                                  gatingTemplate = local$template_path),
                  "cyto_gate_remove failed"),
        error = function(e)
          tryCatch(flowWorkspace::gs_pop_remove(gs, gname), error = function(e2) NULL))
    }
    local$gates[[gname]] <- NULL
    shared$gate_list <- local$gates
    compute_gate_stats()
    showNotification(paste("Gate removed:", gname), type = "message", duration = 2)
  })

  # ── Apply all gates to GatingSet (spans every sample) ──────────────────────
  observeEvent(input$apply_all_btn, {
    fs <- best_fs()
    req(fs, length(local$gates) > 0)
    shared$status <- "busy"

    withProgress(message = "Applying gates to all samples…", value = 0, {
      tryCatch({
        gs <- flowWorkspace::GatingSet(fs)
        incProgress(0.2)

        for (gname in names(local$gates)) {
          g      <- local$gates[[gname]]
          parent <- g$parent %||% "root"
          if (is.null(g$gate)) next
          tryCatch({
            if (identical(g$type, "quadrant")) {
              flowWorkspace::gs_pop_add(
                gs, g$gate, parent = parent,
                names = paste0(gname, c("_++", "_-+", "_--", "_+-")))
            } else {
              flowWorkspace::gs_pop_add(gs, g$gate, parent = parent)
            }
          }, error = function(e)
            showNotification(paste("Skipped gate", gname, ":", conditionMessage(e)),
                             type = "warning", duration = 4))
          incProgress(0.5 / length(local$gates))
        }

        flowWorkspace::recompute(gs)
        incProgress(0.15)

        local$template_path <- file.path(tempdir(), "streamflow_gatingTemplate.csv")
        tryCatch(
          safe_cyto(CytoExploreR::cyto_gatingTemplate_generate(
            gs, gatingTemplate = local$template_path),
            "cyto_gatingTemplate_generate failed"),
          error = function(e)
            tryCatch(CytoExploreR::cyto_gatingTemplate_create(local$template_path),
                     error = function(e2) NULL))
        incProgress(0.15)

        local$gating_set  <- gs
        shared$gating_set <- gs
        compute_gate_stats()
        showNotification(
          sprintf("Gates applied to all %d samples.", length(sampleNames(fs))),
          type = "message", duration = 3)
      }, error = function(e)
        showNotification(paste("GatingSet error:", conditionMessage(e)),
                         type = "error", duration = 6))
    })
    shared$status <- "idle"
  })

  # ── Import gatingTemplate ─────────────────────────────────────────────────
  observeEvent(input$import_template_btn, {
    showNotification("Browse to your gatingTemplate CSV using the button, then click Import Template again.",
                     type = "message", duration = 5)
  })

  observeEvent(input$template_file_picked, {
    sel <- input$template_file_picked
    if (is.list(sel) && identical(sel$error, "no_electron")) {
      showNotification("File selection requires the StreamFLOW desktop app.",
                       type = "warning", duration = 5)
      return()
    }
    template_path <- sel$path
    if (is.null(template_path) || !nzchar(template_path)) return()
    if (!file.exists(template_path) || tolower(tools::file_ext(template_path)) != "csv") {
      showNotification("Please select an existing .csv gatingTemplate file.",
                       type = "error", duration = 5)
      return()
    }
    fs <- best_fs(); req(fs)
    shared$status <- "busy"

    withProgress(message = "Applying gatingTemplate…", value = 0, {
      tryCatch({
        gs <- flowWorkspace::GatingSet(fs)
        incProgress(0.3)
        safe_cyto(CytoExploreR::cyto_gatingTemplate_apply(gs, gatingTemplate = template_path),
                  "cyto_gatingTemplate_apply failed")
        incProgress(0.5)

        local$template_path <- template_path
        local$gating_set    <- gs
        shared$gating_set   <- gs

        nodes <- tryCatch(CytoExploreR::cyto_nodes(gs), error = function(e) character())
        for (nd in nodes) {
          if (!nd %in% names(local$gates))
            local$gates[[nd]] <- list(
              type = "imported", gate = NULL,
              parent = tryCatch(flowWorkspace::gs_pop_get_parent(gs, nd),
                                error = function(e) "root"),
              x_ch = NULL, y_ch = NULL, geom = NULL)
        }
        shared$gate_list <- local$gates
        incProgress(0.2)
        compute_gate_stats()
        showNotification("gatingTemplate applied.", type = "message", duration = 3)
      }, error = function(e)
        showNotification(paste("Template import error:", conditionMessage(e)),
                         type = "error", duration = 6))
    })
    shared$status <- "idle"
  })

  # ── Compute gate statistics (with MFI) ─────────────────────────────────────
  compute_gate_stats <- function() {
    gs <- local$gating_set
    if (!is.null(gs)) {
      tryCatch({
        nodes <- tryCatch(CytoExploreR::cyto_nodes(gs), error = function(e) character())
        rows  <- lapply(nodes, function(nd) {
          n_stats <- flowWorkspace::gs_pop_get_stats(gs, nd, type = "count")
          p_stats <- flowWorkspace::gs_pop_get_stats(gs, nd, type = "percent")
          data.frame(
            Population   = nd,
            Parent       = tryCatch(flowWorkspace::gs_pop_get_parent(gs, nd),
                                    error = function(e) "root"),
            Count        = round(mean(n_stats$count)),
            FreqOfParent = round(mean(p_stats$percent) * 100, 2),
            stringsAsFactors = FALSE)
        })
        df <- if (length(rows) > 0) do.call(rbind, rows) else NULL

        # Attach MFI columns via cyto_stats_compute
        chans <- input$stat_channels
        if (!is.null(df) && !is.null(chans) && length(chans) > 0) {
          mfi <- tryCatch(
            safe_cyto(CytoExploreR::cyto_stats_compute(
              gs, alias = df$Population, channels = chans,
              stat = "median", format = "wide"), "MFI compute failed"),
            error = function(e) NULL)
          df <- merge_mfi(df, mfi, chans)
        }
        local$gate_stats <- df
      }, error = function(e) message("[mod_gating] Stats error: ", conditionMessage(e)))
      return()
    }

    # Fallback: compute counts from raw gates on the current sample
    fs <- best_fs()
    if (is.null(fs) || length(local$gates) == 0) return()
    samp <- input$gate_sample %||% sampleNames(fs)[1]
    if (!samp %in% sampleNames(fs)) return()
    ff <- fs[[samp]]
    n_total <- nrow(exprs(ff))

    rows <- lapply(names(local$gates), function(gname) {
      g <- local$gates[[gname]]
      if (is.null(g$gate) || identical(g$type, "boolean") ||
          identical(g$type, "quadrant")) return(NULL)
      result <- tryCatch(flowCore::filter(ff, g$gate), error = function(e) NULL)
      if (is.null(result)) return(NULL)
      n_in <- tryCatch(sum(result@subSet), error = function(e) NA_integer_)
      data.frame(
        Population   = gname, Parent = g$parent,
        Count        = n_in,
        FreqOfParent = if (!is.na(n_in) && n_total > 0) round(100 * n_in / n_total, 2) else NA,
        stringsAsFactors = FALSE)
    })
    rows <- Filter(Negate(is.null), rows)
    if (length(rows) > 0) local$gate_stats <- do.call(rbind, rows)
  }

  # Best-effort merge of a cyto_stats_compute wide table onto the stats frame
  merge_mfi <- function(df, mfi, chans) {
    if (is.null(mfi)) return(df)
    mfi <- tryCatch(as.data.frame(mfi), error = function(e) NULL)
    if (is.null(mfi) || nrow(mfi) == 0) return(df)
    pop_col <- intersect(c("Population", "alias", "pop", "Population "), names(mfi))
    if (length(pop_col) == 0) {
      # assume first character column identifies the population
      char_cols <- names(mfi)[sapply(mfi, is.character)]
      if (length(char_cols) == 0) return(df)
      pop_col <- char_cols[1]
    }
    keep <- intersect(chans, names(mfi))
    if (length(keep) == 0) return(df)
    tryCatch({
      agg <- aggregate(mfi[keep], by = list(Population = mfi[[pop_col[1]]]),
                       FUN = function(v) round(mean(as.numeric(v), na.rm = TRUE), 1))
      names(agg)[-1] <- paste0("MFI_", names(agg)[-1])
      merge(df, agg, by = "Population", all.x = TRUE)
    }, error = function(e) df)
  }

  observeEvent(input$refresh_tree_btn, {
    compute_gate_stats()
    showNotification("Gate tree refreshed.", type = "message", duration = 2)
  })

  # ── Pop-out current sample into its own window (button or right-click) ──────
  trigger_popout <- function() {
    fs <- best_fs()
    if (is.null(fs)) {
      showNotification("Load samples before opening a pop-out window.",
                       type = "warning", duration = 3)
      return()
    }
    session$sendCustomMessage("streamflow_open_popout", list(
      sample = input$gate_sample %||% "",
      x      = input$x_ch %||% "",
      y      = input$y_ch %||% "",
      dim    = input$plot_dim %||% "2d"))
  }
  observeEvent(input$popout_btn,     trigger_popout())
  observeEvent(input$popout_request, trigger_popout())

  observeEvent(input$stat_channels, { compute_gate_stats() }, ignoreInit = TRUE)

  # ── Statistics table ──────────────────────────────────────────────────────
  output$gate_stats_table <- renderDT({
    df <- local$gate_stats
    if (is.null(df) || nrow(df) == 0)
      return(datatable(
        data.frame(Population = character(), Parent = character(),
                   Count = integer(), `% Parent` = numeric(), check.names = FALSE),
        options = list(dom = "t"), rownames = FALSE))
    dt <- datatable(df, selection = "single", rownames = FALSE,
                    options = list(dom = "tp", pageLength = 12, scrollX = TRUE),
                    class = "compact")
    if ("FreqOfParent" %in% names(df)) dt <- dt %>% formatRound("FreqOfParent", 2)
    if ("Count" %in% names(df))
      dt <- dt %>% formatCurrency("Count", currency = "", interval = 3,
                                  mark = ",", digits = 0)
    mfi_cols <- grep("^MFI_", names(df), value = TRUE)
    if (length(mfi_cols) > 0) dt <- dt %>% formatRound(mfi_cols, 1)
    dt
  })

  # ── cyto_plot_gating_tree (interactive D3) ────────────────────────────────
  output$gating_tree_sample_ui <- renderUI({
    gs <- local$gating_set; req(gs)
    sn <- tryCatch(CytoExploreR::cyto_names(gs), error = function(e) sampleNames(gs))
    selectInput(ns("tree_sample"), "Sample for Tree", choices = sn, selected = sn[1])
  })

  output$cyto_gating_tree <- renderUI({
    gs <- local$gating_set; samp <- input$tree_sample
    req(gs, samp)
    tryCatch({
      gh <- gs[[which(sampleNames(gs) == samp)[1]]]
      widget <- safe_cyto(CytoExploreR::cyto_plot_gating_tree(gh, stat = "freq"),
                          "cyto_plot_gating_tree failed")
      if (!is.null(widget)) widget
      else tags$p(style = "color:#5A7A8A;font-size:12px;",
                  "Apply gates first to see gating tree.")
    }, error = function(e)
      tags$p(style = "color:#C0392B;font-size:12px;",
             paste("Tree error:", conditionMessage(e))))
  })

  # ── Download gatingTemplate CSV ───────────────────────────────────────────
  output$dl_template <- downloadHandler(
    filename = function() paste0("gatingTemplate_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".csv"),
    content  = function(file) {
      gs <- local$gating_set
      if (!is.null(gs) && !is.null(local$template_path) &&
          file.exists(local$template_path)) {
        file.copy(local$template_path, file)
      } else if (!is.null(gs)) {
        tryCatch(
          safe_cyto(CytoExploreR::cyto_gatingTemplate_generate(gs, gatingTemplate = file),
                    "cyto_gatingTemplate_generate failed"),
          error = function(e) writeLines("# No gatingTemplate available", file))
      } else {
        writeLines("# No gates applied", file)
      }
    }
  )

  # ── UI helpers ────────────────────────────────────────────────────────────
  output$drawing_instructions_ui <- renderUI({
    if (local$drawing)
      tags$div(style = "font-size:11px;color:#F39C12;padding:6px;background:#243447;border-radius:4px;margin-bottom:6px;",
               icon("pencil-alt"), " Drawing mode active.")
    else
      tags$div(style = "font-size:11px;color:#8899AA;padding:6px;background:#243447;border-radius:4px;margin-bottom:6px;",
               icon("info-circle"), " Pick a gate type, click 'Start Drawing', then draw on the plot.")
  })

  output$vertex_status_ui <- renderUI({
    if (local$drawing && length(local$vertices) > 0)
      tags$p(style = "font-size:11px;color:#F39C12;margin-top:4px;",
             sprintf("⬡ %d vertices placed", length(local$vertices)))
  })
}
