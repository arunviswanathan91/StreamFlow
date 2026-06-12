# StreamFLOW - mod_transformation.R
# Per-channel transformations using the full CytoExploreR transformer API
# CytoExploreR: cyto_transform, cyto_transformer_logicle, cyto_transformer_arcsinh,
#               cyto_transformer_biex, cyto_transformer_log, cyto_transformer_combine,
#               cyto_transformer_extract, cyto_transform_extract,
#               cyto_fluor_channels, cyto_calibrate, cyto_calibrate_reset

# ── UI ────────────────────────────────────────────────────────────────────────
transformationUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      column(5,
        box(
          title = "Transform Settings", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(6,
              selectInput(ns("global_method"), "Quick-set All Channels",
                          choices = c("— individual —" = "individual",
                                      "All Logicle"    = "logicle",
                                      "All Arcsinh"    = "arcsinh",
                                      "All Log"        = "log",
                                      "All BiEx"       = "biex",
                                      "None"           = "none"),
                          selected = "individual")
            ),
            column(6,
              numericInput(ns("arcsinh_global_cf"), "Global Arcsinh Cofactor",
                           value = 150, min = 1, max = 1000)
            )
          ),
          tags$hr(),
          uiOutput(ns("transform_controls")),
          tags$hr(),
          fluidRow(
            column(4,
              actionButton(ns("apply_trans_btn"),
                           tagList(icon("bolt"), " Apply"),
                           class = "btn btn-primary btn-block")
            ),
            column(4,
              actionButton(ns("reset_trans_btn"),
                           tagList(icon("undo"), " Reset"),
                           class = "btn btn-default btn-block")
            ),
            column(4,
              actionButton(ns("extract_trans_btn"),
                           tagList(icon("download"), " Extract"),
                           class = "btn btn-default btn-block",
                           title = "cyto_transformer_extract / cyto_transform_extract")
            )
          )
        ),

        box(
          title = "Calibration (cyto_calibrate)", width = NULL, solidHeader = TRUE,
          uiOutput(ns("calibrate_channel_ui")),
          fluidRow(
            column(6,
              # value = NULL renders an empty field without emitting the browser
              # "value 'NA' cannot be parsed" warning that value = NA produces.
              numericInput(ns("cal_min"), "Min", value = NULL)
            ),
            column(6,
              numericInput(ns("cal_max"), "Max", value = NULL)
            )
          ),
          fluidRow(
            column(6,
              actionButton(ns("calibrate_btn"), tagList(icon("sliders-h"), " Calibrate"),
                           class = "btn btn-default btn-block btn-sm")
            ),
            column(6,
              actionButton(ns("calibrate_reset_btn"), tagList(icon("times"), " Reset"),
                           class = "btn btn-danger btn-block btn-sm")
            )
          )
        )
      ),

      column(7,
        box(
          title = "Distribution Preview", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(6, uiOutput(ns("preview_channel_ui"))),
            column(6, uiOutput(ns("preview_sample_ui")))
          ),
          withSpinner(plotlyOutput(ns("transform_preview"), height = "360px"),
                      color = "#00B4D8"),
          uiOutput(ns("transform_status_ui"))
        ),

        box(
          title = "Transformer Definitions", width = NULL, solidHeader = TRUE,
          verbatimTextOutput(ns("transformer_info")),
          tags$p(style = "font-size:11px;color:#5A7A8A;",
                 "Output of cyto_transformer_extract() / cyto_transform_extract() after Apply.")
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
transformationServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    transformer_list = NULL,    # combined cytoTransformerList
    settings         = list()   # per-channel method/params cache
  )

  best_fs <- reactive({
    shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Dynamic per-channel controls ──────────────────────────────────────────
  output$transform_controls <- renderUI({
    channels <- tryCatch(CytoExploreR::cyto_fluor_channels(best_fs()),
                         error = function(e) {
                           ch <- shared$channels
                           grep("^(FSC|SSC)", ch, invert = TRUE, value = TRUE, ignore.case = TRUE)
                         })
    req(channels, length(channels) > 0)

    lapply(channels, function(ch) {
      sid <- gsub("[^a-zA-Z0-9]", "_", ch)

      # Determine default from quick-set
      default_method <- switch(input$global_method %||% "individual",
        logicle    = "logicle",
        arcsinh    = "arcsinh",
        log        = "log",
        biex       = "biex",
        none       = "none",
        "logicle"
      )

      tags$div(
        style = "border:1px solid #243447;border-radius:4px;padding:10px;margin-bottom:8px;background:#243447;",
        tags$div(
          style = "display:flex;align-items:center;justify-content:space-between;margin-bottom:6px;",
          tags$span(style = "font-size:12px;font-weight:600;color:#E0E0E0;", ch),
          selectInput(ns(paste0("method_", sid)), label = NULL,
                      choices  = c("None"          = "none",
                                   "Logicle"       = "logicle",
                                   "Arcsinh"       = "arcsinh",
                                   "Log"           = "log",
                                   "BiExponential" = "biex"),
                      selected = default_method,
                      width    = "145px")
        ),
        # Logicle
        conditionalPanel(
          condition = sprintf("input['%s'] == 'logicle'", ns(paste0("method_", sid))),
          fluidRow(
            column(6, sliderInput(ns(paste0("lgcl_w_", sid)), "w", 0.1, 2.0, 0.5, 0.1, ticks = FALSE)),
            column(6, sliderInput(ns(paste0("lgcl_t_", sid)), "t", 1e3, 1e6, 262144, 1e3, ticks = FALSE))
          ),
          fluidRow(
            column(6, sliderInput(ns(paste0("lgcl_m_", sid)), "m", 3, 7, 4.5, 0.5, ticks = FALSE)),
            column(6, sliderInput(ns(paste0("lgcl_a_", sid)), "a", 0, 2, 0,   0.1, ticks = FALSE))
          )
        ),
        # Arcsinh
        conditionalPanel(
          condition = sprintf("input['%s'] == 'arcsinh'", ns(paste0("method_", sid))),
          sliderInput(ns(paste0("arcsinh_cf_", sid)), "Cofactor",
                      1, 500, input$arcsinh_global_cf %||% 150, 1, ticks = FALSE)
        ),
        # Log
        conditionalPanel(
          condition = sprintf("input['%s'] == 'log'", ns(paste0("method_", sid))),
          sliderInput(ns(paste0("log_dec_", sid)), "Decades", 1, 6, 4, 0.5, ticks = FALSE)
        )
      )
    })
  })

  # ── Preview selectors ─────────────────────────────────────────────────────
  output$preview_channel_ui <- renderUI({
    channels <- tryCatch(CytoExploreR::cyto_fluor_channels(best_fs()),
                         error = function(e) shared$channels)
    req(channels)
    selectInput(ns("preview_channel"), "Preview Channel",
                choices = channels, selected = channels[1])
  })

  output$preview_sample_ui <- renderUI({
    fs <- best_fs()
    req(fs)
    sn <- tryCatch(CytoExploreR::cyto_names(fs), error = function(e) sampleNames(fs))
    selectInput(ns("preview_sample"), "Sample", choices = sn, selected = sn[1])
  })

  # ── Debounced histogram preview ───────────────────────────────────────────
  preview_deps <- reactive({
    list(input$preview_channel, input$preview_sample, input$global_method)
  }) %>% debounce(500)

  output$transform_preview <- renderPlotly({
    preview_deps()
    ch      <- input$preview_channel
    samp    <- input$preview_sample
    fs_raw  <- best_fs()
    fs_trans <- shared$trans_flowset

    req(ch, samp, fs_raw)
    req(ch %in% colnames(fs_raw), samp %in% sampleNames(fs_raw))

    raw_vals   <- tryCatch(exprs(fs_raw[[samp]])[, ch], error = function(e) NULL)
    req(raw_vals)
    trans_vals <- tryCatch(
      if (!is.null(fs_trans) && ch %in% colnames(fs_trans) && samp %in% sampleNames(fs_trans))
        exprs(fs_trans[[samp]])[, ch]
      else NULL,
      error = function(e) NULL
    )

    p <- plot_ly() %>%
      add_histogram(x = raw_vals, name = "Before", nbinsx = 100,
                    marker = list(color = "#00B4D8", opacity = 0.6))
    if (!is.null(trans_vals))
      p <- p %>%
        add_histogram(x = trans_vals, name = "After", nbinsx = 100,
                      marker = list(color = "#2EC4B6", opacity = 0.6))

    p %>%
      plotly_dark_layout(title = paste("Distribution:", ch), xlab = ch, ylab = "Count") %>%
      layout(barmode = "overlay")
  })

  # ── Build transformer list ────────────────────────────────────────────────
  build_transformers <- function(fs) {
    channels <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),
                         error = function(e) {
                           ch <- colnames(fs)
                           grep("^(FSC|SSC)", ch, invert = TRUE, value = TRUE, ignore.case = TRUE)
                         })

    trans_pieces <- list()

    for (ch in channels) {
      sid    <- gsub("[^a-zA-Z0-9]", "_", ch)
      method <- input[[paste0("method_", sid)]] %||% "logicle"

      piece <- tryCatch(switch(method,
        logicle = safe_cyto(
          CytoExploreR::cyto_transformer_logicle(
            fs,
            channels = ch,
            w = input[[paste0("lgcl_w_", sid)]] %||% 0.5,
            t = input[[paste0("lgcl_t_", sid)]] %||% 262144,
            m = input[[paste0("lgcl_m_", sid)]] %||% 4.5,
            a = input[[paste0("lgcl_a_", sid)]] %||% 0
          ),
          paste("cyto_transformer_logicle failed for", ch)
        ),
        arcsinh = safe_cyto(
          CytoExploreR::cyto_transformer_arcsinh(
            fs,
            channels  = ch,
            cofactor = input[[paste0("arcsinh_cf_", sid)]] %||% 150
          ),
          paste("cyto_transformer_arcsinh failed for", ch)
        ),
        log = safe_cyto(
          CytoExploreR::cyto_transformer_log(
            fs,
            channels = ch,
            t        = 10 ^ (input[[paste0("log_dec_", sid)]] %||% 4)
          ),
          paste("cyto_transformer_log failed for", ch)
        ),
        biex = safe_cyto(
          CytoExploreR::cyto_transformer_biex(
            fs,
            channels = ch
          ),
          paste("cyto_transformer_biex failed for", ch)
        ),
        none = NULL
      ), error = function(e) NULL)

      if (!is.null(piece)) trans_pieces <- c(trans_pieces, list(piece))
    }

    if (length(trans_pieces) == 0) return(NULL)

    # Combine all transformers
    if (length(trans_pieces) == 1) {
      trans_pieces[[1]]
    } else {
      tryCatch(
        safe_cyto(
          Reduce(function(a, b) CytoExploreR::cyto_transformer_combine(a, b),
                 trans_pieces),
          "cyto_transformer_combine failed"
        ),
        error = function(e) trans_pieces[[1]]
      )
    }
  }

  # ── Apply transforms ──────────────────────────────────────────────────────
  observeEvent(input$apply_trans_btn, {
    fs <- best_fs()
    req(fs)
    shared$status <- "busy"

    withProgress(message = "Building transformers (cyto_transformer_*)…", value = 0, {
      incProgress(0.2)

      transformer <- tryCatch(
        build_transformers(fs),
        error = function(e) {
          showNotification(paste("Transformer build error:", conditionMessage(e)),
                           type = "error", duration = 6)
          NULL
        }
      )

      if (is.null(transformer)) {
        shared$status <- "idle"
        return()
      }

      incProgress(0.3, detail = "cyto_transform()…")

      trans_fs <- tryCatch(
        {
          # cyto_transform — the high-level CytoExploreR apply function
          result <- safe_cyto(
            CytoExploreR::cyto_transform(fs, transformer = transformer),
            "cyto_transform failed"
          )
          if (is.null(result)) {
            # Fallback: apply each component via flowCore
            showNotification("cyto_transform() unavailable, applying via flowCore.",
                             type = "warning", duration = 3)
            apply_transforms_flowcore(fs)
          } else result
        },
        error = function(e) {
          showNotification(paste("Transform error:", conditionMessage(e)),
                           type = "error", duration = 6)
          NULL
        }
      )

      incProgress(0.5, detail = "Done!")
      if (!is.null(trans_fs)) {
        shared$trans_flowset     <- trans_fs
        local$transformer_list   <- transformer
        shared$transforms        <- transformer
        showNotification("Transformations applied via cyto_transform().",
                         type = "message", duration = 3)
      }
    })

    shared$status <- "idle"
  })

  # Fallback: apply transforms via flowCore
  apply_transforms_flowcore <- function(fs) {
    channels <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),
                         error = function(e) {
                           ch <- colnames(fs)
                           grep("^(FSC|SSC)", ch, invert = TRUE, value = TRUE, ignore.case = TRUE)
                         })
    result_fs <- fs
    for (ch in channels) {
      sid    <- gsub("[^a-zA-Z0-9]", "_", ch)
      method <- input[[paste0("method_", sid)]] %||% "logicle"
      fn     <- tryCatch(switch(method,
        logicle = flowCore::logicleTransform(
          transformationId = paste0("logicle_", ch),
          w = input[[paste0("lgcl_w_", sid)]] %||% 0.5,
          t = input[[paste0("lgcl_t_", sid)]] %||% 262144,
          m = input[[paste0("lgcl_m_", sid)]] %||% 4.5,
          a = input[[paste0("lgcl_a_", sid)]] %||% 0
        ),
        arcsinh = flowCore::arcsinhTransform(
          transformationId = paste0("arcsinh_", ch),
          a = 0, b = 1 / (input[[paste0("arcsinh_cf_", sid)]] %||% 150), c = 0
        ),
        biex = flowCore::biexponentialTransform(transformationId = paste0("biex_", ch)),
        NULL
      ), error = function(e) NULL)
      if (!is.null(fn))
        result_fs <- tryCatch(
          flowCore::transform(result_fs, flowCore::transformList(ch, fn)),
          error = function(e) result_fs
        )
    }
    result_fs
  }

  # ── Reset ─────────────────────────────────────────────────────────────────
  observeEvent(input$reset_trans_btn, {
    shared$trans_flowset   <- NULL
    shared$transforms      <- list()
    local$transformer_list <- NULL
    showNotification("Transformations reset.", type = "message", duration = 2)
  })

  # ── Extract transformer info ───────────────────────────────────────────────
  observeEvent(input$extract_trans_btn, {
    fs <- shared$trans_flowset %||% best_fs()
    req(fs)

    info <- tryCatch({
      # Try cyto_transformer_extract on GatingSet
      gs <- shared$gating_set
      if (!is.null(gs)) {
        te <- safe_cyto(CytoExploreR::cyto_transformer_extract(gs),
                        "cyto_transformer_extract failed")
        if (!is.null(te)) paste(capture.output(print(te)), collapse = "\n")
        else "No transformers on GatingSet"
      } else if (!is.null(local$transformer_list)) {
        paste(capture.output(print(local$transformer_list)), collapse = "\n")
      } else {
        "Apply transforms first."
      }
    }, error = function(e) paste("Error:", conditionMessage(e)))

    output$transformer_info <- renderText(info)
    showNotification("Transformer info extracted.", type = "message", duration = 2)
  })

  output$transformer_info <- renderText({
    if (!is.null(local$transformer_list))
      tryCatch(paste(capture.output(print(local$transformer_list)), collapse = "\n"),
               error = function(e) "Transformer applied.")
    else
      "Click 'Extract' after applying transforms."
  })

  output$transform_status_ui <- renderUI({
    if (!is.null(shared$trans_flowset))
      tags$p(style = "font-size:12px;color:#2EC4B6;margin-top:6px;",
             icon("check"), " cyto_transform() applied to ",
             length(shared$trans_flowset), " samples")
  })

  # ── Calibration ───────────────────────────────────────────────────────────
  output$calibrate_channel_ui <- renderUI({
    channels <- shared$channels
    req(channels)
    selectInput(ns("cal_channel"), "Channel", choices = channels, selected = channels[1])
  })

  observeEvent(input$calibrate_btn, {
    ch  <- input$cal_channel
    mn  <- input$cal_min
    mx  <- input$cal_max
    req(ch)
    # Guard against empty/NA bounds — passing NA into cyto_calibrate is meaningless.
    if (!is.numeric(mn) || !is.numeric(mx) || !is.finite(mn) || !is.finite(mx)) {
      showNotification("Enter numeric Min and Max values before calibrating.",
                       type = "warning", duration = 4)
      return()
    }
    if (mn >= mx) {
      showNotification("Calibration Min must be less than Max.",
                       type = "warning", duration = 4)
      return()
    }
    tryCatch(
      safe_cyto(
        CytoExploreR::cyto_calibrate(channel = ch, minimum = mn, maximum = mx),
        "cyto_calibrate failed"
      ),
      error = function(e) NULL
    )
    showNotification(paste("Calibration set for", ch), type = "message", duration = 2)
  })

  observeEvent(input$calibrate_reset_btn, {
    tryCatch(
      safe_cyto(CytoExploreR::cyto_calibrate_reset(), "cyto_calibrate_reset failed"),
      error = function(e) NULL
    )
    showNotification("Calibration reset.", type = "message", duration = 2)
  })
}
