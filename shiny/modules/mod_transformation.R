# StreamFLOW - mod_transformation.R
# Per-channel transform selection with parameter sliders and live preview

# ── UI ────────────────────────────────────────────────────────────────────────
transformationUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left: per-channel transform controls
      column(5,
        box(
          title = "Transform Settings", width = NULL, solidHeader = TRUE,
          uiOutput(ns("transform_controls")),
          tags$hr(),
          fluidRow(
            column(6,
              actionButton(
                ns("apply_trans_btn"),
                label = tagList(icon("bolt"), " Apply All"),
                class = "btn btn-primary btn-block"
              )
            ),
            column(6,
              actionButton(
                ns("reset_trans_btn"),
                label = tagList(icon("undo"), " Reset"),
                class = "btn btn-default btn-block"
              )
            )
          )
        )
      ),

      # Right: histogram preview
      column(7,
        box(
          title = "Distribution Preview", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(6,
              uiOutput(ns("preview_channel_ui"))
            ),
            column(6,
              uiOutput(ns("preview_sample_ui"))
            )
          ),
          withSpinner(
            plotlyOutput(ns("transform_preview"), height = "380px"),
            color = "#00B4D8"
          ),
          uiOutput(ns("transform_status_ui"))
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
transformationServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    transform_list = list()  # named list: channel -> list(method, params)
  )

  # ── Channel controls ─────────────────────────────────────────────────────
  output$transform_controls <- renderUI({
    channels <- shared$channels
    if (is.null(channels) || length(channels) == 0) {
      return(tags$p(style = "color: #5A7A8A; font-size: 12px;", "Load FCS files first."))
    }

    # Only show fluorescence channels by default
    fluor_ch <- grep("^(FSC|SSC)", channels, invert = TRUE, value = TRUE, ignore.case = TRUE)
    if (length(fluor_ch) == 0) fluor_ch <- channels

    lapply(fluor_ch, function(ch) {
      safe_id <- gsub("[^a-zA-Z0-9]", "_", ch)
      tagList(
        tags$div(
          style = "border: 1px solid #243447; border-radius: 4px; padding: 10px; margin-bottom: 8px; background: #243447;",
          tags$div(
            style = "display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px;",
            tags$span(style = "font-size: 12px; font-weight: 600; color: #E0E0E0;", ch),
            selectInput(
              ns(paste0("method_", safe_id)),
              label   = NULL,
              choices = c("None" = "none", "Logicle" = "logicle",
                          "Arcsinh" = "arcsinh", "Log" = "log", "BiExponential" = "biex"),
              selected = "logicle",
              width = "140px"
            )
          ),

          # Logicle parameters
          conditionalPanel(
            condition = sprintf("input['%s'] == 'logicle'", ns(paste0("method_", safe_id))),
            fluidRow(
              column(6, sliderInput(ns(paste0("lgcl_w_",  safe_id)), "w (width)",  0.1, 2.0, 0.5, 0.1, ticks = FALSE)),
              column(6, sliderInput(ns(paste0("lgcl_t_",  safe_id)), "t (top)",    1e3, 1e6, 2.62e5, 1e4, ticks = FALSE))
            ),
            fluidRow(
              column(6, sliderInput(ns(paste0("lgcl_m_",  safe_id)), "m (range)", 3, 7, 4.5, 0.5, ticks = FALSE)),
              column(6, sliderInput(ns(paste0("lgcl_a_",  safe_id)), "a (neg)",   0, 2, 0,   0.1, ticks = FALSE))
            )
          ),

          # Arcsinh parameters
          conditionalPanel(
            condition = sprintf("input['%s'] == 'arcsinh'", ns(paste0("method_", safe_id))),
            sliderInput(ns(paste0("arcsinh_cf_", safe_id)), "Cofactor", 1, 500, 150, 1, ticks = FALSE)
          ),

          # Log parameters
          conditionalPanel(
            condition = sprintf("input['%s'] == 'log'", ns(paste0("method_", safe_id))),
            sliderInput(ns(paste0("log_dec_", safe_id)), "Decades", 1, 6, 4, 0.5, ticks = FALSE)
          )
        )
      )
    })
  })

  # ── Preview channel / sample selectors ───────────────────────────────────
  output$preview_channel_ui <- renderUI({
    channels <- shared$channels
    req(channels)
    fluor_ch <- grep("^(FSC|SSC)", channels, invert = TRUE, value = TRUE, ignore.case = TRUE)
    if (length(fluor_ch) == 0) fluor_ch <- channels
    selectInput(ns("preview_channel"), "Preview Channel", choices = fluor_ch,
                selected = fluor_ch[1])
  })

  output$preview_sample_ui <- renderUI({
    fs <- shared$comp_flowset %||% shared$raw_flowset
    req(fs)
    selectInput(ns("preview_sample"), "Sample",
                choices = sampleNames(fs), selected = sampleNames(fs)[1])
  })

  # ── Live preview histogram (debounced) ────────────────────────────────────
  preview_trigger <- reactive({
    list(input$preview_channel, input$preview_sample,
         # Include key slider inputs to re-trigger
         input[[paste0("method_", gsub("[^a-zA-Z0-9]", "_", input$preview_channel %||% ""))]])
  }) %>% debounce(500)

  output$transform_preview <- renderPlotly({
    preview_trigger()

    ch      <- input$preview_channel
    samp    <- input$preview_sample
    fs_raw  <- shared$comp_flowset %||% shared$raw_flowset
    fs_trans <- shared$trans_flowset

    req(ch, samp, fs_raw)
    req(ch %in% colnames(fs_raw), samp %in% sampleNames(fs_raw))

    raw_vals <- tryCatch(exprs(fs_raw[[samp]])[, ch], error = function(e) NULL)
    req(raw_vals)

    # Build transformed values if trans_flowset exists
    trans_vals <- tryCatch({
      if (!is.null(fs_trans) && ch %in% colnames(fs_trans) && samp %in% sampleNames(fs_trans)) {
        exprs(fs_trans[[samp]])[, ch]
      } else {
        NULL
      }
    }, error = function(e) NULL)

    # Histogram
    p <- plot_ly() %>%
      add_histogram(
        x = raw_vals,
        name = "Before",
        nbinsx = 100,
        marker = list(color = "#00B4D8", opacity = 0.6)
      )

    if (!is.null(trans_vals)) {
      p <- p %>%
        add_histogram(
          x = trans_vals,
          name = "After",
          nbinsx = 100,
          marker = list(color = "#2EC4B6", opacity = 0.6)
        )
    }

    p %>%
      plotly_dark_layout(
        title = paste("Distribution:", ch),
        xlab  = ch,
        ylab  = "Count"
      ) %>%
      layout(barmode = "overlay")
  })

  # ── Collect transform settings ────────────────────────────────────────────
  get_transform_settings <- function() {
    channels <- shared$channels
    req(channels)
    fluor_ch <- grep("^(FSC|SSC)", channels, invert = TRUE, value = TRUE, ignore.case = TRUE)
    if (length(fluor_ch) == 0) fluor_ch <- channels

    settings <- list()
    for (ch in fluor_ch) {
      safe_id <- gsub("[^a-zA-Z0-9]", "_", ch)
      method  <- input[[paste0("method_", safe_id)]] %||% "logicle"
      params  <- switch(method,
        logicle = list(
          w = input[[paste0("lgcl_w_", safe_id)]] %||% 0.5,
          t = input[[paste0("lgcl_t_", safe_id)]] %||% 262144,
          m = input[[paste0("lgcl_m_", safe_id)]] %||% 4.5,
          a = input[[paste0("lgcl_a_", safe_id)]] %||% 0
        ),
        arcsinh = list(
          cofactor = input[[paste0("arcsinh_cf_", safe_id)]] %||% 150
        ),
        log = list(
          decades = input[[paste0("log_dec_", safe_id)]] %||% 4
        ),
        biex = list(),
        none = list()
      )
      settings[[ch]] <- list(method = method, params = params)
    }
    settings
  }

  # ── Apply transforms ──────────────────────────────────────────────────────
  observeEvent(input$apply_trans_btn, {
    fs <- shared$comp_flowset %||% shared$raw_flowset
    req(fs)
    shared$status <- "busy"

    withProgress(message = "Applying transformations...", value = 0, {
      settings <- get_transform_settings()
      incProgress(0.2)

      trans_fs <- tryCatch({
        result_fs <- fs

        channels_to_transform <- names(settings)
        n_ch <- length(channels_to_transform)

        for (i in seq_along(channels_to_transform)) {
          ch     <- channels_to_transform[[i]]
          cfg    <- settings[[ch]]
          method <- cfg$method
          params <- cfg$params

          incProgress(0.6 / n_ch, detail = paste("Transforming", ch))

          if (method == "none") next

          # Build transform function
          trans_fn <- tryCatch(switch(method,
            logicle = flowCore::logicleTransform(
              transformationId = paste0("logicle_", ch),
              w = params$w, t = params$t, m = params$m, a = params$a
            ),
            arcsinh = flowCore::arcsinhTransform(
              transformationId = paste0("arcsinh_", ch),
              a = 0, b = 1 / params$cofactor, c = 0
            ),
            log = flowCore::truncateTransform(
              transformationId = paste0("trunc_", ch), a = 1
            ),
            biex = flowCore::biexponentialTransform(
              transformationId = paste0("biex_", ch)
            )
          ), error = function(e) NULL)

          if (!is.null(trans_fn)) {
            trans_list <- flowCore::transformList(ch, trans_fn)
            result_fs  <- tryCatch(
              flowCore::transform(result_fs, trans_list),
              error = function(e) {
                showNotification(
                  paste("Warning: could not transform", ch, "-", conditionMessage(e)),
                  type = "warning", duration = 4
                )
                result_fs
              }
            )
          }
        }

        result_fs
      }, error = function(e) {
        showNotification(paste("Transform error:", conditionMessage(e)), type = "error", duration = 6)
        NULL
      })

      incProgress(0.2, detail = "Done!")

      if (!is.null(trans_fs)) {
        shared$trans_flowset <- trans_fs
        local$transform_list <- settings
        shared$transforms    <- settings
        showNotification("Transformations applied.", type = "message", duration = 3)
      }
    })

    shared$status <- "idle"
  })

  # ── Reset ─────────────────────────────────────────────────────────────────
  observeEvent(input$reset_trans_btn, {
    shared$trans_flowset <- NULL
    shared$transforms    <- list()
    local$transform_list <- list()
    showNotification("Transformations reset.", type = "message", duration = 2)
  })

  output$transform_status_ui <- renderUI({
    if (!is.null(shared$trans_flowset)) {
      tags$p(
        style = "font-size: 12px; color: #2EC4B6; margin-top: 6px;",
        icon("check"), " Transformations applied to ",
        length(shared$trans_flowset), " samples"
      )
    }
  })
}
