# StreamFLOW - mod_statistics.R
# Population statistics computation and export

# ── UI ────────────────────────────────────────────────────────────────────────
statisticsUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Controls
      column(3,
        box(
          title = "Statistics Settings", width = NULL, solidHeader = TRUE,

          checkboxGroupInput(
            ns("stat_types"),
            "Statistics to Compute",
            choices = c(
              "Count"            = "count",
              "% of Parent"      = "freq_parent",
              "% of Total"       = "freq_total",
              "MFI (mean)"       = "mfi",
              "Geometric MFI"    = "gmfi",
              "CV (%)"           = "cv"
            ),
            selected = c("count", "freq_parent", "mfi")
          ),

          uiOutput(ns("mfi_channel_ui")),

          tags$hr(),

          actionButton(
            ns("compute_btn"),
            tagList(icon("calculator"), " Compute Statistics"),
            class = "btn btn-primary btn-block"
          ),

          tags$hr(),

          fluidRow(
            column(6,
              downloadButton(ns("dl_csv"),   "CSV",   class = "btn btn-default btn-block btn-sm")
            ),
            column(6,
              downloadButton(ns("dl_excel"), "Excel", class = "btn btn-default btn-block btn-sm")
            )
          ),

          uiOutput(ns("compute_status_ui"))
        ),

        box(
          title = "Filter Results", width = NULL, solidHeader = TRUE,
          uiOutput(ns("filter_pop_ui")),
          uiOutput(ns("filter_group_ui"))
        )
      ),

      # Results
      column(9,
        box(
          title = "Statistics Table", width = NULL, solidHeader = TRUE,
          DTOutput(ns("stats_table"))
        ),
        box(
          title = "Bar Chart", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(4,
              uiOutput(ns("bar_stat_ui"))
            ),
            column(4,
              uiOutput(ns("bar_pop_ui"))
            ),
            column(4,
              selectInput(ns("bar_group_by"), "Group By",
                          choices = c("Sample" = "SampleName", "Group" = "Group"))
            )
          ),
          withSpinner(
            plotlyOutput(ns("bar_chart"), height = "320px"),
            color = "#00B4D8"
          )
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
statisticsServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    stats_df    = NULL,
    stats_raw   = NULL
  )

  best_fs <- reactive({
    shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
  })

  # ── Dynamic UI ────────────────────────────────────────────────────────────
  output$mfi_channel_ui <- renderUI({
    req(any(c("mfi", "gmfi", "cv") %in% (input$stat_types %||% character())))
    channels <- shared$channels
    req(channels)
    fluor_ch <- grep("^(FSC|SSC)", channels, invert = TRUE, value = TRUE, ignore.case = TRUE)
    if (length(fluor_ch) == 0) fluor_ch <- channels
    selectInput(
      ns("mfi_channels"), "Channels for MFI/CV",
      choices  = fluor_ch,
      selected = fluor_ch,
      multiple = TRUE
    )
  })

  output$filter_pop_ui <- renderUI({
    df <- local$stats_df
    if (is.null(df) || !"Population" %in% names(df)) return(NULL)
    pops <- unique(df$Population)
    selectInput(ns("filter_pop"), "Filter Population",
                choices  = c("All" = "all", pops),
                selected = "all",
                multiple = FALSE)
  })

  output$filter_group_ui <- renderUI({
    df <- local$stats_df
    if (is.null(df) || !"Group" %in% names(df)) return(NULL)
    groups <- unique(df$Group)
    selectInput(ns("filter_group"), "Filter Group",
                choices  = c("All" = "all", groups),
                selected = "all",
                multiple = FALSE)
  })

  # ── Compute statistics ────────────────────────────────────────────────────
  observeEvent(input$compute_btn, {
    gs <- shared$gating_set
    fs <- best_fs()
    req(fs)

    shared$status <- "busy"

    withProgress(message = "Computing statistics...", value = 0, {
      result <- tryCatch({
        stat_types <- input$stat_types %||% c("count", "freq_parent")
        mfi_ch     <- input$mfi_channels
        annot      <- shared$annotation

        incProgress(0.2, detail = "Gathering populations...")

        # Use GatingSet if available, otherwise just compute on root
        if (!is.null(gs)) {
          incProgress(0.2, detail = "Using GatingSet...")
          all_stats <- tryCatch({
            safe_cyto(
              CytoExploreR::cyto_stats_compute(
                gs,
                stat  = intersect(stat_types,
                        c("count", "freq_parent", "freq_total", "mfi", "gmfi", "cv")),
                channels = if (length(mfi_ch) > 0) mfi_ch else NULL
              ),
              "cyto_stats_compute failed"
            )
          }, error = function(e) NULL)

          if (!is.null(all_stats)) {
            incProgress(0.5)
            as.data.frame(all_stats)
          } else {
            compute_manual_stats(fs, annot, stat_types, mfi_ch)
          }
        } else {
          incProgress(0.2, detail = "Computing from flowSet...")
          compute_manual_stats(fs, annot, stat_types, mfi_ch)
        }
      }, error = function(e) {
        showNotification(paste("Statistics error:", conditionMessage(e)),
                         type = "error", duration = 6)
        NULL
      })

      incProgress(0.1, detail = "Done!")

      if (!is.null(result) && nrow(result) > 0) {
        local$stats_df  <- result
        local$stats_raw <- result
        shared$stats_result <- result
        showNotification(
          sprintf("Statistics computed: %d rows", nrow(result)),
          type = "message", duration = 3
        )
      }
    })

    shared$status <- "idle"
  })

  # Compute stats manually without GatingSet
  compute_manual_stats <- function(fs, annot, stat_types, mfi_ch) {
    rows <- lapply(sampleNames(fs), function(samp) {
      ff   <- fs[[samp]]
      expr <- as.data.frame(exprs(ff))
      n    <- nrow(expr)

      group_val <- "Unknown"
      if (!is.null(annot) && "SampleName" %in% names(annot)) {
        idx_a <- which(annot$SampleName == samp)
        if (length(idx_a) > 0) group_val <- annot$Group[idx_a[1]]
      }

      row <- data.frame(
        SampleName = samp,
        Population = "root",
        Group      = group_val,
        stringsAsFactors = FALSE
      )

      if ("count"       %in% stat_types) row$Count      <- n
      if ("freq_parent" %in% stat_types) row$FreqParent <- 100.0
      if ("freq_total"  %in% stat_types) row$FreqTotal  <- 100.0

      if (any(c("mfi", "gmfi", "cv") %in% stat_types) && length(mfi_ch) > 0) {
        valid_ch <- intersect(mfi_ch, names(expr))
        for (ch in valid_ch) {
          vals <- expr[[ch]]
          vals <- vals[is.finite(vals)]
          safe_id <- gsub("[^a-zA-Z0-9]", "_", ch)
          if ("mfi"  %in% stat_types) row[[paste0("MFI_",  safe_id)]] <- mean(vals,   na.rm = TRUE)
          if ("gmfi" %in% stat_types) row[[paste0("gMFI_", safe_id)]] <- exp(mean(log(pmax(vals, 1))))
          if ("cv"   %in% stat_types) row[[paste0("CV_",   safe_id)]] <-
            if (mean(vals) > 0) sd(vals) / mean(vals) * 100 else NA_real_
        }
      }
      row
    })
    do.call(bind_rows, rows)
  }

  # ── Filtered stats ────────────────────────────────────────────────────────
  filtered_stats <- reactive({
    df <- local$stats_df
    req(df)
    pop_filt   <- input$filter_pop   %||% "all"
    group_filt <- input$filter_group %||% "all"

    if (!is.null(pop_filt) && pop_filt != "all" && "Population" %in% names(df)) {
      df <- df[df$Population == pop_filt, ]
    }
    if (!is.null(group_filt) && group_filt != "all" && "Group" %in% names(df)) {
      df <- df[df$Group == group_filt, ]
    }
    df
  })

  # ── Statistics table ──────────────────────────────────────────────────────
  output$stats_table <- renderDT({
    df <- filtered_stats()
    req(df)
    datatable(
      df,
      rownames  = FALSE,
      selection = "none",
      filter    = "top",
      options   = list(
        pageLength = 20,
        scrollX    = TRUE,
        dom        = "lrtip",
        columnDefs = list(list(className = "dt-center", targets = "_all"))
      ),
      class = "compact stripe hover"
    ) %>%
      formatRound(
        intersect(c("FreqParent", "FreqTotal"), names(df)),
        digits = 2
      ) %>%
      formatRound(
        grep("^(MFI_|gMFI_|CV_)", names(df), value = TRUE),
        digits = 1
      )
  })

  # ── Bar chart ─────────────────────────────────────────────────────────────
  output$bar_stat_ui <- renderUI({
    df <- local$stats_df
    req(df)
    numeric_cols <- names(df)[sapply(df, is.numeric)]
    selectInput(ns("bar_stat"), "Statistic to Plot", choices = numeric_cols,
                selected = numeric_cols[1])
  })

  output$bar_pop_ui <- renderUI({
    df <- local$stats_df
    req(df, "Population" %in% names(df))
    pops <- unique(df$Population)
    selectInput(ns("bar_pop"), "Population",
                choices  = pops,
                selected = pops[1])
  })

  output$bar_chart <- renderPlotly({
    df       <- filtered_stats()
    stat_col <- input$bar_stat
    pop_sel  <- input$bar_pop
    group_by <- input$bar_group_by %||% "SampleName"

    req(df, stat_col, stat_col %in% names(df))

    plot_df <- df
    if (!is.null(pop_sel) && "Population" %in% names(plot_df)) {
      plot_df <- plot_df[plot_df$Population == pop_sel, ]
    }
    req(nrow(plot_df) > 0)

    x_col <- if (group_by %in% names(plot_df)) group_by else names(plot_df)[1]

    plot_ly(
      data   = plot_df,
      x      = ~get(x_col),
      y      = ~get(stat_col),
      type   = "bar",
      color  = if ("Group" %in% names(plot_df)) ~Group else ~get(x_col),
      colors = c("#00B4D8", "#2EC4B6", "#F39C12", "#E74C3C", "#9B59B6"),
      hovertemplate = paste0("%{x}<br>", stat_col, ": %{y:.2f}<extra></extra>")
    ) %>%
      plotly_dark_layout(
        title = paste(stat_col, "—", pop_sel %||% "All Populations"),
        xlab  = x_col,
        ylab  = stat_col
      ) %>%
      layout(barmode = "group")
  })

  # ── Download CSV ─────────────────────────────────────────────────────────
  output$dl_csv <- downloadHandler(
    filename = function() {
      paste0("StreamFLOW_statistics_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".csv")
    },
    content = function(file) {
      df <- local$stats_df
      req(df)
      write.csv(df, file, row.names = FALSE)
    }
  )

  # ── Download Excel ────────────────────────────────────────────────────────
  output$dl_excel <- downloadHandler(
    filename = function() {
      paste0("StreamFLOW_statistics_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".xlsx")
    },
    content = function(file) {
      df <- local$stats_df
      req(df)
      tryCatch({
        if (!requireNamespace("openxlsx", quietly = TRUE)) {
          stop("openxlsx package not available. Saving as CSV instead.")
        }
        wb <- openxlsx::createWorkbook()
        openxlsx::addWorksheet(wb, "Statistics")
        openxlsx::writeData(wb, "Statistics", df)

        # Style header
        header_style <- openxlsx::createStyle(
          fontColour = "#00B4D8", fgFill = "#1B2A3B",
          halign = "center", textDecoration = "bold"
        )
        openxlsx::addStyle(wb, "Statistics", header_style,
                           rows = 1, cols = seq_len(ncol(df)), gridExpand = TRUE)
        openxlsx::saveWorkbook(wb, file, overwrite = TRUE)
      }, error = function(e) {
        write.csv(df, file, row.names = FALSE)
        showNotification(paste("Excel export failed, saved as CSV:", conditionMessage(e)),
                         type = "warning", duration = 5)
      })
    }
  )

  output$compute_status_ui <- renderUI({
    df <- local$stats_df
    if (is.null(df)) return(NULL)
    tags$p(
      style = "font-size: 12px; color: #2EC4B6; margin-top: 6px;",
      icon("check"), sprintf(" %d rows computed", nrow(df))
    )
  })
}
