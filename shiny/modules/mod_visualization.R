# StreamFLOW - mod_visualization.R
# Full CytoExploreR visualization using cyto_plot() family
#
# CytoExploreR API used:
#   cyto_plot()                  – scatter, density, histogram (base R, renderPlot)
#   cyto_plot_profile()          – 1D density distributions in all channels
#   cyto_plot_explore()          – 2D scatter in all channel combinations
#   cyto_plot_gating_scheme()    – full multi-level gating strategy
#   cyto_plot_gating_tree()      – interactive D3 gating tree
#   cyto_plot_theme()            – set dark theme for all cyto_plot calls
#   cyto_plot_theme_reset()      – reset theme
#   cyto_plot_save()             – high-resolution export (PNG/PDF/TIFF/SVG)
#   cyto_plot_complete()         – signal end of save operation
#   cyto_transformer_extract()   – get axes_trans for proper axis display
#   cyto_extract()               – extract population data for overlay
#   cyto_fluor_channels()        – fluorescent channels
#   cyto_nodes()                 – gated population list
#   cyto_names()                 – sample names

# ── UI ────────────────────────────────────────────────────────────────────────
visualizationUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Controls
      column(3,
        box(
          title = "Plot Controls", width = NULL, solidHeader = TRUE,

          selectInput(ns("plot_tab"), "View",
                      choices = c("cyto_plot (Scatter/Density)"   = "cyto",
                                  "cyto_plot_profile (1D All Ch.)"= "profile",
                                  "cyto_plot_explore (All 2D)"    = "explore",
                                  "Gating Scheme"                 = "scheme",
                                  "Gating Tree"                   = "tree"),
                      selected = "cyto"),

          uiOutput(ns("channel_controls_ui")),

          uiOutput(ns("population_ui")),

          uiOutput(ns("sample_selector_ui")),

          # Only shown for cyto_plot
          conditionalPanel(
            condition = sprintf("input['%s'] == 'cyto'", ns("plot_tab")),
            checkboxInput(ns("show_gates"),    "Show Gates",     value = TRUE),
            checkboxInput(ns("back_gating"),   "Back-Gating",    value = FALSE),
            checkboxInput(ns("gate_tracking"), "Gate Tracking",  value = FALSE),
            numericInput( ns("display"),       "Max Events Displayed",
                          value = 25000, min = 100, max = 200000, step = 1000),
            sliderInput(  ns("contour_lines"), "Contour Lines",
                          0, 30, 0, 1, ticks = FALSE),
            sliderInput(  ns("pt_size"),       "Point Size",
                          0.5, 5, 2, 0.5, ticks = FALSE)
          ),

          # Only shown for profile
          conditionalPanel(
            condition = sprintf("input['%s'] == 'profile'", ns("plot_tab")),
            sliderInput(ns("density_smooth"), "Smoothness",    0.1, 2, 0.6, 0.1, ticks = FALSE),
            sliderInput(ns("density_stack"),  "Stack Offset",  0,   1, 0.4, 0.1, ticks = FALSE),
            numericInput(ns("density_layers"),"Samples / Page", 4, 1, 32, 1)
          ),

          tags$hr(),
          selectInput(ns("export_format"), "Export Format",
                      choices = c("PNG" = "png", "PDF" = "pdf",
                                  "TIFF" = "tiff", "SVG" = "svg")),
          numericInput(ns("export_width"),  "Width (in)",  value = 10, min = 2, max = 24, step = 0.5),
          numericInput(ns("export_height"), "Height (in)", value = 8,  min = 2, max = 24, step = 0.5),
          downloadButton(ns("export_plot"), "Export Plot",
                         class = "btn btn-default btn-block btn-sm")
        )
      ),

      # Plot area
      column(9,
        box(
          title = "Visualisation", width = NULL, solidHeader = TRUE,

          # cyto_plot / profile / explore / scheme → base R renderPlot
          conditionalPanel(
            condition = sprintf("['cyto','profile','explore','scheme'].indexOf(input['%s']) >= 0",
                                ns("plot_tab")),
            withSpinner(plotOutput(ns("main_cyto_plot"), height = "530px"),
                        color = "#00B4D8")
          ),

          # Gating tree → htmlwidget
          conditionalPanel(
            condition = sprintf("input['%s'] == 'tree'", ns("plot_tab")),
            uiOutput(ns("tree_sample_ui")),
            withSpinner(
              uiOutput(ns("gating_tree_widget")),
              color = "#00B4D8"
            )
          )
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
visualizationServer <- function(input, output, session, shared) {
  ns <- session$ns

  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Dynamic selectors ─────────────────────────────────────────────────────
  output$channel_controls_ui <- renderUI({
    channels <- shared$channels %||% character()
    tagList(
      selectInput(ns("x_ch"), "X Channel", choices = channels, selected = channels[1]),
      conditionalPanel(
        condition = sprintf("input['%s'] == 'cyto'", ns("plot_tab")),
        selectInput(ns("y_ch"), "Y Channel", choices = channels,
                    selected = if (length(channels) > 1) channels[2] else channels[1])
      )
    )
  })

  output$population_ui <- renderUI({
    gs   <- shared$gating_set
    pops <- if (!is.null(gs)) {
      tryCatch(c("root", CytoExploreR::cyto_nodes(gs)), error = function(e) "root")
    } else "root"
    selectInput(ns("population"), "Population", choices = pops, selected = "root")
  })

  output$sample_selector_ui <- renderUI({
    fs <- best_fs()
    req(fs)
    sn <- tryCatch(CytoExploreR::cyto_names(fs), error = function(e) sampleNames(fs))
    selectInput(ns("samples"), "Samples", choices = sn,
                selected = sn[1], multiple = TRUE)
  })

  output$tree_sample_ui <- renderUI({
    gs <- shared$gating_set
    req(gs)
    sn <- tryCatch(CytoExploreR::cyto_names(gs), error = function(e) sampleNames(gs))
    selectInput(ns("tree_sample"), "Sample for Tree", choices = sn, selected = sn[1])
  })

  # ── Apply cyto_plot_theme for dark look ───────────────────────────────────
  # Called before each plot render; reset after.
  apply_dark_theme <- function() {
    tryCatch(
      safe_cyto(
        CytoExploreR::cyto_plot_theme(
          border_fill     = "#1B2A3B",
          border_line_col = "#243447",
          title_text_col  = "#00B4D8",
          axes_text_col   = "#B0C4D8",
          axes_label_text_col = "#E0E0E0"
        ),
        "cyto_plot_theme failed"
      ),
      error = function(e) NULL
    )
  }

  reset_theme <- function() {
    tryCatch(
      safe_cyto(CytoExploreR::cyto_plot_theme_reset(), "cyto_plot_theme_reset failed"),
      error = function(e) NULL
    )
  }

  # ── Get axes_trans from GatingSet ─────────────────────────────────────────
  get_axes_trans <- function() {
    gs <- shared$gating_set
    if (is.null(gs)) return(NULL)
    tryCatch(
      safe_cyto(CytoExploreR::cyto_transformer_extract(gs),
                "cyto_transformer_extract failed"),
      error = function(e) NULL
    )
  }

  # ── Main base-R plot (cyto_plot, profile, explore, scheme) ─────────────────
  output$main_cyto_plot <- renderPlot({
    par(bg = "#0D1B2A", col.axis = "#E0E0E0", col.lab = "#E0E0E0",
        col.main = "#00B4D8", fg = "#E0E0E0")

    view     <- input$plot_tab  %||% "cyto"
    x_ch     <- input$x_ch
    y_ch     <- input$y_ch
    samples  <- input$samples
    pop      <- input$population %||% "root"
    gs       <- shared$gating_set
    fs       <- best_fs()
    req(fs)

    apply_dark_theme()
    on.exit(reset_theme(), add = TRUE)

    axes_trans <- get_axes_trans()

    tryCatch({
      if (view == "cyto") {
        # ── cyto_plot ──────────────────────────────────────────────────────
        req(x_ch)
        samp_idx <- if (!is.null(samples) && length(samples) > 0) {
          which(sampleNames(fs) %in% samples)
        } else 1L

        if (!is.null(gs)) {
          CytoExploreR::cyto_plot(
            gs[samp_idx],
            parent         = pop,
            channels       = c(x_ch, y_ch),
            display        = input$display  %||% 25000,
            contour_lines  = input$contour_lines %||% 0,
            point_size     = input$pt_size  %||% 2,
            gate_track     = isTRUE(input$gate_tracking),
            back_gating    = isTRUE(input$back_gating),
            axes_trans     = axes_trans
          )
        } else {
          # Fallback: flowSet plot
          sub_fs <- if (length(samp_idx) > 0) fs[samp_idx] else fs[1]
          CytoExploreR::cyto_plot(
            sub_fs,
            channels       = c(x_ch, y_ch),
            display        = input$display %||% 25000,
            contour_lines  = input$contour_lines %||% 0,
            point_size     = input$pt_size %||% 2,
            axes_trans     = axes_trans
          )
        }

      } else if (view == "profile") {
        # ── cyto_plot_profile ─────────────────────────────────────────────
        fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),
                             error = function(e) shared$channels)
        req(fluor_ch)

        samp_sel <- if (!is.null(samples) && length(samples) > 0) {
          which(sampleNames(fs) %in% samples)
        } else seq_len(min(8, length(fs)))

        obj <- if (!is.null(gs)) gs[samp_sel] else fs[samp_sel]

        CytoExploreR::cyto_plot_profile(
          obj,
          parent         = if (!is.null(gs)) pop else NULL,
          channels       = fluor_ch,
          density_smooth = input$density_smooth %||% 0.6,
          density_stack  = input$density_stack  %||% 0.4,
          density_layers = as.integer(input$density_layers %||% 4),
          axes_trans     = axes_trans
        )

      } else if (view == "explore") {
        # ── cyto_plot_explore ─────────────────────────────────────────────
        fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),
                             error = function(e) shared$channels)
        req(fluor_ch)

        samp_1 <- if (!is.null(samples) && length(samples) > 0)
          which(sampleNames(fs) %in% samples[1])
        else 1L

        obj <- if (!is.null(gs)) gs[[samp_1]] else fs[[samp_1]]

        CytoExploreR::cyto_plot_explore(
          obj,
          channels   = fluor_ch,
          axes_trans = axes_trans,
          parent     = if (!is.null(gs)) pop else NULL
        )

      } else if (view == "scheme") {
        # ── cyto_plot_gating_scheme ───────────────────────────────────────
        req(gs)
        samp_idx <- if (!is.null(samples) && length(samples) > 0)
          which(sampleNames(gs) %in% samples[1])
        else 1L

        CytoExploreR::cyto_plot_gating_scheme(
          gs[samp_idx],
          back_gate   = isTRUE(input$back_gating),
          gate_track  = isTRUE(input$gate_tracking)
        )
      }
    }, error = function(e) {
      plot.new()
      text(0.5, 0.5, paste(view, "error:\n", conditionMessage(e)),
           col = "#C0392B", cex = 0.9, adj = c(0.5, 0.5))
    })
  }, bg = "#0D1B2A")

  # ── Gating tree widget ────────────────────────────────────────────────────
  output$gating_tree_widget <- renderUI({
    gs   <- shared$gating_set
    samp <- input$tree_sample
    req(gs, samp)

    tryCatch({
      gh     <- gs[[which(sampleNames(gs) == samp)[1]]]
      widget <- safe_cyto(
        CytoExploreR::cyto_plot_gating_tree(gh, stat = "freq"),
        "cyto_plot_gating_tree failed"
      )
      if (!is.null(widget)) widget
      else tags$p(style = "color:#5A7A8A;font-size:12px;padding:20px;",
                  "Apply gates first to see the gating tree.")
    }, error = function(e) {
      tags$p(style = "color:#C0392B;font-size:12px;",
             paste("Tree error:", conditionMessage(e)))
    })
  })

  # ── Export ────────────────────────────────────────────────────────────────
  output$export_plot <- downloadHandler(
    filename = function() {
      fmt <- input$export_format %||% "png"
      paste0("StreamFLOW_", input$plot_tab, "_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".", fmt)
    },
    content = function(file) {
      fs  <- best_fs()
      gs  <- shared$gating_set
      req(fs)

      view    <- input$plot_tab  %||% "cyto"
      fmt     <- input$export_format %||% "png"
      w       <- input$export_width  %||% 10
      h       <- input$export_height %||% 8
      pop     <- input$population    %||% "root"
      samples <- input$samples
      x_ch    <- input$x_ch
      y_ch    <- input$y_ch

      axes_trans <- get_axes_trans()

      tryCatch({
        # cyto_plot_save opens the device; the following cyto_plot call renders;
        # cyto_plot_complete() closes and saves.
        save_as <- paste0(tempfile(), ".", fmt)

        safe_cyto(
          CytoExploreR::cyto_plot_save(
            save_as = save_as, height = h, width = w, units = "in", res = 300
          ),
          "cyto_plot_save failed"
        )

        apply_dark_theme()

        samp_idx <- if (!is.null(samples) && length(samples) > 0) {
          which(sampleNames(fs) %in% samples)
        } else seq_len(min(4, length(fs)))

        if (view == "cyto") {
          obj <- if (!is.null(gs)) gs[samp_idx] else fs[samp_idx]
          CytoExploreR::cyto_plot(obj, parent = pop,
                                   channels = c(x_ch, y_ch),
                                   axes_trans = axes_trans,
                                   contour_lines = input$contour_lines %||% 0)
        } else if (view == "profile") {
          fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),
                               error = function(e) shared$channels)
          obj <- if (!is.null(gs)) gs[samp_idx] else fs[samp_idx]
          CytoExploreR::cyto_plot_profile(obj, parent = pop,
                                           channels = fluor_ch,
                                           axes_trans = axes_trans)
        } else if (view == "scheme" && !is.null(gs)) {
          CytoExploreR::cyto_plot_gating_scheme(gs[samp_idx[1]])
        } else {
          obj <- if (!is.null(gs)) gs[samp_idx] else fs[samp_idx]
          CytoExploreR::cyto_plot(obj, parent = pop,
                                   channels = c(x_ch, y_ch),
                                   axes_trans = axes_trans)
        }

        safe_cyto(CytoExploreR::cyto_plot_complete(), "cyto_plot_complete failed")
        reset_theme()

        if (file.exists(save_as)) {
          file.copy(save_as, file, overwrite = TRUE)
        } else {
          # Fallback: open device manually
          if (fmt == "pdf") pdf(file, width = w, height = h)
          else if (fmt == "svg") svg(file, width = w, height = h)
          else if (fmt == "tiff") tiff(file, width = w * 300, height = h * 300, res = 300)
          else png(file, width = w * 300, height = h * 300, res = 300, bg = "#0D1B2A")
          apply_dark_theme()
          obj <- if (!is.null(gs)) gs[samp_idx] else fs[samp_idx]
          CytoExploreR::cyto_plot(obj, parent = pop,
                                   channels = c(x_ch, y_ch),
                                   axes_trans = axes_trans)
          reset_theme()
          dev.off()
        }
      }, error = function(e) {
        tryCatch(dev.off(), error = function(e2) NULL)
        png(file, width = 1000, height = 800, bg = "#0D1B2A")
        plot.new()
        text(0.5, 0.5, paste("Export error:", conditionMessage(e)),
             col = "#C0392B", cex = 1)
        dev.off()
      })
    }
  )
}
