# StreamFLOW - mod_dimredux.R
# Dimensionality reduction using CytoExploreR::cyto_map()
#
# CytoExploreR API used:
#   cyto_map()              – UMAP / tSNE / PCA dimension-reduced maps
#   cyto_sample()           – subsample flowFrame or flowSet
#   cyto_extract()          – extract a population for mapping
#   cyto_fluor_channels()   – fluorescent channel list
#   cyto_nodes()            – gated population list
#   cyto_names()            – sample names
#   cyto_transformer_extract() – axes_trans for proper labelling

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

          uiOutput(ns("population_ui")),

          uiOutput(ns("channel_picker_ui")),

          numericInput(ns("n_cells"), "Cells to Subsample",
                       value = 5000, min = 100, max = 100000, step = 500),

          # UMAP params (passed through cyto_map)
          conditionalPanel(
            condition = sprintf("input['%s'] == 'umap'", ns("method")),
            sliderInput(ns("umap_neighbors"), "n_neighbors", 5, 50, 15, 1, ticks = FALSE),
            sliderInput(ns("umap_min_dist"),  "min_dist", 0.01, 1.0, 0.1, 0.01, ticks = FALSE)
          ),

          # tSNE params
          conditionalPanel(
            condition = sprintf("input['%s'] == 'tsne'", ns("method")),
            sliderInput(ns("tsne_perplexity"), "Perplexity", 5, 100, 30, 1, ticks = FALSE)
          ),

          tags$hr(),

          selectInput(ns("color_by"), "Colour By",
                      choices = c("Sample"         = "sample",
                                  "Sample Group"   = "group",
                                  "Channel Expr."  = "channel")),

          uiOutput(ns("color_channel_ui")),

          sliderInput(ns("pt_size"),  "Point Size", 1, 8, 3, 0.5, ticks = FALSE),
          sliderInput(ns("pt_alpha"), "Opacity",    0.1, 1, 0.7, 0.1, ticks = FALSE),

          tags$hr(),

          actionButton(ns("run_btn"),
                       tagList(icon("play"), " Run cyto_map()"),
                       class = "btn btn-primary btn-block"),

          tags$div(style = "margin-top:6px;",
            downloadButton(ns("export_map"), "Export Map",
                           class = "btn btn-default btn-block btn-sm")
          ),

          uiOutput(ns("run_status_ui"))
        )
      ),

      # Plot area
      column(9,
        box(
          title = "cyto_map() — Dimension-Reduced Map", width = NULL, solidHeader = TRUE,
          withSpinner(plotlyOutput(ns("dim_plot"), height = "540px"), color = "#00B4D8")
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
dimreduxServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    result   = NULL,   # data.frame with Dim1, Dim2, Sample, Group, channels
    map_obj  = NULL    # raw cyto_map output
  )

  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Dynamic UI ────────────────────────────────────────────────────────────
  output$population_ui <- renderUI({
    gs   <- shared$gating_set
    pops <- if (!is.null(gs)) {
      tryCatch(c("root", CytoExploreR::cyto_nodes(gs)), error = function(e) "root")
    } else "root"
    selectInput(ns("population"), "Population to Map", choices = pops, selected = "root")
  })

  output$channel_picker_ui <- renderUI({
    fs <- best_fs()
    req(fs)
    fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),
                         error = function(e) {
                           ch <- shared$channels %||% colnames(fs)
                           grep("^(FSC|SSC)", ch, invert = TRUE, value = TRUE,
                                ignore.case = TRUE)
                         })
    if (length(fluor_ch) == 0) fluor_ch <- shared$channels %||% colnames(fs)
    checkboxGroupInput(ns("selected_channels"), "Channels for Map",
                       choices = fluor_ch, selected = fluor_ch)
  })

  output$color_channel_ui <- renderUI({
    req(input$color_by == "channel")
    channels <- shared$channels %||% character()
    selectInput(ns("color_channel"), "Color Channel", choices = channels)
  })

  # ── Run cyto_map() ────────────────────────────────────────────────────────
  observeEvent(input$run_btn, {
    fs      <- best_fs()
    gs      <- shared$gating_set
    ch_sel  <- input$selected_channels
    req(fs, ch_sel, length(ch_sel) >= 2)

    shared$status <- "busy"

    withProgress(message = paste("Running cyto_map(method =", input$method, ")…"), value = 0, {

      result <- tryCatch({
        incProgress(0.1, detail = "Preparing data…")

        n_target <- as.integer(input$n_cells)
        pop      <- input$population %||% "root"
        method   <- input$method %||% "umap"

        # ── Attempt cyto_map() ─────────────────────────────────────────────
        # cyto_map() operates on a GatingSet, GatingHierarchy, flowSet or flowFrame
        map_result <- if (!is.null(gs)) {
          incProgress(0.1, detail = "cyto_map() on GatingSet…")

          # Build extra args per method
          extra_args <- switch(method,
            umap = list(
              n_neighbors = as.integer(input$umap_neighbors %||% 15),
              min_dist    = input$umap_min_dist %||% 0.1
            ),
            tsne = list(
              perplexity = as.integer(input$tsne_perplexity %||% 30)
            ),
            pca  = list(),
            list()
          )

          tryCatch(
            do.call(
              CytoExploreR::cyto_map,
              c(list(x        = gs,
                     parent   = pop,
                     channels = ch_sel,
                     display  = n_target,
                     type     = method),
                extra_args)
            ),
            error = function(e) {
              message("[cyto_map GatingSet] ", conditionMessage(e))
              NULL
            }
          )
        } else {
          # No GatingSet: use flowSet directly
          incProgress(0.1, detail = "cyto_map() on flowSet…")
          tryCatch(
            CytoExploreR::cyto_map(
              fs,
              channels = ch_sel,
              display  = n_target,
              type     = method
            ),
            error = function(e) {
              message("[cyto_map flowSet] ", conditionMessage(e))
              NULL
            }
          )
        }

        incProgress(0.3, detail = "Processing result…")

        if (!is.null(map_result)) {
          local$map_obj <- map_result

          # cyto_map returns a modified flowSet/GatingSet with DM1/DM2 added,
          # OR a data.frame — handle both
          df <- if (is.data.frame(map_result)) {
            map_result
          } else {
            # Extract exprs including DM1/DM2 columns
            tryCatch({
              sn <- tryCatch(CytoExploreR::cyto_names(map_result),
                             error = function(e) sampleNames(map_result))
              all_rows <- lapply(sn, function(s) {
                ff   <- if (inherits(map_result, "GatingSet")) map_result[[s]] else map_result[[s]]
                expr <- as.data.frame(exprs(ff))
                dm_cols <- grep("^(DM|UMAP|tSNE|PC)", names(expr), value = TRUE, ignore.case = TRUE)
                if (length(dm_cols) < 2) return(NULL)
                df_s <- expr[, union(dm_cols, intersect(ch_sel, names(expr))), drop = FALSE]
                df_s$Sample <- s
                annot <- shared$annotation
                df_s$Group <- if (!is.null(annot) && "SampleName" %in% names(annot)) {
                  idx_a <- which(annot$SampleName == s)
                  if (length(idx_a) > 0) annot$Group[idx_a[1]] else "Unknown"
                } else "Unknown"
                # Rename first two dim columns to Dim1/Dim2
                names(df_s)[1:2] <- c("Dim1", "Dim2")
                df_s
              })
              do.call(rbind, Filter(Negate(is.null), all_rows))
            }, error = function(e) NULL)
          }
          df
        } else {
          # Fallback: manual UMAP/tSNE/PCA
          incProgress(0, detail = "cyto_map unavailable — manual fallback…")
          showNotification("cyto_map() unavailable, using manual fallback.",
                           type = "warning", duration = 4)

          # cyto_sample for subsampling
          all_data <- lapply(sampleNames(fs), function(samp) {
            ff <- fs[[samp]]
            sampled <- tryCatch(
              CytoExploreR::cyto_sample(ff, display = floor(n_target / length(fs))),
              error = function(e) {
                n <- min(nrow(exprs(ff)), floor(n_target / length(fs)))
                ff[sample(nrow(exprs(ff)), n), ]
              }
            )
            expr <- as.data.frame(exprs(sampled))
            df_s <- expr[, intersect(ch_sel, names(expr)), drop = FALSE]
            df_s$Sample <- samp
            annot <- shared$annotation
            df_s$Group <- if (!is.null(annot) && "SampleName" %in% names(annot)) {
              idx_a <- which(annot$SampleName == samp)
              if (length(idx_a) > 0) annot$Group[idx_a[1]] else "Unknown"
            } else "Unknown"
            df_s
          })
          df_all <- do.call(rbind, all_data)
          mat    <- as.matrix(df_all[, ch_sel, drop = FALSE])
          mat[!is.finite(mat)] <- 0

          incProgress(0.2, detail = "Computing coordinates…")
          coords <- switch(method,
            umap = {
              cfg            <- umap::umap.defaults
              cfg$n_neighbors <- as.integer(input$umap_neighbors %||% 15)
              cfg$min_dist    <- input$umap_min_dist %||% 0.1
              umap::umap(mat, config = cfg)$layout
            },
            tsne = {
              perp <- min(as.integer(input$tsne_perplexity %||% 30),
                          floor((nrow(mat) - 1) / 3))
              Rtsne::Rtsne(mat, perplexity = max(perp, 1),
                           check_duplicates = FALSE, verbose = FALSE)$Y
            },
            pca = {
              pc <- prcomp(mat, center = TRUE, scale. = TRUE)
              pc$x[, 1:2]
            }
          )
          cbind(df_all, data.frame(Dim1 = coords[, 1], Dim2 = coords[, 2]))
        }
      }, error = function(e) {
        showNotification(paste("cyto_map error:", conditionMessage(e)),
                         type = "error", duration = 8)
        NULL
      })

      incProgress(0.2, detail = "Done!")

      if (!is.null(result) && is.data.frame(result) && nrow(result) > 0) {
        local$result      <- result
        shared$dim_result <- result
        showNotification(
          sprintf("cyto_map() complete: %d cells, method = %s", nrow(result), input$method),
          type = "message", duration = 3
        )
      } else {
        # Neither cyto_map nor the manual fallback produced a usable embedding.
        # Without this the panel just stays on its placeholder with no reason.
        showNotification(
          "Dimensionality reduction produced no result. Check channel selection and streamflow.log.",
          type = "warning", duration = 6)
      }
    })

    shared$status <- "idle"
  })

  # ── Interactive plotly map ─────────────────────────────────────────────────
  output$dim_plot <- renderPlotly({
    df <- local$result
    if (is.null(df) || !is.data.frame(df))
      return(
        plot_ly() %>% layout(
          paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
          annotations = list(list(
            text = "Run cyto_map() to see the dimension-reduced map here.",
            font = list(color = "#5A7A8A", size = 14), showarrow = FALSE,
            xref = "paper", yref = "paper", x = 0.5, y = 0.5
          ))
        )
      )

    req("Dim1" %in% names(df), "Dim2" %in% names(df))

    color_by  <- input$color_by  %||% "sample"
    pt_size   <- input$pt_size   %||% 3
    pt_alpha  <- input$pt_alpha  %||% 0.7
    method_lbl <- toupper(input$method %||% "umap")

    color_vec <- switch(color_by,
      sample  = df$Sample,
      group   = df$Group,
      channel = {
        ch <- input$color_channel
        if (!is.null(ch) && ch %in% names(df)) df[[ch]] else df$Sample
      }
    )

    hover_text <- paste0(
      "Sample: ", df$Sample, "<br>Group: ", df$Group,
      if (color_by == "channel" && !is.null(input$color_channel) &&
          input$color_channel %in% names(df))
        paste0("<br>", input$color_channel, ": ",
               round(df[[input$color_channel]], 2))
      else ""
    )

    # Continuous vs categorical colouring
    if (color_by == "channel" && !is.null(input$color_channel) &&
        input$color_channel %in% names(df)) {
      plot_ly(
        data = df, x = ~Dim1, y = ~Dim2,
        type = "scatter", mode = "markers",
        marker = list(
          size    = pt_size, opacity = pt_alpha,
          color   = color_vec,
          colorscale = list(list(0,"#0D1B2A"), list(0.3,"#00B4D8"),
                            list(0.7,"#2EC4B6"), list(1,"#FFFFFF")),
          showscale = TRUE,
          colorbar  = list(title = list(text = input$color_channel,
                                        font = list(color = "#E0E0E0")))
        ),
        text          = hover_text,
        hovertemplate = "%{text}<extra></extra>"
      ) %>% plotly_dark_layout(
        title = paste0(method_lbl, " Map — ", nrow(df), " cells"),
        xlab = paste(method_lbl, "1"),
        ylab = paste(method_lbl, "2")
      )
    } else {
      plot_ly(
        data  = df, x = ~Dim1, y = ~Dim2,
        type  = "scatter", mode = "markers",
        color = color_vec,
        marker = list(size = pt_size, opacity = pt_alpha),
        text  = hover_text,
        hovertemplate = "%{text}<extra></extra>"
      ) %>% plotly_dark_layout(
        title = paste0(method_lbl, " Map — ", nrow(df), " cells"),
        xlab = paste(method_lbl, "1"),
        ylab = paste(method_lbl, "2")
      )
    }
  })

  output$run_status_ui <- renderUI({
    df <- local$result
    if (is.null(df)) return(NULL)
    method_lbl <- toupper(input$method %||% "umap")
    tags$p(style = "font-size:12px;color:#2EC4B6;margin-top:6px;",
           icon("check"),
           sprintf(" cyto_map(%s): %d cells", method_lbl, nrow(df)))
  })

  # ── Export ────────────────────────────────────────────────────────────────
  output$export_map <- downloadHandler(
    filename = function() {
      paste0("StreamFLOW_cytomap_", input$method, "_",
             format(Sys.time(), "%Y%m%d_%H%M%S"), ".png")
    },
    content = function(file) {
      df <- local$result
      req(df, is.data.frame(df), "Dim1" %in% names(df))

      color_by  <- input$color_by %||% "sample"
      color_col <- switch(color_by,
        channel = if (!is.null(input$color_channel) && input$color_channel %in% names(df))
                    input$color_channel else "Sample",
        group   = "Group",
        "Sample"
      )
      method_lbl <- toupper(input$method %||% "umap")

      gg_color <- if (color_col %in% c("Sample", "Group", "sample", "group")) {
        ggplot(df, aes(x = Dim1, y = Dim2, colour = .data[[color_col]])) +
          geom_point(size = input$pt_size %||% 1.5,
                     alpha = input$pt_alpha %||% 0.7) +
          scale_colour_brewer(palette = "Set2")
      } else {
        ggplot(df, aes(x = Dim1, y = Dim2, colour = .data[[color_col]])) +
          geom_point(size = input$pt_size %||% 1.5,
                     alpha = input$pt_alpha %||% 0.7) +
          scale_colour_viridis_c(option = "plasma")
      }

      p <- gg_color +
        labs(title   = paste(method_lbl, "Map — cyto_map()"),
             x       = paste(method_lbl, "1"),
             y       = paste(method_lbl, "2"),
             colour  = color_col) +
        theme_streamflow()

      ggsave(file, plot = p, width = 10, height = 8, dpi = 150, bg = "#0D1B2A")
    }
  )
}
