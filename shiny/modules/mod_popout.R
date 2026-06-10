# StreamFLOW - mod_popout.R
# A focused single-sample window opened by Electron (FlowJo-style "open in new
# window"). Served from the same Shiny app via the ?view=popout query string and
# rendered as a separate Shiny session. Data is read from the shared app_state
# environment (same R process as the main session).

# ── UI for a pop-out window ────────────────────────────────────────────────────
popout_ui <- function(qs) {
  samp <- qs$sample %||% "Sample"
  tagList(
    tags$head(
      tags$style(HTML("
        body, html { background:#0D1B2A; color:#E0E0E0; margin:0; padding:0;
          font-family:'Segoe UI',sans-serif; overflow:hidden; }
        .popout-bar { background:#081420; padding:8px 14px; border-bottom:1px solid #0A1F30;
          display:flex; align-items:center; gap:12px; }
        .popout-logo { color:#00B4D8; font-weight:700; letter-spacing:2px; font-size:13px; }
        .popout-sample { color:#2EC4B6; font-size:13px; }
        .popout-body { padding:10px; }
        .form-control, .selectize-input { background:#243447 !important; color:#E0E0E0 !important;
          border:1px solid #2E4460 !important; }
        label { color:#B0C4D8 !important; font-size:12px !important; }
      "))
    ),
    tags$div(class = "popout-bar",
      tags$span(class = "popout-logo", "StreamFLOW"),
      tags$span(class = "popout-sample", samp)
    ),
    tags$div(class = "popout-body",
      fluidRow(
        column(3, uiOutput("popout_controls")),
        column(9, plotlyOutput("popout_plot", height = "640px"))
      )
    )
  )
}

# ── Server for a pop-out window ────────────────────────────────────────────────
popout_server <- function(input, output, session, qs) {
  fs   <- app_state$flowset
  samp <- qs$sample

  output$popout_controls <- renderUI({
    if (is.null(fs))
      return(tags$p(style = "color:#F39C12;font-size:12px;",
                    "No data loaded in the main window yet."))
    chans <- colnames(fs)
    x_sel <- if (!is.null(qs$x) && qs$x %in% chans) qs$x else chans[1]
    y_sel <- if (!is.null(qs$y) && qs$y %in% chans) qs$y else
             if (length(chans) > 1) chans[2] else chans[1]
    tagList(
      radioButtons("dim", "Mode",
                   choices  = c("2D" = "2d", "1D" = "1d"),
                   selected = qs$dim %||% "2d", inline = TRUE),
      selectInput("x_ch", "X Channel", choices = chans, selected = x_sel),
      conditionalPanel("input.dim == '2d'",
        selectInput("y_ch", "Y Channel", choices = chans, selected = y_sel))
    )
  })

  output$popout_plot <- renderPlotly({
    req(fs, samp, samp %in% sampleNames(fs))
    x_ch <- input$x_ch %||% qs$x
    req(x_ch, x_ch %in% colnames(fs))

    ff   <- fs[[samp]]
    expr <- as.data.frame(exprs(ff))
    n    <- min(nrow(expr), 20000)
    df   <- expr[sample(nrow(expr), n), , drop = FALSE]

    if (identical(input$dim, "1d")) {
      xv <- df[[x_ch]]; xv <- xv[is.finite(xv)]
      d  <- density(xv, n = 512)
      return(
        plot_ly(x = d$x, y = d$y, type = "scatter", mode = "lines",
                fill = "tozeroy", line = list(color = "#00B4D8", width = 1.5),
                fillcolor = "rgba(0,180,216,0.25)", hoverinfo = "none") %>%
          plotly_dark_layout(title = sprintf("%s — %s", samp, x_ch),
                             xlab = x_ch, ylab = "Density")
      )
    }

    y_ch <- input$y_ch %||% qs$y
    req(y_ch, y_ch %in% colnames(fs))
    dens <- tryCatch({
      dd <- MASS::kde2d(df[[x_ch]], df[[y_ch]], n = 100)
      fields::interp.surface(dd, cbind(df[[x_ch]], df[[y_ch]]))
    }, error = function(e) rep(1, nrow(df)))
    dens[!is.finite(dens)] <- 0

    plot_ly(data = df, x = ~get(x_ch), y = ~get(y_ch),
            type = "scattergl", mode = "markers",
            marker = list(size = 3, opacity = 0.6, color = dens,
              colorscale = list(list(0, "#081420"), list(0.3, "#00B4D8"),
                                list(0.7, "#2EC4B6"), list(1, "#FFFFFF")),
              showscale = FALSE),
            hoverinfo = "none") %>%
      plotly_dark_layout(title = sprintf("%s — %s vs %s", samp, x_ch, y_ch),
                         xlab = x_ch, ylab = y_ch)
  })
}
