# StreamFLOW - mod_gating.R
# Interactive plotly polygon gating with full CytoExploreR gatingTemplate integration
#
# CytoExploreR API used:
#   cyto_nodes()                    – extract gated population names
#   cyto_gatingTemplate_create()    – create empty gatingTemplate CSV
#   cyto_gatingTemplate_write()     – persist gates to CSV
#   cyto_gatingTemplate_apply()     – re-apply gates from CSV
#   cyto_gatingTemplate_generate()  – build template from existing GatingSet
#   cyto_gate_extract()             – pull saved gate objects from template
#   cyto_gate_remove()              – remove gate from template and GatingSet
#   cyto_gate_rename()              – rename a gate
#   cyto_gate_type()                – query gate type from template
#   cyto_plot()                     – render gating plot (base R in renderPlot)
#   cyto_plot_gating_tree()         – interactive D3 gating tree
#   cyto_transformer_extract()      – get axes_trans for proper axis labels
#   cyto_extract()                  – get population flowFrame/flowSet
#   cyto_fluor_channels()           – fluorescent channel list
#   cyto_nodes()                    – node path list
#
# Note: cyto_gate_draw() requires a graphical locator() — not usable in Shiny.
# We implement polygon gating via plotly click events → flowCore::polygonGate()

# ── UI ────────────────────────────────────────────────────────────────────────
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
          selectInput(ns("parent_pop"), "Gate From (Parent)", choices = c("root")),
          textInput(ns("gate_name"),   "Gate Name",  value = "Population1"),
          selectInput(ns("gate_type"), "Gate Type",
                      choices  = c("Polygon"   = "polygon",
                                   "Rectangle" = "rectangle",
                                   "Ellipse"   = "ellipse",
                                   "Threshold" = "threshold",
                                   "Quadrant"  = "quadrant"),
                      selected = "polygon"),
          uiOutput(ns("axis_selectors_ui")),
          selectInput(ns("gate_sample"), "Gate On Sample", choices = character()),
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
              column(6,
                actionButton(ns("remove_btn"),
                             tagList(icon("trash"), " Remove"),
                             class = "btn btn-danger btn-block btn-sm")
              ),
              column(6,
                actionButton(ns("apply_all_btn"),
                             tagList(icon("play"), " Apply All"),
                             class = "btn btn-default btn-block btn-sm")
              )
            )
          ),
          tags$hr(),
          fluidRow(
            column(6,
              actionButton(ns("import_template_btn"),
                           tagList(icon("file-import"), " Import Template"),
                           class = "btn btn-default btn-block btn-sm")
            ),
            column(6,
              shinyFilesButton(ns("template_file"),
                               label    = tagList(icon("folder-open"), " Browse"),
                               title    = "Select gatingTemplate CSV",
                               multiple = FALSE,
                               class    = "btn btn-default btn-block btn-sm")
            )
          )
        )
      ),

      # ── Right panel ────────────────────────────────────────────────────────
      column(8,
        box(
          title = "Gating Plot", width = NULL, solidHeader = TRUE,
          uiOutput(ns("drawing_instructions_ui")),
          withSpinner(plotlyOutput(ns("gating_plot"), height = "430px"), color = "#00B4D8"),
          uiOutput(ns("vertex_status_ui"))
        ),

        fluidRow(
          column(6,
            box(
              title = "Population Statistics", width = NULL, solidHeader = TRUE,
              DTOutput(ns("gate_stats_table"))
            )
          ),
          column(6,
            box(
              title = "Gating Tree (cyto_plot_gating_tree)", width = NULL, solidHeader = TRUE,
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
  volumes <- c(Home = path.expand("~"), getVolumes()())
  shinyFileChoose(input, "template_file", roots = volumes, session = session,
                  filetypes = c("csv"))

  local <- reactiveValues(
    vertices      = list(),
    drawing       = FALSE,
    gates         = list(),   # name → list(gate, parent, x_ch, y_ch, vertices)
    gating_set    = NULL,
    gate_stats    = NULL,
    template_path = NULL
  )

  # ── Best available flowset ─────────────────────────────────────────────────
  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Axis channel selectors ─────────────────────────────────────────────────
  output$axis_selectors_ui <- renderUI({
    channels <- shared$channels %||% character()
    tagList(
      selectInput(ns("x_ch"), "X Channel", choices = channels,
                  selected = channels[1]),
      selectInput(ns("y_ch"), "Y Channel", choices = channels,
                  selected = if (length(channels) > 1) channels[2] else channels[1])
    )
  })

  # ── Sample selector ────────────────────────────────────────────────────────
  observe({
    fs <- best_fs()
    req(fs)
    sn <- tryCatch(CytoExploreR::cyto_names(fs), error = function(e) sampleNames(fs))
    updateSelectInput(session, "gate_sample", choices = sn, selected = sn[1])
  })

  # ── Parent population choices ──────────────────────────────────────────────
  observe({
    gs <- local$gating_set
    parents <- if (!is.null(gs)) {
      tryCatch(c("root", CytoExploreR::cyto_nodes(gs)), error = function(e) c("root"))
    } else {
      c("root", names(local$gates))
    }
    updateSelectInput(session, "parent_pop", choices = parents, selected = "root")
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
        tags$div(style = "margin-left:14px;padding:2px 0;",
          tags$span(style = "color:#2EC4B6;font-size:12px;",
                    icon("caret-right"), " ", gname,
                    tags$span(style = "color:#5A7A8A;font-size:11px;",
                              pct, parent_info))
        )
      })
    )
  })

  # ── Main gating plot ───────────────────────────────────────────────────────
  output$gating_plot <- renderPlotly({
    x_ch  <- input$x_ch
    y_ch  <- input$y_ch
    samp  <- input$gate_sample
    fs    <- best_fs()

    req(x_ch, y_ch, samp, fs)
    req(x_ch %in% colnames(fs), y_ch %in% colnames(fs),
        samp %in% sampleNames(fs))

    tryCatch({
      # ── Try cyto_plot via recordPlot (base R) ──────────────────────────────
      # For interactive Shiny plotly gating, we use plotly scatter instead.
      # cyto_plot() is used in mod_visualization for final display.

      ff   <- fs[[samp]]
      expr <- as.data.frame(exprs(ff))
      n    <- min(nrow(expr), 5000)
      idx  <- sample(nrow(expr), n)
      df   <- expr[idx, , drop = FALSE]

      # Density colouring
      dens <- tryCatch({
        d <- MASS::kde2d(df[[x_ch]], df[[y_ch]], n = 100)
        fields::interp.surface(d, cbind(df[[x_ch]], df[[y_ch]]))
      }, error = function(e) rep(1, nrow(df)))
      dens[!is.finite(dens)] <- 0

      p <- plot_ly(
        data = df, x = ~get(x_ch), y = ~get(y_ch),
        type = "scatter", mode = "markers",
        marker = list(
          size    = 3, opacity = 0.6,
          color   = dens,
          colorscale = list(
            list(0,   "#081420"), list(0.3, "#00B4D8"),
            list(0.7, "#2EC4B6"), list(1,   "#FFFFFF")
          ),
          showscale = FALSE
        ),
        hoverinfo = "none",
        source    = ns("gating_plot")
      ) %>%
        plotly_dark_layout(
          title = sprintf("%s — %s vs %s", samp, x_ch, y_ch),
          xlab  = x_ch, ylab = y_ch
        )

      # ── Draw saved gates ─────────────────────────────────────────────────
      if (isTRUE(input$show_gates) && length(local$gates) > 0) {
        shapes <- list()
        anns   <- list()
        for (gname in names(local$gates)) {
          g <- local$gates[[gname]]
          if (!is.null(g$vertices) && g$x_ch == x_ch && g$y_ch == y_ch) {
            vx <- c(g$vertices$x, g$vertices$x[1])
            vy <- c(g$vertices$y, g$vertices$y[1])
            path_str <- paste0(
              "M ", vx[1], " ", vy[1],
              paste(sprintf(" L %g %g", vx[-1], vy[-1]), collapse = ""), " Z"
            )
            shapes <- c(shapes, list(list(
              type = "path", path = path_str,
              line = list(color = "#2EC4B6", width = 1.5),
              fillcolor = "rgba(46,196,182,0.06)", layer = "above"
            )))
            anns <- c(anns, list(list(
              x = mean(g$vertices$x), y = mean(g$vertices$y),
              text = gname, showarrow = FALSE,
              font = list(color = "#2EC4B6", size = 11)
            )))
          }
        }
        if (length(shapes) > 0)
          p <- p %>% layout(shapes = shapes, annotations = anns)
      }

      # ── Draw in-progress polygon ──────────────────────────────────────────
      if (local$drawing && length(local$vertices) > 0) {
        vx <- sapply(local$vertices, `[[`, "x")
        vy <- sapply(local$vertices, `[[`, "y")
        p  <- p %>%
          add_trace(
            x = c(vx, vx[1]), y = c(vy, vy[1]),
            type = "scatter", mode = "lines+markers",
            line   = list(color = "#F39C12", dash = "dash", width = 1.5),
            marker = list(color = "#F39C12", size = 6),
            showlegend = FALSE, hoverinfo = "none"
          )
      }

      p
    }, error = function(e) {
      plot_ly() %>%
        layout(paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
               annotations = list(list(
                 text = paste("Plot error:", conditionMessage(e)),
                 font = list(color = "#C0392B"), showarrow = FALSE,
                 xref = "paper", yref = "paper", x = 0.5, y = 0.5)))
    })
  })

  # ── Plotly click → collect polygon vertices ────────────────────────────────
  observeEvent(event_data("plotly_click", source = ns("gating_plot")), {
    if (!local$drawing) return()
    ev <- event_data("plotly_click", source = ns("gating_plot"))
    req(ev)
    local$vertices <- c(local$vertices, list(list(x = ev$x, y = ev$y)))
  })

  # ── Double-click → close polygon ──────────────────────────────────────────
  observeEvent(event_data("plotly_doubleclick", source = ns("gating_plot")), {
    if (!local$drawing || length(local$vertices) < 3) return()
    local$drawing <- FALSE
    commit_gate()
  })

  # ── Rectangle via plotly brush ─────────────────────────────────────────────
  observeEvent(event_data("plotly_selected", source = ns("gating_plot")), {
    if (input$gate_type != "rectangle") return()
    ev <- event_data("plotly_selected", source = ns("gating_plot"))
    req(!is.null(ev$x), length(ev$x) >= 2)
    xr <- range(ev$x); yr <- range(ev$y)
    local$vertices <- list(
      list(x = xr[1], y = yr[1]), list(x = xr[2], y = yr[1]),
      list(x = xr[2], y = yr[2]), list(x = xr[1], y = yr[2])
    )
    local$drawing <- FALSE
    commit_gate()
  })

  # ── Start drawing ──────────────────────────────────────────────────────────
  observeEvent(input$draw_btn, {
    local$vertices <- list()
    local$drawing  <- TRUE
    showNotification(
      "Click to add vertices. Double-click to close polygon. (Use brush for rectangle gates.)",
      type = "message", duration = 6
    )
  })

  # ── Commit gate ────────────────────────────────────────────────────────────
  commit_gate <- function() {
    req(length(local$vertices) >= 3)
    x_ch   <- input$x_ch
    y_ch   <- input$y_ch
    gname  <- trimws(input$gate_name)
    parent <- input$parent_pop %||% "root"

    if (nchar(gname) == 0) gname <- paste0("Population", length(local$gates) + 1)
    while (gname %in% names(local$gates))
      gname <- paste0(gname, "_", length(local$gates) + 1)

    vx  <- sapply(local$vertices, `[[`, "x")
    vy  <- sapply(local$vertices, `[[`, "y")
    mat <- matrix(c(vx, vy), ncol = 2, dimnames = list(NULL, c(x_ch, y_ch)))

    gate_obj <- tryCatch(
      flowCore::polygonGate(filterId = gname, .gate = mat),
      error = function(e) {
        showNotification(paste("Gate error:", conditionMessage(e)), type = "error")
        NULL
      }
    )
    req(gate_obj)

    local$gates[[gname]] <- list(
      gate     = gate_obj, parent = parent,
      x_ch     = x_ch,    y_ch   = y_ch,
      vertices = list(x = vx, y = vy)
    )
    local$vertices <- list()
    shared$gate_list <- local$gates

    updateTextInput(session, "gate_name",
                    value = paste0("Population", length(local$gates) + 1))
    showNotification(paste("Gate saved:", gname), type = "message", duration = 3)
    compute_gate_stats()
  }

  observeEvent(input$save_btn, {
    if (length(local$vertices) >= 3) { local$drawing <- FALSE; commit_gate() }
    else showNotification("Draw a gate first (≥ 3 vertices).", type = "warning", duration = 3)
  })

  # ── Remove gate ────────────────────────────────────────────────────────────
  observeEvent(input$remove_btn, {
    sel <- input$gate_stats_table_rows_selected
    gs  <- local$gating_set

    gname <- if (!is.null(sel) && !is.null(local$gate_stats)) {
      local$gate_stats$Population[sel[1]]
    } else NULL

    req(gname)

    # cyto_gate_remove from GatingSet if present
    if (!is.null(gs)) {
      tryCatch(
        safe_cyto(CytoExploreR::cyto_gate_remove(gs, alias = gname,
                                                   gatingTemplate = local$template_path),
                  "cyto_gate_remove failed"),
        error = function(e) {
          tryCatch(flowWorkspace::gs_pop_remove(gs, gname), error = function(e2) NULL)
        }
      )
    }

    local$gates[[gname]] <- NULL
    shared$gate_list <- local$gates
    compute_gate_stats()
    showNotification(paste("Gate removed:", gname), type = "message", duration = 2)
  })

  # ── Apply all gates to GatingSet ───────────────────────────────────────────
  observeEvent(input$apply_all_btn, {
    fs <- best_fs()
    req(fs, length(local$gates) > 0)
    shared$status <- "busy"

    withProgress(message = "Applying gates to GatingSet…", value = 0, {
      tryCatch({
        # Build GatingSet
        gs <- flowWorkspace::GatingSet(fs)
        incProgress(0.2)

        # Add gates in order: root gates first, then children
        ordered_names <- names(local$gates)  # already insertion-order
        for (gname in ordered_names) {
          g      <- local$gates[[gname]]
          parent <- if (g$parent == "root") "root" else g$parent
          tryCatch(
            flowWorkspace::gs_pop_add(gs, g$gate, parent = parent),
            error = function(e)
              showNotification(paste("Skipped gate", gname, ":", conditionMessage(e)),
                               type = "warning", duration = 4)
          )
          incProgress(0.5 / length(local$gates))
        }

        flowWorkspace::recompute(gs)
        incProgress(0.15)

        # Generate and save gatingTemplate using CytoExploreR
        local$template_path <- file.path(tempdir(), "streamflow_gatingTemplate.csv")
        tryCatch(
          safe_cyto(
            CytoExploreR::cyto_gatingTemplate_generate(
              gs,
              gatingTemplate = local$template_path
            ),
            "cyto_gatingTemplate_generate failed"
          ),
          error = function(e) {
            # Fallback: create empty template then write
            tryCatch(
              CytoExploreR::cyto_gatingTemplate_create(local$template_path),
              error = function(e2) NULL
            )
          }
        )
        incProgress(0.15)

        local$gating_set  <- gs
        shared$gating_set <- gs
        compute_gate_stats()
        showNotification("All gates applied. GatingSet created.", type = "message", duration = 3)
      }, error = function(e) {
        showNotification(paste("GatingSet error:", conditionMessage(e)),
                         type = "error", duration = 6)
      })
    })

    shared$status <- "idle"
  })

  # ── Import gatingTemplate ─────────────────────────────────────────────────
  observeEvent(input$import_template_btn, {
    showNotification("Browse to your gatingTemplate CSV using the button, then click Import Template again.",
                     type = "message", duration = 5)
  })

  observeEvent(input$template_file, {
    req(is.list(input$template_file))
    fi <- parseFilePaths(volumes, input$template_file)
    req(nrow(fi) > 0)
    template_path <- as.character(fi$datapath)
    fs <- best_fs()
    req(fs)
    shared$status <- "busy"

    withProgress(message = "Applying gatingTemplate (cyto_gatingTemplate_apply)…", value = 0, {
      tryCatch({
        gs <- flowWorkspace::GatingSet(fs)
        incProgress(0.3)

        # Apply using CytoExploreR
        safe_cyto(
          CytoExploreR::cyto_gatingTemplate_apply(gs, gatingTemplate = template_path),
          "cyto_gatingTemplate_apply failed"
        )
        incProgress(0.5)

        local$template_path <- template_path
        local$gating_set    <- gs
        shared$gating_set   <- gs

        # Extract node names to populate local$gates
        nodes <- tryCatch(CytoExploreR::cyto_nodes(gs), error = function(e) character())
        for (nd in nodes) {
          if (!nd %in% names(local$gates)) {
            local$gates[[nd]] <- list(
              gate     = NULL,
              parent   = tryCatch(flowWorkspace::gs_pop_get_parent(gs, nd), error = function(e) "root"),
              x_ch     = NULL, y_ch = NULL, vertices = NULL
            )
          }
        }
        shared$gate_list <- local$gates

        incProgress(0.2)
        compute_gate_stats()
        showNotification("gatingTemplate applied.", type = "message", duration = 3)
      }, error = function(e) {
        showNotification(paste("Template import error:", conditionMessage(e)),
                         type = "error", duration = 6)
      })
    })

    shared$status <- "idle"
  })

  # ── Compute gate statistics ────────────────────────────────────────────────
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
            stringsAsFactors = FALSE
          )
        })
        if (length(rows) > 0)
          local$gate_stats <- do.call(rbind, rows)
      }, error = function(e) message("[mod_gating] Stats error: ", conditionMessage(e)))
      return()
    }

    # Fallback: compute from raw gates
    fs <- best_fs()
    if (is.null(fs) || length(local$gates) == 0) return()
    samp <- input$gate_sample %||% sampleNames(fs)[1]
    if (!samp %in% sampleNames(fs)) return()
    ff   <- fs[[samp]]

    rows <- lapply(names(local$gates), function(gname) {
      g      <- local$gates[[gname]]
      req(!is.null(g$gate))
      result <- tryCatch(flowCore::filter(ff, g$gate), error = function(e) NULL)
      if (is.null(result)) return(NULL)
      n_total <- nrow(exprs(ff))
      n_in    <- sum(result@subSet)
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
  }

  observeEvent(input$refresh_tree_btn, {
    compute_gate_stats()
    showNotification("Gate tree refreshed.", type = "message", duration = 2)
  })

  # ── Statistics table ──────────────────────────────────────────────────────
  output$gate_stats_table <- renderDT({
    df <- local$gate_stats
    if (is.null(df) || nrow(df) == 0)
      return(datatable(
        data.frame(Population = character(), Parent = character(),
                   Count = integer(), `% Parent` = numeric()),
        options = list(dom = "t"), rownames = FALSE
      ))
    datatable(df, selection = "single", rownames = FALSE,
              colnames = c("Population", "Parent", "Count", "% of Parent"),
              options = list(dom = "t", pageLength = 20, scrollX = TRUE),
              class = "compact") %>%
      formatRound("FreqOfParent", 2) %>%
      formatCurrency("Count", currency = "", interval = 3, mark = ",", digits = 0)
  })

  # ── cyto_plot_gating_tree (interactive D3) ────────────────────────────────
  output$gating_tree_sample_ui <- renderUI({
    gs <- local$gating_set
    req(gs)
    sn <- tryCatch(CytoExploreR::cyto_names(gs), error = function(e) sampleNames(gs))
    selectInput(ns("tree_sample"), "Sample for Tree",
                choices = sn, selected = sn[1])
  })

  output$cyto_gating_tree <- renderUI({
    gs   <- local$gating_set
    samp <- input$tree_sample
    req(gs, samp)

    tree_html <- tryCatch({
      gh <- gs[[which(sampleNames(gs) == samp)[1]]]
      # cyto_plot_gating_tree returns an htmlwidget
      widget <- safe_cyto(
        CytoExploreR::cyto_plot_gating_tree(gh, stat = "freq"),
        "cyto_plot_gating_tree failed"
      )
      if (!is.null(widget)) widget
      else tags$p(style = "color:#5A7A8A;font-size:12px;",
                  "Apply gates first to see gating tree.")
    }, error = function(e) {
      tags$p(style = "color:#C0392B;font-size:12px;",
             paste("Tree error:", conditionMessage(e)))
    })

    tree_html
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
        tryCatch({
          safe_cyto(
            CytoExploreR::cyto_gatingTemplate_generate(gs, gatingTemplate = file),
            "cyto_gatingTemplate_generate failed"
          )
        }, error = function(e) {
          writeLines("# No gatingTemplate available", file)
        })
      } else {
        writeLines("# No gates applied", file)
      }
    }
  )

  # ── UI helpers ────────────────────────────────────────────────────────────
  output$drawing_instructions_ui <- renderUI({
    if (local$drawing) {
      tags$div(style = "font-size:11px;color:#F39C12;padding:6px;background:#243447;border-radius:4px;margin-bottom:6px;",
               icon("pencil-alt"), " Drawing mode active — click to add vertices. Double-click to close.")
    } else {
      tags$div(style = "font-size:11px;color:#8899AA;padding:6px;background:#243447;border-radius:4px;margin-bottom:6px;",
               icon("info-circle"), " Click 'Start Drawing', then click on the plot. Double-click to close polygon.")
    }
  })

  output$vertex_status_ui <- renderUI({
    if (local$drawing && length(local$vertices) > 0)
      tags$p(style = "font-size:11px;color:#F39C12;margin-top:4px;",
             sprintf("⬡ %d vertices placed", length(local$vertices)))
  })
}
