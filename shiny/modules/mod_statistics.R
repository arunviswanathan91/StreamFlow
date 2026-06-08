# StreamFLOW - mod_statistics.R
# Population statistics using CytoExploreR: cyto_stats_compute, cyto_export,
# cyto_save, cyto_group_by, cyto_filter, cyto_select, cyto_nodes

# ── UI ────────────────────────────────────────────────────────────────────────
statisticsUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # ── Left control column ────────────────────────────────────────────────
      column(3,
        box(
          title = tagList(icon("sliders-h"), " Compute Statistics"),
          width = NULL, solidHeader = TRUE,

          # Statistic types (maps to CytoExploreR stat= argument)
          checkboxGroupInput(
            ns("stat_types"),
            "Statistics (CytoExploreR)",
            choices = c(
              "Count (events)"       = "count",
              "Frequency (% parent)" = "freq",
              "Mean (MFI)"           = "mean",
              "Median (MedFI)"       = "median",
              "Mode (ModFI)"         = "mode",
              "Geometric Mean (GMFI)"= "geo mean",
              "CV (%)"               = "CV"
            ),
            selected = c("count", "freq", "median")
          ),

          # Population selector
          uiOutput(ns("population_ui")),

          # Channel selector for intensity stats
          uiOutput(ns("channel_ui")),

          # Output format
          selectInput(ns("stat_format"), "Table Format",
                      choices  = c("Long (tidy)" = "long", "Wide (pivot)" = "wide"),
                      selected = "long"),

          # Group-by (cyto_group_by)
          uiOutput(ns("group_by_ui")),

          # Density smoothing for mode
          conditionalPanel(
            condition = sprintf("input['%s'].indexOf('mode') >= 0", ns("stat_types")),
            sliderInput(ns("density_smooth"), "Mode density smooth",
                        min = 0.1, max = 2.0, value = 0.6, step = 0.1)
          ),

          tags$hr(),

          actionButton(
            ns("compute_btn"),
            tagList(icon("calculator"), " Compute Statistics"),
            class = "btn btn-primary btn-block"
          ),

          uiOutput(ns("compute_status_ui"))
        ),

        # ── Export box ────────────────────────────────────────────────────
        box(
          title = tagList(icon("download"), " Export"),
          width = NULL, solidHeader = TRUE, collapsible = TRUE,

          fluidRow(
            column(6,
              downloadButton(ns("dl_csv"),   "CSV",   class = "btn btn-default btn-block btn-sm")
            ),
            column(6,
              downloadButton(ns("dl_excel"), "Excel", class = "btn btn-default btn-block btn-sm")
            )
          ),

          tags$hr(),

          # cyto_export — FlowJo / Cytobank
          selectInput(ns("export_type"), "Workspace Export",
                      choices = c("FlowJo (.wsp)" = "flowjo",
                                  "Cytobank (.xml)" = "cytobank")),

          actionButton(ns("export_workspace_btn"),
                       tagList(icon("file-export"), " Export Workspace"),
                       class = "btn btn-default btn-block btn-sm"),

          tags$hr(),

          # cyto_save — save gated FCS files
          uiOutput(ns("save_pop_ui")),

          checkboxInput(ns("save_inverse"), "Inverse-transform when saving", FALSE),

          actionButton(ns("save_fcs_btn"),
                       tagList(icon("save"), " Save Gated FCS Files"),
                       class = "btn btn-default btn-block btn-sm")
        ),

        # ── Filter box ────────────────────────────────────────────────────
        box(
          title = tagList(icon("filter"), " Filter Results"),
          width = NULL, solidHeader = TRUE, collapsible = TRUE,

          uiOutput(ns("filter_pop_ui")),
          uiOutput(ns("filter_group_ui")),
          uiOutput(ns("filter_sample_ui"))
        )
      ),

      # ── Right results column ───────────────────────────────────────────────
      column(9,
        tabBox(
          width = NULL,

          # Statistics table tab
          tabPanel(
            tagList(icon("table"), " Statistics Table"),
            DTOutput(ns("stats_table"))
          ),

          # Bar chart tab
          tabPanel(
            tagList(icon("bar-chart"), " Bar Chart"),
            fluidRow(
              column(4, uiOutput(ns("bar_stat_ui"))),
              column(4, uiOutput(ns("bar_pop_ui"))),
              column(4, uiOutput(ns("bar_color_ui")))
            ),
            selectInput(ns("bar_type"), "Chart Type",
                        choices = c("Bar" = "bar", "Box" = "box", "Violin" = "violin"),
                        width = "160px"),
            withSpinner(
              plotlyOutput(ns("bar_chart"), height = "380px"),
              color = "#00B4D8"
            )
          ),

          # Heatmap tab
          tabPanel(
            tagList(icon("th"), " Heatmap"),
            withSpinner(
              plotlyOutput(ns("heatmap_plot"), height = "480px"),
              color = "#00B4D8"
            )
          ),

          # Summary tab
          tabPanel(
            tagList(icon("info-circle"), " Summary"),
            verbatimTextOutput(ns("summary_text"))
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
    stats_list  = list()   # named list: stat_type -> data.frame
  )

  # ── Convenience reactives ──────────────────────────────────────────────────
  best_object <- reactive({
    shared$gating_set %||% shared$trans_flowset %||%
      shared$comp_flowset %||% shared$raw_flowset
  })

  gating_set_available <- reactive({
    !is.null(shared$gating_set) &&
      inherits(shared$gating_set, c("GatingSet", "GatingHierarchy"))
  })

  all_nodes <- reactive({
    gs <- shared$gating_set
    if (is.null(gs)) return("root")
    tryCatch(cyto_nodes(gs), error = function(e) "root")
  })

  all_channels <- reactive({
    ch <- shared$fluor_channels %||% shared$channels
    ch %||% character(0)
  })

  experiment_vars <- reactive({
    annot <- shared$annotation
    if (is.null(annot) || ncol(annot) == 0) return(character(0))
    setdiff(names(annot), c("SampleName", "name"))
  })

  # ── Dynamic UIs ───────────────────────────────────────────────────────────
  output$population_ui <- renderUI({
    nodes <- all_nodes()
    selectInput(ns("populations"), "Populations",
                choices  = c("All" = "all", nodes),
                selected = "all",
                multiple = TRUE)
  })

  output$channel_ui <- renderUI({
    req(any(c("mean", "median", "mode", "geo mean", "CV") %in%
              (input$stat_types %||% character(0))))
    ch <- all_channels()
    if (length(ch) == 0) return(helpText("No channels available"))
    checkboxGroupInput(ns("stat_channels"), "Channels for Intensity Stats",
                       choices  = ch,
                       selected = ch,
                       inline   = FALSE)
  })

  output$group_by_ui <- renderUI({
    vars <- experiment_vars()
    if (length(vars) == 0) return(NULL)
    selectInput(ns("group_by_var"), "Group By (cyto_group_by)",
                choices  = c("None" = "none", setNames(vars, vars)),
                selected = "none")
  })

  output$save_pop_ui <- renderUI({
    nodes <- all_nodes()
    selectInput(ns("save_pop"), "Population to Save",
                choices  = nodes,
                selected = if ("root" %in% nodes) "root" else nodes[1])
  })

  output$filter_pop_ui <- renderUI({
    df <- local$stats_df
    if (is.null(df) || !"Population" %in% names(df)) return(NULL)
    pops <- unique(df$Population)
    selectInput(ns("filter_pop"), "Filter Population",
                choices  = c("All" = "all", pops),
                selected = "all", multiple = TRUE)
  })

  output$filter_group_ui <- renderUI({
    df <- local$stats_df
    grp_col <- intersect(c("Group", "group"), names(df %||% data.frame()))
    if (length(grp_col) == 0) return(NULL)
    groups <- unique(df[[grp_col[1]]])
    selectInput(ns("filter_group"), "Filter Group",
                choices  = c("All" = "all", groups),
                selected = "all", multiple = TRUE)
  })

  output$filter_sample_ui <- renderUI({
    df <- local$stats_df
    samp_col <- intersect(c("SampleName", "name", "Sample"), names(df %||% data.frame()))
    if (length(samp_col) == 0) return(NULL)
    samps <- unique(df[[samp_col[1]]])
    selectInput(ns("filter_sample"), "Filter Sample",
                choices  = c("All" = "all", samps),
                selected = "all", multiple = TRUE)
  })

  # ── Core compute function ──────────────────────────────────────────────────
  # Calls cyto_stats_compute once per selected stat type, merges results
  run_cyto_stats <- function(x, stat_types, channels, pops, trans, format,
                             density_smooth) {
    result_list <- list()

    # Determine populations to query
    all_pops <- if (gating_set_available()) all_nodes() else NULL
    query_pops <- if (!is.null(pops) && !("all" %in% pops)) pops else all_pops

    # For count & freq: no channel argument needed
    count_freq_stats <- intersect(stat_types, c("count", "freq"))
    intensity_stats  <- setdiff(stat_types, c("count", "freq"))

    if (length(count_freq_stats) > 0) {
      for (st in count_freq_stats) {
        df <- safe_cyto(
          {
            if (gating_set_available()) {
              cyto_stats_compute(
                x,
                alias  = if (length(query_pops) > 0) query_pops else NULL,
                stat   = st,
                format = format,
                density_smooth = density_smooth
              )
            } else {
              cyto_stats_compute(
                x,
                stat   = st,
                format = format,
                density_smooth = density_smooth
              )
            }
          },
          paste("cyto_stats_compute", st)
        )
        if (!is.null(df) && is.data.frame(df) && nrow(df) > 0) {
          result_list[[st]] <- as.data.frame(df)
        }
      }
    }

    # For intensity statistics: pass channels
    if (length(intensity_stats) > 0 && length(channels) > 0) {
      for (st in intensity_stats) {
        df <- safe_cyto(
          {
            if (gating_set_available()) {
              cyto_stats_compute(
                x,
                alias    = if (length(query_pops) > 0) query_pops else NULL,
                channels = channels,
                trans    = trans,
                stat     = st,
                format   = format,
                density_smooth = density_smooth
              )
            } else {
              cyto_stats_compute(
                x,
                channels = channels,
                trans    = trans,
                stat     = st,
                format   = format,
                density_smooth = density_smooth
              )
            }
          },
          paste("cyto_stats_compute", st)
        )
        if (!is.null(df) && is.data.frame(df) && nrow(df) > 0) {
          result_list[[st]] <- as.data.frame(df)
        }
      }
    }

    result_list
  }

  # Merge multiple stat data.frames into one wide table
  merge_stat_results <- function(stat_list, format) {
    if (length(stat_list) == 0) return(NULL)

    # In "long" format, bind rows after adding a "Statistic" column
    if (format == "long") {
      dfs <- lapply(names(stat_list), function(st) {
        df <- stat_list[[st]]
        df$Statistic <- st
        df
      })
      out <- tryCatch(dplyr::bind_rows(dfs), error = function(e) do.call(rbind, dfs))
      return(out)
    }

    # In "wide" format, join by key columns
    id_cols <- c("SampleName", "name", "Population", "alias", "parent",
                 "Group", "Treatment", "Replicate", "Statistic")
    dfs <- stat_list

    if (length(dfs) == 1) return(dfs[[1]])

    merged <- dfs[[1]]
    for (i in seq_along(dfs)[-1]) {
      common_keys <- intersect(names(merged), id_cols)
      common_keys <- intersect(common_keys, names(dfs[[i]]))
      if (length(common_keys) > 0) {
        merged <- tryCatch(
          dplyr::full_join(merged, dfs[[i]], by = common_keys),
          error = function(e) cbind(merged, dfs[[i]])
        )
      }
    }
    merged
  }

  # Fallback: manual stats when CytoExploreR API unavailable
  compute_fallback_stats <- function(fs, stat_types, channels, annot) {
    rows <- lapply(flowCore::sampleNames(fs), function(samp) {
      ff   <- fs[[samp]]
      expr <- as.data.frame(flowCore::exprs(ff))
      n    <- nrow(expr)

      grp <- tryCatch({
        if (!is.null(annot) && "Group" %in% names(annot)) {
          idx <- which(annot$SampleName == samp)
          if (length(idx) > 0) annot$Group[idx[1]] else "Unknown"
        } else "Unknown"
      }, error = function(e) "Unknown")

      row <- data.frame(SampleName = samp, Population = "root",
                        Group = grp, stringsAsFactors = FALSE)

      if ("count"   %in% stat_types) row$Count     <- n
      if ("freq"    %in% stat_types) row$Frequency  <- 100.0

      valid_ch <- intersect(channels %||% character(0), names(expr))
      if (length(valid_ch) > 0) {
        for (ch in valid_ch) {
          v  <- expr[[ch]]
          v  <- v[is.finite(v)]
          id <- make.names(ch)
          if ("mean"     %in% stat_types) row[[paste0("MFI_",    id)]] <- mean(v)
          if ("median"   %in% stat_types) row[[paste0("MedFI_",  id)]] <- stats::median(v)
          if ("mode"     %in% stat_types) {
            dens <- tryCatch(density(v, n = 512), error = function(e) NULL)
            row[[paste0("ModFI_", id)]] <- if (!is.null(dens)) dens$x[which.max(dens$y)] else NA
          }
          if ("geo mean" %in% stat_types) row[[paste0("GMFI_",   id)]] <- exp(mean(log(pmax(v, 1e-3))))
          if ("CV"       %in% stat_types) {
            mu <- mean(v)
            row[[paste0("CV_", id)]] <- if (mu > 0) stats::sd(v) / mu * 100 else NA
          }
        }
      }
      row
    })
    do.call(dplyr::bind_rows, rows)
  }

  # ── Compute button ────────────────────────────────────────────────────────
  observeEvent(input$compute_btn, {
    obj <- best_object()
    req(obj)

    stat_types     <- input$stat_types    %||% c("count", "median")
    channels       <- input$stat_channels %||% all_channels()
    pops           <- input$populations   %||% "all"
    format         <- input$stat_format   %||% "long"
    density_smooth <- input$density_smooth %||% 0.6
    group_var      <- input$group_by_var  %||% "none"

    # Get transformers from GatingSet for proper back-transform
    trans <- tryCatch(
      if (gating_set_available()) cyto_transformer_extract(shared$gating_set) else NA,
      error = function(e) NA
    )

    shared$status <- "busy"

    withProgress(message = "Computing statistics...", value = 0, {
      incProgress(0.1, detail = "Initialising...")

      result_df <- tryCatch({
        # If group_by is set, use cyto_group_by to split and compute per group
        if (group_var != "none" && gating_set_available()) {
          incProgress(0.1, detail = "Grouping samples...")
          gs_groups <- safe_cyto(
            cyto_group_by(shared$gating_set, group_by = group_var),
            "cyto_group_by"
          )

          if (!is.null(gs_groups) && is.list(gs_groups)) {
            grp_dfs <- lapply(names(gs_groups), function(grp_name) {
              gs_sub <- gs_groups[[grp_name]]
              sl <- run_cyto_stats(gs_sub, stat_types, channels, pops,
                                   trans, format, density_smooth)
              df <- merge_stat_results(sl, format)
              if (!is.null(df)) df[[group_var]] <- grp_name
              df
            })
            grp_dfs <- Filter(Negate(is.null), grp_dfs)
            if (length(grp_dfs) > 0) dplyr::bind_rows(grp_dfs) else NULL
          } else {
            # fallback: ungrouped
            sl <- run_cyto_stats(obj, stat_types, channels, pops,
                                 trans, format, density_smooth)
            merge_stat_results(sl, format)
          }
        } else {
          incProgress(0.2, detail = "Computing per-population stats...")
          sl <- run_cyto_stats(obj, stat_types, channels, pops,
                               trans, format, density_smooth)
          local$stats_list <- sl
          merge_stat_results(sl, format)
        }
      }, error = function(e) {
        showNotification(
          paste("cyto_stats_compute error:", conditionMessage(e)),
          type = "error", duration = 8
        )
        NULL
      })

      # Fallback to manual stats if CytoExploreR returned nothing
      if (is.null(result_df) || (is.data.frame(result_df) && nrow(result_df) == 0)) {
        incProgress(0.2, detail = "Using fallback stats...")
        fs <- shared$trans_flowset %||% shared$comp_flowset %||% shared$raw_flowset
        if (!is.null(fs) && inherits(fs, c("flowSet", "ncdfFlowSet", "cytoset"))) {
          result_df <- tryCatch(
            compute_fallback_stats(fs, stat_types, channels, shared$annotation),
            error = function(e) NULL
          )
        }
      }

      incProgress(0.9, detail = "Finalising...")

      if (!is.null(result_df) && is.data.frame(result_df) && nrow(result_df) > 0) {
        local$stats_df      <- result_df
        shared$stats_result <- result_df
        showNotification(
          sprintf("Computed %d rows x %d columns", nrow(result_df), ncol(result_df)),
          type = "message", duration = 3
        )
      } else {
        showNotification("No statistics produced — check populations/channels.",
                         type = "warning", duration = 5)
      }
    })

    shared$status <- "idle"
  })

  # ── Filtered table ────────────────────────────────────────────────────────
  filtered_stats <- reactive({
    df <- local$stats_df
    req(df)

    # Filter population
    pop_f  <- input$filter_pop    %||% "all"
    grp_f  <- input$filter_group  %||% "all"
    samp_f <- input$filter_sample %||% "all"

    pop_col  <- intersect(c("Population", "alias"), names(df))[1]
    grp_col  <- intersect(c("Group", "group"), names(df))[1]
    samp_col <- intersect(c("SampleName", "name", "Sample"), names(df))[1]

    if (!is.na(pop_col) && !("all" %in% pop_f) && length(pop_f) > 0)
      df <- df[df[[pop_col]] %in% pop_f, ]
    if (!is.na(grp_col) && !("all" %in% grp_f) && length(grp_f) > 0)
      df <- df[df[[grp_col]] %in% grp_f, ]
    if (!is.na(samp_col) && !("all" %in% samp_f) && length(samp_f) > 0)
      df <- df[df[[samp_col]] %in% samp_f, ]

    df
  })

  # ── Stats table ───────────────────────────────────────────────────────────
  output$stats_table <- renderDT({
    df <- filtered_stats()
    req(df)

    numeric_cols <- names(df)[sapply(df, is.numeric)]
    pct_cols     <- grep("(?i)(freq|pct|percent|\\%)", names(df), value = TRUE)

    datatable(
      df,
      rownames  = FALSE,
      selection = "none",
      filter    = "top",
      options   = list(
        pageLength = 25,
        scrollX    = TRUE,
        dom        = "lrtip",
        columnDefs = list(list(className = "dt-center", targets = "_all"))
      ),
      class = "compact stripe hover"
    ) %>%
      formatRound(columns = intersect(pct_cols, numeric_cols), digits = 2) %>%
      formatRound(columns = setdiff(numeric_cols, pct_cols),   digits = 3)
  })

  # ── Bar/Box/Violin chart ──────────────────────────────────────────────────
  output$bar_stat_ui <- renderUI({
    df <- local$stats_df
    req(df)
    num_cols <- names(df)[sapply(df, is.numeric)]
    if (length(num_cols) == 0) return(NULL)
    selectInput(ns("bar_stat"), "Statistic",
                choices = num_cols, selected = num_cols[1])
  })

  output$bar_pop_ui <- renderUI({
    df <- local$stats_df
    req(df)
    pop_col <- intersect(c("Population", "alias"), names(df))[1]
    if (is.na(pop_col)) return(NULL)
    pops <- unique(df[[pop_col]])
    selectInput(ns("bar_pop"), "Population",
                choices = pops, selected = pops[1])
  })

  output$bar_color_ui <- renderUI({
    df <- local$stats_df
    req(df)
    cat_cols <- names(df)[sapply(df, function(x) is.character(x) || is.factor(x))]
    cat_cols <- setdiff(cat_cols, c("name", "SampleName"))
    if (length(cat_cols) == 0) return(NULL)
    selectInput(ns("bar_color"), "Color By",
                choices  = cat_cols,
                selected = if ("Group" %in% cat_cols) "Group" else cat_cols[1])
  })

  output$bar_chart <- renderPlotly({
    df       <- filtered_stats()
    stat_col <- input$bar_stat
    pop_sel  <- input$bar_pop
    color_by <- input$bar_color %||% NULL
    chart_type <- input$bar_type %||% "bar"

    req(df, stat_col, stat_col %in% names(df))

    # Subset to selected population
    pop_col <- intersect(c("Population", "alias"), names(df))[1]
    if (!is.na(pop_col) && !is.null(pop_sel)) {
      df <- df[df[[pop_col]] == pop_sel, ]
    }
    req(nrow(df) > 0)

    x_col <- intersect(c("SampleName", "name", "Sample"), names(df))[1]
    if (is.na(x_col)) x_col <- names(df)[1]

    color_sym <- if (!is.null(color_by) && color_by %in% names(df)) {
      as.formula(paste0("~", color_by))
    } else NULL

    p <- switch(chart_type,
      "box" = plot_ly(df, y = ~get(stat_col), color = color_sym,
                      type = "box",
                      colors = c("#00B4D8", "#2EC4B6", "#F39C12",
                                 "#E74C3C", "#9B59B6", "#27AE60")),
      "violin" = plot_ly(df, y = ~get(stat_col), color = color_sym,
                         type = "violin",
                         colors = c("#00B4D8", "#2EC4B6", "#F39C12",
                                    "#E74C3C", "#9B59B6", "#27AE60")),
      # default: bar
      plot_ly(df,
              x      = ~get(x_col),
              y      = ~get(stat_col),
              type   = "bar",
              color  = color_sym,
              colors = c("#00B4D8", "#2EC4B6", "#F39C12",
                         "#E74C3C", "#9B59B6", "#27AE60"),
              hovertemplate = paste0("%{x}<br>", stat_col,
                                    ": %{y:.3f}<extra></extra>"))
    )

    p %>%
      plotly_dark_layout(
        title = paste(stat_col, if (!is.null(pop_sel)) paste("—", pop_sel)),
        xlab  = x_col,
        ylab  = stat_col
      ) %>%
      layout(barmode = "group")
  })

  # ── Heatmap ───────────────────────────────────────────────────────────────
  output$heatmap_plot <- renderPlotly({
    df <- filtered_stats()
    req(df)

    num_cols  <- names(df)[sapply(df, is.numeric)]
    req(length(num_cols) >= 2)

    samp_col <- intersect(c("SampleName", "name", "Sample"), names(df))[1]
    pop_col  <- intersect(c("Population", "alias"), names(df))[1]

    row_label <- if (!is.na(samp_col)) {
      if (!is.na(pop_col))
        paste0(df[[samp_col]], " / ", df[[pop_col]])
      else
        df[[samp_col]]
    } else seq_len(nrow(df))

    mat <- as.matrix(df[, num_cols, drop = FALSE])
    # Normalise each column 0-1 for display
    mat_norm <- apply(mat, 2, function(col) {
      r <- range(col, na.rm = TRUE)
      if (diff(r) == 0) return(rep(0.5, length(col)))
      (col - r[1]) / diff(r)
    })

    plot_ly(
      z          = mat_norm,
      x          = num_cols,
      y          = row_label,
      type       = "heatmap",
      colorscale = list(c(0, "#0D1B2A"), c(0.5, "#00B4D8"), c(1, "#2EC4B6")),
      hovertemplate = "Sample: %{y}<br>Channel: %{x}<br>Value: %{z:.3f}<extra></extra>"
    ) %>%
      plotly_dark_layout(
        title = "Statistics Heatmap (normalised per column)",
        xlab  = "Channel / Statistic",
        ylab  = "Sample / Population"
      ) %>%
      layout(margin = list(l = 160, b = 120))
  })

  # ── Summary text ──────────────────────────────────────────────────────────
  output$summary_text <- renderPrint({
    df <- local$stats_df
    if (is.null(df)) {
      cat("No statistics computed yet.\n")
      return(invisible(NULL))
    }
    cat(sprintf("Rows: %d  |  Columns: %d\n\n", nrow(df), ncol(df)))
    num_cols <- names(df)[sapply(df, is.numeric)]
    if (length(num_cols) > 0) {
      cat("Numeric column summary:\n")
      print(summary(df[, num_cols, drop = FALSE]))
    }
    cat("\nColumn types:\n")
    print(sapply(df, class))
  })

  # ── Compute status UI ─────────────────────────────────────────────────────
  output$compute_status_ui <- renderUI({
    df <- local$stats_df
    if (is.null(df)) return(NULL)
    tags$p(
      style = "font-size: 12px; color: #2EC4B6; margin-top: 6px;",
      icon("check"),
      sprintf(" %d rows × %d cols", nrow(df), ncol(df))
    )
  })

  # ── Download CSV ──────────────────────────────────────────────────────────
  output$dl_csv <- downloadHandler(
    filename = function() {
      paste0("StreamFLOW_stats_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".csv")
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
      paste0("StreamFLOW_stats_", format(Sys.time(), "%Y%m%d_%H%M%S"), ".xlsx")
    },
    content = function(file) {
      df <- local$stats_df
      req(df)
      if (!requireNamespace("openxlsx", quietly = TRUE)) {
        write.csv(df, file, row.names = FALSE)
        return(invisible(NULL))
      }
      wb <- openxlsx::createWorkbook()
      openxlsx::addWorksheet(wb, "Statistics")
      openxlsx::writeData(wb, "Statistics", df)
      openxlsx::addStyle(wb, "Statistics",
        openxlsx::createStyle(
          fontColour     = "#00B4D8",
          fgFill         = "#1B2A3B",
          halign         = "center",
          textDecoration = "bold"
        ),
        rows = 1, cols = seq_len(ncol(df)), gridExpand = TRUE
      )
      openxlsx::saveWorkbook(wb, file, overwrite = TRUE)
    }
  )

  # ── Export workspace (cyto_export) ────────────────────────────────────────
  observeEvent(input$export_workspace_btn, {
    gs <- shared$gating_set
    if (is.null(gs)) {
      showNotification("No GatingSet available for workspace export.",
                       type = "warning", duration = 4)
      return()
    }

    exp_type <- input$export_type %||% "flowjo"
    ext      <- if (exp_type == "flowjo") ".wsp" else ".xml"
    fname    <- paste0("StreamFLOW_workspace_",
                       format(Sys.time(), "%Y%m%d_%H%M%S"), ext)
    out_path <- file.path(tempdir(), fname)

    result <- safe_cyto(
      cyto_export(gs, save_as = out_path),
      "cyto_export"
    )

    if (!is.null(result) || file.exists(out_path)) {
      showNotification(
        paste("Workspace exported to:", basename(out_path)),
        type = "message", duration = 5
      )
    } else {
      showNotification("Workspace export failed. Is the GatingSet fully gated?",
                       type = "error", duration = 6)
    }
  })

  # ── Save gated FCS files (cyto_save) ──────────────────────────────────────
  observeEvent(input$save_fcs_btn, {
    gs <- shared$gating_set
    if (is.null(gs)) {
      showNotification("No GatingSet available for FCS export.",
                       type = "warning", duration = 4)
      return()
    }

    pop  <- input$save_pop      %||% "root"
    inv  <- input$save_inverse  %||% FALSE
    trans <- tryCatch(
      cyto_transformer_extract(gs),
      error = function(e) NULL
    )
    save_dir <- file.path(tempdir(),
                          paste0("GatedFCS_", format(Sys.time(), "%Y%m%d_%H%M%S")))

    result <- safe_cyto(
      cyto_save(
        gs,
        parent  = pop,
        save_as = save_dir,
        inverse = inv,
        trans   = if (inv && !is.null(trans)) trans else NULL
      ),
      "cyto_save"
    )

    if (file.exists(save_dir)) {
      fcs_files <- list.files(save_dir, pattern = "\\.fcs$", full.names = FALSE)
      showNotification(
        sprintf("Saved %d FCS file(s) to: %s", length(fcs_files), save_dir),
        type = "message", duration = 6
      )
    } else {
      showNotification("FCS save failed or no files produced.",
                       type = "error", duration = 5)
    }
  })
}
