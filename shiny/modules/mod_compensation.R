# StreamFLOW - mod_compensation.R
# Spillover computation, matrix editing, compensation application
# CytoExploreR: cyto_spillover_extract, cyto_spillover_compute,
#               cyto_spillover_spread_compute, cyto_compensate,
#               cyto_plot_compensation, cyto_fluor_channels

library(rhandsontable)

# ── UI ────────────────────────────────────────────────────────────────────────
compensationUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left controls
      column(4,
        box(
          title = "Compensation Source", width = NULL, solidHeader = TRUE,
          radioButtons(
            ns("comp_source"), label = NULL,
            choices  = c("Extract from FCS (cyto_spillover_extract)" = "embedded",
                         "Upload CSV Matrix"                          = "upload",
                         "Compute from Controls (cyto_spillover_compute)" = "controls"),
            selected = "embedded"
          ),

          conditionalPanel(
            condition = sprintf("input['%s'] == 'upload'", ns("comp_source")),
            fileInput(ns("spillover_csv"), "Upload Spillover CSV",
                      accept = ".csv", placeholder = "Select CSV…")
          ),

          conditionalPanel(
            condition = sprintf("input['%s'] == 'controls'", ns("comp_source")),
            actionButton(ns("controls_folder_btn"),
                         label = tagList(icon("folder-open"), " Browse Controls Folder"),
                         class = "btn btn-default btn-block",
                         onclick = sprintf(
                           "streamflowPickFolder('%s', 'Single-stain controls folder')",
                           ns("controls_folder_picked")
                         )),
            uiOutput(ns("controls_folder_label"))
          ),

          tags$hr(),
          actionButton(ns("load_matrix_btn"),
                       tagList(icon("sync"), " Load / Compute Matrix"),
                       class = "btn btn-primary btn-block"),
          tags$hr(),
          actionButton(ns("apply_comp_btn"),
                       tagList(icon("check"), " Apply Compensation"),
                       class = "btn btn-success btn-block"),
          uiOutput(ns("comp_status_ui")),
          tags$hr(),
          # Spillover spreading
          actionButton(ns("spreading_btn"),
                       tagList(icon("project-diagram"), " Compute Spreading Matrix"),
                       class = "btn btn-default btn-block btn-sm"),
          uiOutput(ns("spreading_status_ui"))
        ),

        box(
          title = "Comparison Channels", width = NULL, solidHeader = TRUE,
          uiOutput(ns("comparison_channel_ui"))
        )
      ),

      # Right panel
      column(8,
        box(
          title = "Spillover Matrix", width = NULL, solidHeader = TRUE,
          tabsetPanel(
            tabPanel("Heatmap",
              withSpinner(plotlyOutput(ns("spillover_heatmap"), height = "300px"),
                          color = "#00B4D8")
            ),
            tabPanel("Edit Matrix",
              tags$p(style = "font-size:11px;color:#8899AA;margin:6px 0;",
                     "Diagonal = 1. Edit any cell to override."),
              withSpinner(rHandsontableOutput(ns("spillover_table"), height = "280px"),
                          color = "#00B4D8")
            ),
            tabPanel("Spreading Matrix",
              tags$p(style = "font-size:11px;color:#8899AA;margin:6px 0;",
                     "Computed via cyto_spillover_spread_compute()."),
              withSpinner(plotlyOutput(ns("spreading_heatmap"), height = "280px"),
                          color = "#00B4D8")
            )
          )
        ),

        box(
          title = "Compensation Visualisation (cyto_plot_compensation)", width = NULL, solidHeader = TRUE,
          tabsetPanel(
            tabPanel("Before / After Scatter",
              fluidRow(
                column(6,
                  tags$p(style = "font-size:11px;color:#5A7A8A;text-align:center;", "Before"),
                  withSpinner(plotlyOutput(ns("before_plot"), height = "230px"), color = "#00B4D8")
                ),
                column(6,
                  tags$p(style = "font-size:11px;color:#5A7A8A;text-align:center;", "After"),
                  withSpinner(plotlyOutput(ns("after_plot"), height = "230px"), color = "#00B4D8")
                )
              )
            ),
            tabPanel("cyto_plot_compensation",
              uiOutput(ns("comp_plot_sample_ui")),
              withSpinner(plotOutput(ns("cyto_comp_plot"), height = "300px"), color = "#00B4D8"),
              tags$p(style = "font-size:11px;color:#5A7A8A;margin-top:4px;",
                     "Shows spillover of all fluorescent channels for selected sample.")
            )
          )
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
compensationServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    spillover         = NULL,
    spreading_matrix  = NULL,
    controls_path     = NULL
  )

  output$controls_folder_label <- renderUI({
    if (!is.null(local$controls_path))
      tags$p(style = "font-size:11px;color:#2EC4B6;margin:4px 0;",
             icon("check-circle"), " ", basename(local$controls_path))
  })

  observeEvent(input$controls_folder_picked, {
    sel <- input$controls_folder_picked
    if (is.list(sel) && identical(sel$error, "no_electron")) {
      showNotification("Folder selection requires the StreamFLOW desktop app.",
                       type = "warning", duration = 5)
      return()
    }
    p <- sel$path
    if (is.null(p) || !nzchar(p)) return()
    if (!dir.exists(p)) {
      showNotification(paste("Folder not found:", p), type = "error", duration = 5)
      return()
    }
    local$controls_path <- p
  })

  # ── Load / compute spillover ──────────────────────────────────────────────
  observeEvent(input$load_matrix_btn, {
    req(shared$raw_flowset)
    shared$status <- "busy"

    withProgress(message = "Loading compensation matrix…", value = 0, {
      fs  <- shared$raw_flowset
      incProgress(0.2)

      mat <- tryCatch({
        if (input$comp_source == "embedded") {
          # cyto_spillover_extract — correct CytoExploreR API
          sp <- safe_cyto(
            CytoExploreR::cyto_spillover_extract(fs[[1]]),
            "cyto_spillover_extract failed"
          )
          if (is.null(sp)) {
            # Fallback: read raw keyword
            sp2 <- flowCore::spillover(fs[[1]])
            if (is.list(sp2)) sp2 <- sp2[[which(!sapply(sp2, is.null))[1]]]
            sp2
          } else sp

        } else if (input$comp_source == "upload") {
          req(input$spillover_csv)
          as.matrix(read.csv(input$spillover_csv$datapath, row.names = 1, check.names = FALSE))

        } else if (input$comp_source == "controls") {
          req(local$controls_path)
          ctrl_files <- list.files(local$controls_path, pattern = "\\.fcs$",
                                   full.names = TRUE, ignore.case = TRUE)
          req(length(ctrl_files) > 0)
          ctrl_fs <- CytoExploreR::cyto_load(ctrl_files)
          # cyto_spillover_compute — CytoExploreR proper call
          safe_cyto(
            CytoExploreR::cyto_spillover_compute(ctrl_fs),
            "cyto_spillover_compute failed"
          )
        }
      }, error = function(e) {
        showNotification(paste("Matrix error:", conditionMessage(e)), type = "error", duration = 6)
        NULL
      })

      incProgress(0.6)
      if (!is.null(mat)) {
        mat <- as.matrix(mat)
        local$spillover         <- mat
        shared$spillover_matrix <- mat
        showNotification("Spillover matrix loaded via cyto_spillover_extract().",
                         type = "message", duration = 3)
      } else {
        # safe_cyto swallows the underlying error to the log; surface it so the
        # user isn't left staring at an unchanged panel with no explanation.
        showNotification(
          "Could not obtain a spillover matrix from the selected source. See streamflow.log for details.",
          type = "warning", duration = 6)
      }
    })

    shared$status <- "idle"
  })

  # ── Spreading matrix ──────────────────────────────────────────────────────
  observeEvent(input$spreading_btn, {
    req(shared$raw_flowset, local$spillover)
    shared$status <- "busy"
    withProgress(message = "Computing spreading matrix (cyto_spillover_spread_compute)…", value = 0, {
      incProgress(0.3)
      sm <- tryCatch(
        safe_cyto(
          CytoExploreR::cyto_spillover_spread_compute(
            shared$raw_flowset,
            spillover = local$spillover
          ),
          "cyto_spillover_spread_compute failed"
        ),
        error = function(e) {
          showNotification(paste("Spreading matrix error:", conditionMessage(e)),
                           type = "error", duration = 5)
          NULL
        }
      )
      incProgress(0.7)
      if (!is.null(sm)) {
        local$spreading_matrix <- as.matrix(sm)
        showNotification("Spreading matrix computed.", type = "message", duration = 3)
      }
    })
    shared$status <- "idle"
  })

  output$spreading_status_ui <- renderUI({
    if (!is.null(local$spreading_matrix))
      tags$p(style = "font-size:12px;color:#2EC4B6;margin-top:4px;",
             icon("check"), " Spreading matrix ready")
  })

  # ── Heatmap ───────────────────────────────────────────────────────────────
  make_heatmap <- function(mat, title) {
    req(mat)
    plot_ly(z = mat, x = colnames(mat), y = rownames(mat),
            type = "heatmap",
            colorscale = list(list(0,"#0D1B2A"), list(0.5,"#00B4D8"), list(1,"#2EC4B6")),
            hovertemplate = "%{y} → %{x}: %{z:.4f}<extra></extra>") %>%
      plotly_dark_layout(title = title) %>%
      layout(xaxis = list(tickangle = -45), margin = list(b = 80, l = 80))
  }

  output$spillover_heatmap  <- renderPlotly({ req(local$spillover);        make_heatmap(local$spillover,        "Spillover Matrix") })
  output$spreading_heatmap  <- renderPlotly({ req(local$spreading_matrix); make_heatmap(local$spreading_matrix, "Spillover Spreading Matrix") })

  # ── Editable table ────────────────────────────────────────────────────────
  output$spillover_table <- renderRHandsontable({
    req(local$spillover)
    rhandsontable(as.data.frame(local$spillover), readOnly = FALSE, width = "100%") %>%
      hot_table(stretchH = "all") %>%
      hot_cols(renderer = "
        function(instance, td, row, col, prop, value, cellProperties) {
          Handsontable.renderers.NumericRenderer.apply(this, arguments);
          if (row === col) { td.style.background='#1A3A50'; td.style.color='#00B4D8'; }
          else if (value > 0.1) { td.style.background='#3A2010'; }
        }")
  })

  observeEvent(input$spillover_table, {
    req(input$spillover_table)
    mat <- as.matrix(hot_to_r(input$spillover_table))
    local$spillover         <- mat
    shared$spillover_matrix <- mat
  })

  # ── Apply compensation via cyto_compensate() ──────────────────────────────
  observeEvent(input$apply_comp_btn, {
    req(shared$raw_flowset, local$spillover)
    shared$status <- "busy"

    withProgress(message = "Applying compensation (cyto_compensate)…", value = 0, {
      incProgress(0.3)
      comp_fs <- tryCatch({
        # cyto_compensate — high-level CytoExploreR wrapper
        result <- safe_cyto(
          CytoExploreR::cyto_compensate(shared$raw_flowset,
                                         spillover = local$spillover),
          "cyto_compensate failed"
        )
        if (is.null(result)) {
          # Fallback to flowCore
          comp_obj <- flowCore::compensation(local$spillover)
          flowCore::compensate(shared$raw_flowset, comp_obj)
        } else result
      }, error = function(e) {
        showNotification(paste("Compensation error:", conditionMessage(e)),
                         type = "error", duration = 6)
        NULL
      })
      incProgress(0.7)
      if (!is.null(comp_fs)) {
        shared$comp_flowset <- comp_fs
        showNotification("Compensation applied via cyto_compensate().",
                         type = "message", duration = 3)
      }
    })

    shared$status <- "idle"
  })

  # ── Comparison channel selectors ──────────────────────────────────────────
  output$comparison_channel_ui <- renderUI({
    req(shared$raw_flowset)
    fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(shared$raw_flowset),
                         error = function(e) {
                           ch <- colnames(shared$raw_flowset)
                           grep("^(FSC|SSC)", ch, invert = TRUE, value = TRUE, ignore.case = TRUE)
                         })
    if (length(fluor_ch) < 2) fluor_ch <- colnames(shared$raw_flowset)
    tagList(
      selectInput(ns("comp_ch_x"), "X Axis", choices = fluor_ch, selected = fluor_ch[1]),
      selectInput(ns("comp_ch_y"), "Y Axis", choices = fluor_ch,
                  selected = if (length(fluor_ch) > 1) fluor_ch[2] else fluor_ch[1])
    )
  })

  # ── Before / After scatter ────────────────────────────────────────────────
  make_scatter <- function(fs, x_ch, y_ch, title) {
    req(fs, x_ch, y_ch)
    tryCatch({
      ff   <- fs[[1]]
      expr <- as.data.frame(exprs(ff))
      req(x_ch %in% names(expr), y_ch %in% names(expr))
      idx  <- sample(nrow(expr), min(nrow(expr), 3000))
      df   <- expr[idx, ]
      plot_ly(data = df, x = ~get(x_ch), y = ~get(y_ch),
              type = "scatter", mode = "markers",
              marker = list(size = 2, opacity = 0.5, color = "#00B4D8"),
              hoverinfo = "none") %>%
        plotly_dark_layout(title = title, xlab = x_ch, ylab = y_ch)
    }, error = function(e) {
      plot_ly() %>% layout(paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B",
                           annotations = list(list(text = conditionMessage(e),
                             font = list(color = "#C0392B"), showarrow = FALSE,
                             xref = "paper", yref = "paper", x = 0.5, y = 0.5)))
    })
  }

  output$before_plot <- renderPlotly({
    req(shared$raw_flowset, input$comp_ch_x, input$comp_ch_y)
    make_scatter(shared$raw_flowset, input$comp_ch_x, input$comp_ch_y, "Before")
  })

  output$after_plot <- renderPlotly({
    fs <- shared$comp_flowset %||% shared$raw_flowset
    req(fs, input$comp_ch_x, input$comp_ch_y)
    make_scatter(fs, input$comp_ch_x, input$comp_ch_y, "After")
  })

  # ── cyto_plot_compensation plot ───────────────────────────────────────────
  output$comp_plot_sample_ui <- renderUI({
    fs <- shared$comp_flowset %||% shared$raw_flowset
    req(fs)
    selectInput(ns("comp_plot_sample"), "Sample",
                choices  = sampleNames(fs),
                selected = sampleNames(fs)[1])
  })

  output$cyto_comp_plot <- renderPlot({
    fs   <- shared$comp_flowset %||% shared$raw_flowset
    samp <- input$comp_plot_sample
    req(fs, samp, samp %in% sampleNames(fs))

    par(bg = "#0D1B2A", col.axis = "#E0E0E0", col.lab = "#E0E0E0",
        col.main = "#00B4D8", fg = "#E0E0E0")

    tryCatch(
      safe_cyto(
        CytoExploreR::cyto_plot_compensation(fs[[samp]]),
        "cyto_plot_compensation failed"
      ),
      error = function(e) {
        plot.new()
        text(0.5, 0.5, paste("cyto_plot_compensation():\n", conditionMessage(e)),
             col = "#C0392B", cex = 0.9)
      }
    )
  }, bg = "#0D1B2A")

  output$comp_status_ui <- renderUI({
    if (!is.null(shared$comp_flowset))
      tags$p(style = "font-size:12px;color:#2EC4B6;margin-top:8px;",
             icon("check"), " Compensation applied")
  })
}
