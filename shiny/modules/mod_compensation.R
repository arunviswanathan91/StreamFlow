# StreamFLOW - mod_compensation.R
# Spillover computation, matrix editing, and compensation application

library(rhandsontable)

# ── UI ────────────────────────────────────────────────────────────────────────
compensationUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left control panel
      column(4,
        box(
          title = "Compensation Source", width = NULL, solidHeader = TRUE,
          radioButtons(
            ns("comp_source"),
            label   = NULL,
            choices = c(
              "Use Embedded Matrix"       = "embedded",
              "Upload CSV Matrix"         = "upload",
              "Compute from Controls"     = "controls"
            ),
            selected = "embedded"
          ),

          # Upload CSV
          conditionalPanel(
            condition = sprintf("input['%s'] == 'upload'", ns("comp_source")),
            fileInput(
              ns("spillover_csv"),
              label       = "Upload Spillover CSV",
              accept      = ".csv",
              placeholder = "Select CSV file..."
            )
          ),

          # Controls folder
          conditionalPanel(
            condition = sprintf("input['%s'] == 'controls'", ns("comp_source")),
            shinyDirButton(
              ns("controls_folder"),
              label = "Browse Controls Folder",
              title = "Select single-stain controls folder",
              class = "btn btn-default btn-block",
              icon  = icon("folder-open")
            ),
            uiOutput(ns("controls_folder_label"))
          ),

          tags$hr(),
          actionButton(
            ns("load_matrix_btn"),
            label = tagList(icon("sync"), " Load / Compute Matrix"),
            class = "btn btn-primary btn-block"
          ),
          tags$hr(),
          actionButton(
            ns("apply_comp_btn"),
            label = tagList(icon("check"), " Apply Compensation"),
            class = "btn btn-success btn-block"
          ),
          uiOutput(ns("comp_status_ui"))
        ),

        box(
          title = "Comparison View Channels", width = NULL, solidHeader = TRUE,
          uiOutput(ns("comparison_channel_ui"))
        )
      ),

      # Right: matrix heatmap + editable table
      column(8,
        box(
          title = "Spillover Matrix", width = NULL, solidHeader = TRUE,
          tabsetPanel(
            tabPanel("Heatmap",
              withSpinner(
                plotlyOutput(ns("spillover_heatmap"), height = "320px"),
                color = "#00B4D8"
              )
            ),
            tabPanel("Edit Matrix",
              tags$p(style = "font-size: 11px; color: #8899AA; margin: 6px 0;",
                     "Values on the diagonal should be 1. Click any cell to edit."),
              withSpinner(
                rHandsontableOutput(ns("spillover_table"), height = "300px"),
                color = "#00B4D8"
              )
            )
          )
        ),

        box(
          title = "Before / After Comparison", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(6,
              tags$p(style = "font-size: 11px; color: #5A7A8A; text-align: center;", "Before Compensation"),
              withSpinner(plotlyOutput(ns("before_plot"), height = "250px"), color = "#00B4D8")
            ),
            column(6,
              tags$p(style = "font-size: 11px; color: #5A7A8A; text-align: center;", "After Compensation"),
              withSpinner(plotlyOutput(ns("after_plot"), height = "250px"), color = "#00B4D8")
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
  volumes <- c(Home = path.expand("~"), getVolumes()())
  shinyDirChoose(input, "controls_folder", roots = volumes, session = session)

  local <- reactiveValues(
    spillover    = NULL,
    controls_path = NULL
  )

  output$controls_folder_label <- renderUI({
    if (!is.null(local$controls_path)) {
      tags$p(style = "font-size: 11px; color: #2EC4B6; margin: 4px 0;",
             icon("check-circle"), " ", basename(local$controls_path))
    }
  })

  observeEvent(input$controls_folder, {
    req(is.list(input$controls_folder))
    path <- parseDirPath(volumes, input$controls_folder)
    if (length(path) > 0) local$controls_path <- path
  })

  # ── Load / compute spillover matrix ──────────────────────────────────────
  observeEvent(input$load_matrix_btn, {
    req(shared$raw_flowset)
    shared$status <- "busy"

    withProgress(message = "Loading compensation matrix...", value = 0, {
      fs <- shared$raw_flowset
      incProgress(0.3)

      mat <- tryCatch({
        if (input$comp_source == "embedded") {
          sp <- flowCore::spillover(fs[[1]])
          if (is.null(sp)) {
            showNotification("No embedded spillover matrix found in this FCS file.", type = "warning", duration = 5)
            NULL
          } else {
            # Use first non-null spillover keyword
            if (is.list(sp)) sp[[which(!sapply(sp, is.null))[1]]] else sp
          }

        } else if (input$comp_source == "upload") {
          req(input$spillover_csv)
          read.csv(input$spillover_csv$datapath, row.names = 1, check.names = FALSE)

        } else if (input$comp_source == "controls") {
          req(local$controls_path)
          ctrl_files <- list.files(local$controls_path, pattern = "\\.fcs$",
                                   full.names = TRUE, ignore.case = TRUE)
          req(length(ctrl_files) > 0)
          ctrl_fs <- flowCore::read.flowSet(files = ctrl_files, transformation = FALSE)
          safe_cyto(
            CytoExploreR::cyto_spillover_compute(ctrl_fs),
            "Failed to compute spillover from controls"
          )
        }
      }, error = function(e) {
        showNotification(paste("Matrix error:", conditionMessage(e)), type = "error", duration = 6)
        NULL
      })

      incProgress(0.7)
      if (!is.null(mat)) {
        local$spillover         <- as.matrix(mat)
        shared$spillover_matrix <- local$spillover
        showNotification("Spillover matrix loaded.", type = "message", duration = 3)
      }
    })

    shared$status <- "idle"
  })

  # ── Heatmap ───────────────────────────────────────────────────────────────
  output$spillover_heatmap <- renderPlotly({
    req(local$spillover)
    mat <- local$spillover
    p <- plot_ly(
      z         = mat,
      x         = colnames(mat),
      y         = rownames(mat),
      type      = "heatmap",
      colorscale = list(
        list(0,   "#0D1B2A"),
        list(0.5, "#00B4D8"),
        list(1,   "#2EC4B6")
      ),
      zmin = 0, zmax = 1,
      hovertemplate = "%{y} → %{x}: %{z:.4f}<extra></extra>"
    ) %>%
      plotly_dark_layout(title = "Spillover Matrix") %>%
      layout(
        xaxis = list(tickangle = -45),
        margin = list(b = 80, l = 80)
      )
    p
  })

  # ── Editable handsontable ─────────────────────────────────────────────────
  output$spillover_table <- renderRHandsontable({
    req(local$spillover)
    rhandsontable(
      as.data.frame(local$spillover),
      readOnly = FALSE,
      width    = "100%"
    ) %>%
      hot_table(stretchH = "all") %>%
      hot_cols(renderer = "
        function(instance, td, row, col, prop, value, cellProperties) {
          Handsontable.renderers.NumericRenderer.apply(this, arguments);
          if (row === col) { td.style.background = '#1A3A50'; td.style.color = '#00B4D8'; }
          else if (value > 0.1) { td.style.background = '#3A2010'; }
        }
      ")
  })

  # Update matrix from table edits
  observeEvent(input$spillover_table, {
    req(input$spillover_table)
    df <- hot_to_r(input$spillover_table)
    mat <- as.matrix(df)
    local$spillover         <- mat
    shared$spillover_matrix <- mat
  })

  # ── Apply compensation ────────────────────────────────────────────────────
  observeEvent(input$apply_comp_btn, {
    req(shared$raw_flowset, local$spillover)
    shared$status <- "busy"

    withProgress(message = "Applying compensation...", value = 0, {
      incProgress(0.3)
      comp_fs <- tryCatch({
        mat <- local$spillover
        # Build compensation object
        comp_obj <- flowCore::compensation(mat)
        flowCore::compensate(shared$raw_flowset, comp_obj)
      }, error = function(e) {
        showNotification(paste("Compensation error:", conditionMessage(e)), type = "error", duration = 6)
        NULL
      })
      incProgress(0.7)

      if (!is.null(comp_fs)) {
        shared$comp_flowset <- comp_fs
        showNotification("Compensation applied successfully.", type = "message", duration = 3)
      }
    })

    shared$status <- "idle"
  })

  # ── Comparison channel selectors ──────────────────────────────────────────
  output$comparison_channel_ui <- renderUI({
    req(shared$raw_flowset)
    channels <- colnames(shared$raw_flowset)
    fluor_ch <- grep("^(FSC|SSC)", channels, invert = TRUE, value = TRUE, ignore.case = TRUE)
    if (length(fluor_ch) < 2) fluor_ch <- channels
    tagList(
      selectInput(ns("comp_ch_x"), "X Axis", choices = fluor_ch, selected = fluor_ch[1]),
      selectInput(ns("comp_ch_y"), "Y Axis", choices = fluor_ch,
                  selected = if (length(fluor_ch) > 1) fluor_ch[2] else fluor_ch[1])
    )
  })

  # ── Before / After scatter plots ──────────────────────────────────────────
  make_scatter <- function(fs, x_ch, y_ch, title) {
    req(fs, x_ch, y_ch)
    tryCatch({
      ff   <- fs[[1]]
      expr <- as.data.frame(exprs(ff))
      req(x_ch %in% names(expr), y_ch %in% names(expr))
      # Subsample for speed
      n   <- min(nrow(expr), 3000)
      idx <- sample(nrow(expr), n)
      df  <- expr[idx, ]

      plot_ly(
        data = df,
        x    = ~get(x_ch),
        y    = ~get(y_ch),
        type = "scatter",
        mode = "markers",
        marker = list(
          size    = 2,
          opacity = 0.5,
          color   = "#00B4D8"
        ),
        hoverinfo = "none"
      ) %>%
        plotly_dark_layout(title = title, xlab = x_ch, ylab = y_ch)
    }, error = function(e) {
      plot_ly() %>% layout(title = list(text = "Error", font = list(color = "#C0392B")),
                           paper_bgcolor = "#0D1B2A", plot_bgcolor = "#1B2A3B")
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

  output$comp_status_ui <- renderUI({
    if (!is.null(shared$comp_flowset)) {
      tags$p(style = "font-size: 12px; color: #2EC4B6; margin-top: 8px;",
             icon("check"), " Compensation applied")
    }
  })
}
