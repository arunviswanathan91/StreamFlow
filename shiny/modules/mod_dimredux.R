# StreamFLOW - mod_dimredux.R
# UMAP, tSNE, PCA dimensionality reduction

# ── UI ────────────────────────────────────────────────────────────────────────
dimreduxUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Controls
      column(3,
        box(
          title = "Reduction Settings", width = NULL, solidHeader = TRUE,

          selectInput(ns("method"), "Method",
                      choices  = c("UMAP" = "umap", "tSNE" = "tsne", "PCA" = "pca"),
                      selected = "umap"),

          uiOutput(ns("channel_picker_ui")),

          numericInput(ns("n_cells"), "Cells to Subsample",
                       value = 5000, min = 100, max = 50000, step = 500),

          # UMAP parameters
          conditionalPanel(
            condition = sprintf("input['%s'] == 'umap'", ns("method")),
            sliderInput(ns("umap_neighbors"), "n_neighbors", 5, 50, 15, 1, ticks = FALSE),
            sliderInput(ns("umap_min_dist"),  "min_dist",   0.01, 1.0, 0.1, 0.01, ticks = FALSE)
          ),

          # tSNE parameters
          conditionalPanel(
            condition = sprintf("input['%s'] == 'tsne'", ns("method")),
            sliderInput(ns("tsne_perplexity"), "Perplexity", 5, 100, 30, 1, ticks = FALSE)
          ),

          tags$hr(),

          selectInput(ns("color_by"), "Colour By",
                      choices = c("Sample Group"   = "group",
                                  "Channel Expr."  = "channel",
                                  "Population"     = "population")),

          uiOutput(ns("color_channel_ui")),

          sliderInput(ns("pt_size"),  "Point Size", 1, 8, 3, 0.5, ticks = FALSE),
          sliderInput(ns("pt_alpha"), "Opacity",    0.1, 1, 0.7, 0.1, ticks = FALSE),

          tags$hr(),

          withSpinner(
            actionButton(
              ns("run_btn"),
              tagList(icon("play"), " Run Reduction"),
              class = "btn btn-primary btn-block"
            ),
            type = 4, color = "#00B4D8", size = 0.4
          ),

          tags$div(style = "margin-top: 6px;",
            downloadButton(ns("export_map"), "Export Map", class = "btn btn-default btn-block btn-sm")
          ),

          uiOutput(ns("run_status_ui"))
        )
      ),

      # Plot
      column(9,
        box(
          title = "Dimensionality Reduction Map", width = NULL, solidHeader = TRUE,
          withSpinner(
            plotlyOutput(ns("dim_plot"), height = "540px"),
            color = "#00B4D8"
          )
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
dimreduxServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    result     = NULL,   # data.frame with Dim1, Dim2, Sample, Group, and channel cols
    running    = FALSE
  )

  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Dynamic UI ────────────────────────────────────────────────────────────
  output$channel_picker_ui <- renderUI({
    channels <- shared$channels
    req(channels)
    fluor_ch <- grep("^(FSC|SSC)", channels, invert = TRUE, value = TRUE, ignore.case = TRUE)
    if (length(fluor_ch) == 0) fluor_ch <- channels
    checkboxGroupInput(
      ns("selected_channels"),
      "Channels for Reduction",
      choices  = fluor_ch,
      selected = fluor_ch
    )
  })

  output$color_channel_ui <- renderUI({
    req(input$color_by == "channel")
    channels <- shared$channels
    req(channels)
    selectInput(ns("color_channel"), "Color Channel", choices = channels)
  })

  # ── Run reduction ─────────────────────────────────────────────────────────
  observeEvent(input$run_btn, {
    fs      <- best_fs()
    req(fs)
    ch_sel  <- input$selected_channels
    req(ch_sel, length(ch_sel) >= 2)

    shared$status  <- "busy"
    local$running  <- TRUE

    withProgress(message = paste("Running", toupper(input$method), "..."), value = 0, {
      result <- tryCatch({
        incProgress(0.1, detail = "Subsampling cells...")

        # Collect and subsample data
        n_target <- as.integer(input$n_cells)
        all_expr <- lapply(sampleNames(fs), function(samp) {
          ff   <- fs[[samp]]
          expr <- as.data.frame(exprs(ff))
          n    <- min(nrow(expr), max(1, floor(n_target / length(fs))))
          idx  <- sample(nrow(expr), n)
          df   <- expr[idx, ch_sel, drop = FALSE]
          df$Sample <- samp
          annot <- shared$annotation
          if (!is.null(annot) && "SampleName" %in% names(annot)) {
            idx_a <- which(annot$SampleName == samp)
            df$Group <- if (length(idx_a) > 0) annot$Group[idx_a[1]] else "Unknown"
          } else {
            df$Group <- "Unknown"
          }
          df
        })
        df_all <- do.call(rbind, all_expr)
        mat    <- as.matrix(df_all[, ch_sel, drop = FALSE])
        mat[is.na(mat)] <- 0
        mat[!is.finite(mat)] <- 0

        incProgress(0.3, detail = "Computing embedding...")

        coords <- switch(input$method,
          umap = {
            req(requireNamespace("umap", quietly = TRUE))
            cfg            <- umap::umap.defaults
            cfg$n_neighbors <- as.integer(input$umap_neighbors)
            cfg$min_dist    <- input$umap_min_dist
            res <- umap::umap(mat, config = cfg)
            res$layout
          },
          tsne = {
            req(requireNamespace("Rtsne", quietly = TRUE))
            perp <- min(as.integer(input$tsne_perplexity), floor((nrow(mat) - 1) / 3))
            perp <- max(perp, 1)
            res  <- Rtsne::Rtsne(mat, perplexity = perp, check_duplicates = FALSE, verbose = FALSE)
            res$Y
          },
          pca = {
            pc <- prcomp(mat, center = TRUE, scale. = TRUE)
            pc$x[, 1:2]
          }
        )

        incProgress(0.4, detail = "Preparing results...")

        result_df <- cbind(
          df_all,
          data.frame(Dim1 = coords[, 1], Dim2 = coords[, 2])
        )
        result_df
      }, error = function(e) {
        showNotification(
          paste(toupper(input$method), "error:", conditionMessage(e)),
          type = "error", duration = 8
        )
        NULL
      })

      incProgress(0.2, detail = "Done!")

      if (!is.null(result)) {
        local$result   <- result
        shared$dim_result <- result
        showNotification(
          sprintf("%s complete: %d cells", toupper(input$method), nrow(result)),
          type = "message", duration = 3
        )
      }
    })

    local$running  <- FALSE
    shared$status  <- "idle"
  })

  # ── Plot ─────────────────────────────────────────────────────────────────
  output$dim_plot <- renderPlotly({
    df <- local$result
    if (is.null(df)) {
      return(
        plot_ly() %>%
          layout(
            paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
            annotations = list(list(
              text = "Run dimensionality reduction to see results here.",
              font = list(color = "#5A7A8A", size = 14), showarrow = FALSE,
              xref = "paper", yref = "paper", x = 0.5, y = 0.5
            ))
          )
      )
    }

    req("Dim1" %in% names(df), "Dim2" %in% names(df))

    color_by <- input$color_by %||% "group"
    pt_size  <- input$pt_size  %||% 3
    pt_alpha <- input$pt_alpha %||% 0.7

    color_vec <- switch(color_by,
      group = df$Group,
      channel = {
        ch <- input$color_channel
        if (!is.null(ch) && ch %in% names(df)) df[[ch]] else df$Group
      },
      population = df$Group
    )

    method_label <- switch(input$method %||% "umap",
      umap = "UMAP", tsne = "t-SNE", pca = "PCA", "Dim"
    )

    plot_ly(
      data = df,
      x    = ~Dim1,
      y    = ~Dim2,
      type = "scatter",
      mode = "markers",
      color = color_vec,
      marker = list(size = pt_size, opacity = pt_alpha),
      text  = ~paste0("Sample: ", Sample, "<br>Group: ", Group),
      hovertemplate = "%{text}<extra></extra>"
    ) %>%
      plotly_dark_layout(
        title = paste(method_label, "Map —", nrow(df), "cells"),
        xlab  = paste(method_label, "1"),
        ylab  = paste(method_label, "2")
      )
  })

  output$run_status_ui <- renderUI({
    df <- local$result
    if (is.null(df)) return(NULL)
    method_label <- switch(input$method %||% "umap",
      umap = "UMAP", tsne = "t-SNE", pca = "PCA", "")
    tags$p(
      style = "font-size: 12px; color: #2EC4B6; margin-top: 6px;",
      icon("check"), sprintf(" %s: %d cells", method_label, nrow(df))
    )
  })

  # ── Export ────────────────────────────────────────────────────────────────
  output$export_map <- downloadHandler(
    filename = function() {
      paste0("StreamFLOW_", input$method, "_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".png")
    },
    content = function(file) {
      df <- local$result
      req(df)
      color_by <- input$color_by %||% "group"
      color_col <- switch(color_by,
        channel = if (!is.null(input$color_channel) && input$color_channel %in% names(df))
                    input$color_channel else "Group",
        "Group"
      )

      p <- ggplot(df, aes(x = Dim1, y = Dim2, colour = .data[[color_col]])) +
        geom_point(size = input$pt_size %||% 1.5,
                   alpha = input$pt_alpha %||% 0.7) +
        scale_color_viridis_c(option = "plasma") +
        labs(
          title  = paste(toupper(input$method %||% "UMAP"), "Map"),
          x = "Dim 1", y = "Dim 2",
          colour = color_col
        ) +
        theme_streamflow()

      ggsave(file, plot = p, width = 10, height = 8, dpi = 150, bg = "#0D1B2A")
    }
  )
}
