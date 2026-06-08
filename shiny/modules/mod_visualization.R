# StreamFLOW - mod_visualization.R
# cyto_plot outputs, gating scheme, export

# ── UI ────────────────────────────────────────────────────────────────────────
visualizationUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left controls
      column(3,
        box(
          title = "Plot Controls", width = NULL, solidHeader = TRUE,
          selectInput(ns("plot_type"), "Plot Type",
                      choices = c("Scatter"   = "scatter",
                                  "Density"   = "density",
                                  "Histogram" = "histogram",
                                  "Contour"   = "contour",
                                  "Overlay"   = "overlay"),
                      selected = "scatter"),

          uiOutput(ns("channel_controls_ui")),
          uiOutput(ns("population_ui")),
          uiOutput(ns("sample_selector_ui")),

          selectInput(ns("color_by"), "Colour By",
                      choices = c("Density"        = "density",
                                  "Sample Group"   = "group",
                                  "Population"     = "population",
                                  "Channel Expr."  = "channel")),

          uiOutput(ns("color_channel_ui")),

          checkboxInput(ns("show_gates"),    "Show Gates",          value = TRUE),
          checkboxInput(ns("back_gating"),   "Back-Gating",         value = FALSE),
          checkboxInput(ns("gate_tracking"), "Gate Tracking",       value = FALSE),

          sliderInput(ns("pt_size"),    "Point Size",    0.5, 5, 2, 0.5, ticks = FALSE),
          sliderInput(ns("pt_alpha"),   "Opacity",       0.1, 1, 0.6, 0.1, ticks = FALSE),

          tags$hr(),
          selectInput(ns("export_format"), "Export Format",
                      choices = c("PNG" = "png", "PDF" = "pdf", "SVG" = "svg")),
          downloadButton(ns("export_plot"), "Export Plot",
                         class = "btn btn-default btn-block btn-sm")
        )
      ),

      # Right: plot tabs
      column(9,
        box(
          title = "Visualization", width = NULL, solidHeader = TRUE,
          tabsetPanel(
            id = ns("viz_tabs"),
            tabPanel("Plot",
              withSpinner(plotlyOutput(ns("main_plot"), height = "520px"), color = "#00B4D8")
            ),
            tabPanel("Gating Scheme",
              withSpinner(plotOutput(ns("gating_scheme_plot"), height = "520px"), color = "#00B4D8"),
              tags$p(style = "font-size: 11px; color: #5A7A8A; margin-top: 6px;",
                     "Full multi-level gating strategy. Requires gates to be applied.")
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

  # ── Dynamic UI ───────────────────────────────────────────────────────────
  output$channel_controls_ui <- renderUI({
    channels <- shared$channels
    req(channels)
    tagList(
      selectInput(ns("x_channel"), "X Channel", choices = channels, selected = channels[1]),
      conditionalPanel(
        condition = sprintf("input['%s'] != 'histogram'", ns("plot_type")),
        selectInput(ns("y_channel"), "Y Channel", choices = channels,
                    selected = if (length(channels) > 1) channels[2] else channels[1])
      )
    )
  })

  output$population_ui <- renderUI({
    gs     <- shared$gating_set
    pops   <- if (!is.null(gs)) {
      tryCatch(flowWorkspace::gs_get_pop_paths(gs, path = "auto"), error = function(e) c("root"))
    } else {
      c("root")
    }
    selectInput(ns("population"), "Population", choices = pops, selected = "root")
  })

  output$sample_selector_ui <- renderUI({
    fs <- best_fs()
    req(fs)
    selectInput(ns("samples"), "Samples",
                choices  = sampleNames(fs),
                selected = sampleNames(fs)[1],
                multiple = TRUE)
  })

  output$color_channel_ui <- renderUI({
    req(input$color_by == "channel")
    channels <- shared$channels
    req(channels)
    selectInput(ns("color_channel"), "Color Channel", choices = channels)
  })

  # ── Main plot ─────────────────────────────────────────────────────────────
  output$main_plot <- renderPlotly({
    fs       <- best_fs()
    x_ch     <- input$x_channel
    y_ch     <- input$y_channel
    samples  <- input$samples
    pt_size  <- input$pt_size  %||% 2
    pt_alpha <- input$pt_alpha %||% 0.6
    plot_type <- input$plot_type

    req(fs, x_ch, samples)
    req(x_ch %in% colnames(fs))

    tryCatch({
      # Collect data across selected samples
      all_data <- lapply(samples, function(samp) {
        if (!samp %in% sampleNames(fs)) return(NULL)
        ff   <- fs[[samp]]
        expr <- as.data.frame(exprs(ff))
        n    <- min(nrow(expr), 3000)
        idx  <- sample(nrow(expr), n)
        df   <- expr[idx, , drop = FALSE]
        df$Sample <- samp

        # Add group from annotation
        annot <- shared$annotation
        if (!is.null(annot) && "SampleName" %in% names(annot)) {
          idx_a <- which(annot$SampleName == samp)
          df$Group <- if (length(idx_a) > 0) annot$Group[idx_a[1]] else "Unknown"
        } else {
          df$Group <- "Unknown"
        }
        df
      })
      all_data <- Filter(Negate(is.null), all_data)
      req(length(all_data) > 0)
      df <- do.call(rbind, all_data)

      color_by <- input$color_by %||% "density"

      # Determine colour vector
      color_vec <- switch(color_by,
        density = {
          tryCatch({
            dens <- with(df, {
              d <- MASS::kde2d(get(x_ch), get(y_ch %||% x_ch), n = 100)
              fields::interp.surface(d, cbind(get(x_ch), get(y_ch %||% x_ch)))
            })
            dens
          }, error = function(e) rep(1, nrow(df)))
        },
        group   = df$Group,
        channel = {
          ch <- input$color_channel %||% x_ch
          if (ch %in% names(df)) df[[ch]] else rep(1, nrow(df))
        },
        rep(1, nrow(df))
      )

      p <- switch(plot_type,
        scatter = {
          plot_ly(
            data   = df,
            x      = ~get(x_ch),
            y      = ~get(y_ch),
            type   = "scatter",
            mode   = "markers",
            color  = color_vec,
            colors = c("#0D1B2A", "#00B4D8", "#2EC4B6", "#FFFFFF"),
            marker = list(size = pt_size, opacity = pt_alpha),
            hoverinfo = "none"
          )
        },
        histogram = {
          plot_ly(
            data  = df,
            x     = ~get(x_ch),
            type  = "histogram",
            color = ~Sample,
            nbinsx = 100,
            opacity = pt_alpha
          ) %>% layout(barmode = "overlay")
        },
        density = {
          plot_ly(
            data  = df,
            x     = ~get(x_ch),
            y     = ~get(y_ch),
            type  = "histogram2dcontour",
            colorscale = list(
              list(0, "#0D1B2A"),
              list(0.5, "#00B4D8"),
              list(1, "#2EC4B6")
            ),
            contours = list(showlabels = FALSE),
            line = list(color = "#00B4D8")
          )
        },
        contour = {
          plot_ly(
            data  = df,
            x     = ~get(x_ch),
            y     = ~get(y_ch),
            type  = "histogram2dcontour",
            colorscale = "Blues",
            line = list(color = "#2EC4B6", width = 1)
          )
        },
        overlay = {
          traces <- lapply(unique(df$Sample), function(s) {
            ds <- df[df$Sample == s, ]
            list(x = ds[[x_ch]], y = ds[[y_ch]], name = s)
          })
          p <- plot_ly()
          for (tr in traces) {
            p <- p %>% add_trace(
              x = tr$x, y = tr$y, name = tr$name,
              type = "scatter", mode = "markers",
              marker = list(size = pt_size, opacity = pt_alpha)
            )
          }
          p
        }
      )

      p %>% plotly_dark_layout(
        title = sprintf("%s vs %s", x_ch, y_ch %||% x_ch),
        xlab  = x_ch,
        ylab  = y_ch %||% "Count"
      )
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

  # ── Gating scheme plot ────────────────────────────────────────────────────
  output$gating_scheme_plot <- renderPlot({
    gs <- shared$gating_set
    if (is.null(gs)) {
      par(bg = "#0D1B2A", col.axis = "#E0E0E0", col.lab = "#E0E0E0")
      plot.new()
      text(0.5, 0.5, "Apply gates first\n(Gating tab → Apply All Gates)",
           col = "#8899AA", cex = 1.2, adj = c(0.5, 0.5))
      return()
    }

    tryCatch(
      safe_cyto(
        CytoExploreR::cyto_plot_gating_scheme(
          gs,
          back_gating   = input$back_gating,
          gate_tracking = input$gate_tracking
        ),
        "Failed to render gating scheme"
      ),
      error = function(e) {
        par(bg = "#0D1B2A")
        plot.new()
        text(0.5, 0.5, paste("Error:", conditionMessage(e)),
             col = "#C0392B", cex = 0.9)
      }
    )
  }, bg = "#0D1B2A")

  # ── Export ────────────────────────────────────────────────────────────────
  output$export_plot <- downloadHandler(
    filename = function() {
      paste0("StreamFLOW_plot_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".",
             input$export_format %||% "png")
    },
    content = function(file) {
      fmt <- input$export_format %||% "png"
      fs  <- best_fs()
      req(fs)

      if (fmt == "png") {
        png(file, width = 1200, height = 900, res = 150, bg = "#0D1B2A")
      } else if (fmt == "pdf") {
        pdf(file, width = 10, height = 7.5)
      } else {
        svg(file, width = 10, height = 7.5)
      }

      x_ch <- input$x_channel
      y_ch <- input$y_channel %||% x_ch
      samp <- (input$samples %||% sampleNames(fs))[1]

      tryCatch({
        if (!is.null(shared$gating_set)) {
          safe_cyto(
            CytoExploreR::cyto_plot(
              shared$gating_set[[samp]],
              parent   = input$population %||% "root",
              channels = c(x_ch, y_ch)
            ),
            "cyto_plot failed"
          )
        } else {
          ff   <- fs[[samp]]
          expr <- as.data.frame(exprs(ff))
          par(bg = "#0D1B2A", col.axis = "#E0E0E0", col.lab = "#E0E0E0",
              col.main = "#00B4D8")
          smoothScatter(expr[[x_ch]], expr[[y_ch]],
                        xlab = x_ch, ylab = y_ch,
                        main = paste(samp, "-", x_ch, "vs", y_ch),
                        colramp = colorRampPalette(c("#0D1B2A", "#00B4D8", "#2EC4B6")))
        }
      }, error = function(e) {
        plot.new()
        text(0.5, 0.5, paste("Export error:", conditionMessage(e)), col = "#C0392B")
      })

      dev.off()
    }
  )
}
