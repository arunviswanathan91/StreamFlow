# StreamFLOW - mod_setup.R
# FCS file loading, annotation, and channel management
# CytoExploreR: cyto_load, cyto_setup, cyto_details, cyto_names, cyto_names_parse,
#               cyto_channels, cyto_fluor_channels, cyto_markers, cyto_markers_edit,
#               cyto_clean, cyto_barcode, cyto_check

# ── UI ────────────────────────────────────────────────────────────────────────
setupUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left panel
      column(4,
        box(
          title = "Data Import", width = NULL, solidHeader = TRUE,
          tags$div(
            style = "display:flex;gap:6px;margin-bottom:10px;",
            actionButton(
              ns("fcs_folder_btn"),
              label = tagList(icon("folder-open"), " Browse Folder"),
              class = "btn btn-default",
              style = "flex:1;",
              onclick = sprintf(
                "streamflowPickFolder('%s', 'Select folder containing FCS files')",
                ns("fcs_folder_picked")
              )
            ),
            actionButton(
              ns("fcs_files_btn"),
              label = tagList(icon("file-medical"), " Select Files"),
              class = "btn btn-default",
              style = "flex:1;",
              onclick = sprintf(
                "streamflowPickFiles('%s', 'Select FCS files', [{name:'FCS Files',extensions:['fcs','FCS']}])",
                ns("fcs_files_picked")
              )
            )
          ),
          uiOutput(ns("selected_folder_label")),
          tags$hr(),
          checkboxInput(ns("recursive"), "Include sub-folders",    value = FALSE),
          checkboxInput(ns("do_clean"),  "Run flowAI quality check (cyto_clean)", value = FALSE),
          checkboxInput(ns("do_barcode"),"Barcode samples (cyto_barcode)",        value = FALSE),
          # cyto_names_parse() is interactive (prompts for delimiter) and hangs
          # in headless Electron — names are auto-parsed safely on load instead.
          actionButton(
            ns("load_fcs_btn"),
            label = tagList(icon("upload"), " Load FCS Files"),
            class = "btn btn-primary btn-block"
          ),
          tags$hr(),
          uiOutput(ns("flowset_summary"))
        ),

        box(
          title = "Channels & Markers", width = NULL, solidHeader = TRUE,
          tabsetPanel(
            tabPanel("Channels",
              tags$p(style = "font-size:11px;color:#5A7A8A;margin:6px 0;",
                     "Fluorescent channels detected by cyto_fluor_channels():"),
              uiOutput(ns("fluor_channels_ui")),
              tags$hr(),
              tags$p(style = "font-size:11px;color:#5A7A8A;margin:6px 0;",
                     "All channels (include/exclude per-channel):"),
              uiOutput(ns("channel_selector"))
            ),
            tabPanel("Markers",
              tags$p(style = "font-size:11px;color:#5A7A8A;margin:6px 0;",
                     "Assign marker names to channels (cyto_markers_edit):"),
              uiOutput(ns("marker_editor_ui")),
              actionButton(ns("save_markers_btn"),
                           tagList(icon("save"), " Save Marker Names"),
                           class = "btn btn-success btn-sm")
            )
          )
        )
      ),

      # Right panel
      column(8,
        box(
          title = "Sample Annotation (cyto_details)", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(12,
              tags$div(
                style = "display:flex;gap:8px;flex-wrap:wrap;margin-bottom:10px;",
                actionButton(ns("add_row"),    tagList(icon("plus"),   " Add Row"),    class = "btn btn-default btn-sm"),
                actionButton(ns("remove_row"), tagList(icon("minus"),  " Remove Row"), class = "btn btn-danger btn-sm"),
                actionButton(ns("save_annot"), tagList(icon("save"),   " Save CSV"),   class = "btn btn-success btn-sm"),
                actionButton(
                  ns("load_annot_btn"),
                  label   = tagList(icon("file-csv"), " Load CSV"),
                  class   = "btn btn-default btn-sm",
                  onclick = sprintf(
                    "streamflowPickFile('%s', 'Select annotation CSV', [{name: 'CSV files', extensions: ['csv']}])",
                    ns("load_annot_picked")
                  )
                ),
                actionButton(ns("details_btn"),
                             tagList(icon("edit"), " Edit Details"),
                             class = "btn btn-default btn-sm",
                             title = "Opens cyto_details_edit() editor")
              ),
              DTOutput(ns("annotation_table"))
            )
          )
        ),

        box(
          title = "FCS Preview", width = NULL, solidHeader = TRUE,
          uiOutput(ns("fcs_preview"))
        )
      )
    )
  )
}

# ── Server ────────────────────────────────────────────────────────────────────
setupServer <- function(input, output, session, shared) {
  ns <- session$ns

  local <- reactiveValues(
    folder_path  = NULL,
    picked_files = NULL,   # set when user picks files directly (overrides folder scan)
    flowset      = NULL,
    annotation   = data.frame(
      SampleName = character(), Group = character(),
      Treatment  = character(), Replicate = character(),
      TimePoint  = character(), stringsAsFactors = FALSE
    ),
    marker_map   = data.frame(Channel = character(), Marker = character(),
                              stringsAsFactors = FALSE)
  )

  # ── Folder selection (native Electron dialog) ─────────────────────────────
  observeEvent(input$fcs_folder_picked, {
    sel <- input$fcs_folder_picked
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
    local$folder_path <- p
    shared$fcs_folder <- p
    showNotification(
      paste0("Folder selected: ", basename(p)),
      type = "message", duration = 3
    )
  })

  # ── Direct FCS file selection ─────────────────────────────────────────────
  observeEvent(input$fcs_files_picked, {
    sel <- input$fcs_files_picked
    if (is.list(sel) && identical(sel$error, "no_electron")) {
      showNotification("File selection requires the StreamFLOW desktop app.",
                       type = "warning", duration = 5)
      return()
    }
    paths <- sel$paths
    if (is.null(paths) || length(paths) == 0) return()
    valid <- paths[file.exists(paths)]
    if (length(valid) == 0) {
      showNotification("None of the selected files could be found.",
                       type = "error", duration = 5)
      return()
    }
    parent_dir <- unique(dirname(valid))
    local$folder_path <- if (length(parent_dir) == 1) parent_dir else dirname(valid[1])
    local$picked_files <- valid
    shared$fcs_folder  <- local$folder_path
    showNotification(
      sprintf("%d FCS file(s) selected directly.", length(valid)),
      type = "message", duration = 3
    )
  })

  output$selected_folder_label <- renderUI({
    if (!is.null(local$folder_path)) {
      tags$p(style = "font-size:11px;color:#2EC4B6;word-break:break-all;margin:6px 0;",
             icon("check-circle"), " ", local$folder_path)
    } else {
      tags$p(style = "font-size:11px;color:#5A7A8A;margin:6px 0;", "No folder selected")
    }
  })

  # ── Load FCS files ────────────────────────────────────────────────────────
  observeEvent(input$load_fcs_btn, {
    req(local$folder_path)
    folder <- local$folder_path
    shared$status <- "busy"
    shinyjs::disable("load_fcs_btn")

    fs <- withProgress(message = "Loading FCS files…", value = 0, {

      incProgress(0.05, detail = "Locating FCS files…")
      fcs_files <- if (!is.null(local$picked_files)) {
        local$picked_files
      } else {
        list.files(folder, pattern = "\\.fcs$",
                   recursive  = isTRUE(input$recursive),
                   full.names = TRUE, ignore.case = TRUE)
      }

      if (length(fcs_files) == 0) {
        showNotification(
          paste0("No .fcs files found in: ", basename(folder),
                 if (isTRUE(input$recursive)) "" else " — try enabling 'Include sub-folders'"),
          type = "warning", duration = 7
        )
        return(NULL)
      }

      showNotification(
        sprintf("Found %d FCS file(s). Reading (this may take a minute for large files)…", length(fcs_files)),
        type = "message", duration = 5
      )

      # ── Validate each file header before bulk read ─────────────────────────
      # cyto_load() loads saved workspaces, NOT raw FCS — do not use it here.
      incProgress(0.10, detail = sprintf("Validating %d FCS headers…", length(fcs_files)))
      valid_files <- Filter(function(f) {
        tryCatch({ flowCore::read.FCSheader(f); TRUE }, error = function(e) {
          message(sprintf("[StreamFLOW] Skipping unreadable FCS: %s — %s",
                          basename(f), conditionMessage(e)))
          FALSE
        })
      }, fcs_files)

      if (length(valid_files) == 0) {
        showNotification(
          "No valid FCS files found. Ensure the folder contains uncorrupted .fcs files.",
          type = "error", duration = 10
        )
        return(NULL)
      }

      if (length(valid_files) < length(fcs_files)) {
        showNotification(
          sprintf("%d of %d file(s) were skipped (unreadable headers).",
                  length(fcs_files) - length(valid_files), length(fcs_files)),
          type = "warning", duration = 6
        )
      }

      # ── flowCore::read.flowSet — primary FCS loader ────────────────────────
      incProgress(0.20, detail = sprintf("Reading %d file(s) with flowCore…", length(valid_files)))
      loaded_fs <- tryCatch(
        flowCore::read.flowSet(files = valid_files, transformation = FALSE),
        error = function(e) {
          showNotification(
            paste0("flowCore::read.flowSet() failed: ", conditionMessage(e)),
            type = "error", duration = 10
          )
          NULL
        }
      )

      if (is.null(loaded_fs)) {
        return(NULL)
      }

      incProgress(0.10)
      loaded_fs
    })

    if (is.null(fs)) {
      shared$status <- "idle"
      shinyjs::enable("load_fcs_btn")
      return()
    }

    withProgress(message = "Processing loaded files…", value = 0.5, {

      # ── cyto_barcode: assign sample-ID barcodes ────────────────────────────
      if (isTRUE(input$do_barcode)) {
        incProgress(0, detail = "cyto_barcode(): tagging samples…")
        fs <- tryCatch(
          safe_cyto(CytoExploreR::cyto_barcode(fs), "cyto_barcode failed") %||% fs,
          error = function(e) fs
        )
      }

      # ── cyto_clean: remove anomalous events ──────────────────────────────
      if (isTRUE(input$do_clean)) {
        incProgress(0, detail = "cyto_clean(): removing anomalous events…")
        showNotification("Running flowAI quality check (cyto_clean)…",
                         type = "message", duration = 4)
        fs <- tryCatch(
          safe_cyto(CytoExploreR::cyto_clean(fs), "cyto_clean failed") %||% fs,
          error = function(e) {
            showNotification(paste("cyto_clean skipped:", conditionMessage(e)),
                             type = "warning", duration = 4)
            fs
          }
        )
      }

      incProgress(0.2, detail = "Building annotation table…")

      # ── cyto_names: extract sample names ──────────────────────────────────
      sample_names <- tryCatch(
        CytoExploreR::cyto_names(fs),
        error = function(e) sampleNames(fs)
      )

      # ── Parse names into details (non-interactive, regex-based) ─────────────
      # cyto_names_parse() prompts interactively for a delimiter and will hang
      # permanently in headless Electron — never call it here.
      annot_df <- tryCatch({
        parts <- strsplit(tools::file_path_sans_ext(basename(sample_names)),
                          "[_\\-\\s]+")
        max_cols <- max(lengths(parts))
        if (max_cols >= 2) {
          mat <- t(sapply(parts, function(x) {
            length(x) <- max_cols; x[is.na(x)] <- ""; x
          }))
          as.data.frame(mat, stringsAsFactors = FALSE)
        } else NULL
      }, error = function(e) NULL)

      if (is.null(annot_df) || nrow(annot_df) == 0) {
        annot_df <- data.frame(
          SampleName = sample_names,
          Group      = rep("Group1", length(sample_names)),
          Treatment  = rep("Control", length(sample_names)),
          Replicate  = as.character(seq_along(sample_names)),
          TimePoint  = rep("T0", length(sample_names)),
          stringsAsFactors = FALSE
        )
      } else {
        if (!"SampleName" %in% names(annot_df))
          annot_df$SampleName <- sample_names
      }

      local$picked_files <- NULL   # consumed; next load uses folder scan
      local$flowset      <- fs
      local$annotation   <- annot_df
      shared$raw_flowset    <- fs
      shared$annotation     <- annot_df
      shared$n_samples      <- length(fs)
      shared$experiment_name <- basename(folder)

      # ── cyto_channels / cyto_fluor_channels ──────────────────────────────
      all_ch   <- tryCatch(CytoExploreR::cyto_channels(fs),   error = function(e) colnames(fs))
      fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(fs), error = function(e) {
        grep("^(FSC|SSC)", all_ch, invert = TRUE, value = TRUE, ignore.case = TRUE)
      })
      shared$channels      <- all_ch
      shared$fluor_channels <- fluor_ch

      # ── cyto_markers: build initial marker map ────────────────────────────
      markers <- tryCatch(CytoExploreR::cyto_markers(fs), error = function(e) NULL)
      local$marker_map <- data.frame(
        Channel = all_ch,
        Marker  = if (!is.null(markers) && length(markers) == length(all_ch))
                    markers else all_ch,
        stringsAsFactors = FALSE
      )
      shared$markers      <- if (!is.null(markers)) markers else all_ch
      shared$all_channels <- all_ch

      incProgress(0.1, detail = "Done!")
      showNotification(
        sprintf("Successfully loaded %d FCS file(s). %d fluorescent channel(s) detected.",
                length(fs), length(fluor_ch)),
        type = "message", duration = 5
      )
    })

    shared$status <- "idle"
    shinyjs::enable("load_fcs_btn")
  })

  # ── cyto_details_edit (button) ────────────────────────────────────────────
  observeEvent(input$details_btn, {
    req(local$flowset)
    showNotification(
      "cyto_details_edit() requires an interactive R session. Use the annotation table below to edit details in StreamFLOW.",
      type = "warning", duration = 5
    )
  })

  # ── Annotation table ─────────────────────────────────────────────────────
  output$annotation_table <- renderDT({
    req(nrow(local$annotation) > 0)
    datatable(local$annotation,
              editable  = list(target = "cell", disable = list(columns = 0)),
              selection = "single",
              rownames  = FALSE,
              options   = list(pageLength = 15, dom = "tip", scrollX = TRUE,
                               columnDefs = list(list(className = "dt-center", targets = "_all"))),
              class = "compact stripe hover")
  }, server = TRUE)

  observeEvent(input$annotation_table_cell_edit, {
    info <- input$annotation_table_cell_edit
    df   <- local$annotation
    df[info$row, info$col + 1] <- info$value
    local$annotation  <- df
    shared$annotation <- df
  })

  observeEvent(input$add_row, {
    local$annotation <- rbind(local$annotation,
      data.frame(SampleName = "NewSample", Group = "Group1",
                 Treatment = "Control", Replicate = "1", TimePoint = "T0",
                 stringsAsFactors = FALSE))
    shared$annotation <- local$annotation
  })

  observeEvent(input$remove_row, {
    sel <- input$annotation_table_rows_selected
    if (!is.null(sel) && length(sel) > 0) {
      local$annotation  <- local$annotation[-sel, , drop = FALSE]
      shared$annotation <- local$annotation
    } else {
      showNotification("Select a row to remove.", type = "warning", duration = 3)
    }
  })

  observeEvent(input$save_annot, {
    req(local$folder_path)
    out <- file.path(local$folder_path, "sample_annotation.csv")
    tryCatch({
      # Also persist via cyto_details_save if possible
      write.csv(local$annotation, out, row.names = FALSE)
      if (!is.null(local$flowset)) {
        tryCatch(
          safe_cyto(CytoExploreR::cyto_details_save(local$annotation,
                                                      save_as = out),
                    "cyto_details_save failed"),
          error = function(e) NULL
        )
      }
      showNotification(paste("Annotation saved to", out), type = "message", duration = 4)
    }, error = function(e) {
      showNotification(paste("Save failed:", conditionMessage(e)), type = "error", duration = 5)
    })
  })

  observeEvent(input$load_annot_picked, {
    sel <- input$load_annot_picked
    if (is.list(sel) && identical(sel$error, "no_electron")) {
      showNotification("File selection requires the StreamFLOW desktop app.",
                       type = "warning", duration = 5)
      return()
    }
    p <- sel$path
    if (is.null(p) || !nzchar(p)) return()
    if (!file.exists(p) || tolower(tools::file_ext(p)) != "csv") {
      showNotification("Please select an existing .csv file.", type = "error", duration = 5)
      return()
    }
    tryCatch({
      df <- read.csv(p, stringsAsFactors = FALSE)
      local$annotation  <- df
      shared$annotation <- df
      showNotification("Annotation loaded.", type = "message", duration = 3)
    }, error = function(e) {
      showNotification(paste("Load failed:", conditionMessage(e)), type = "error", duration = 5)
    })
  })

  # ── Channel selector ──────────────────────────────────────────────────────
  output$fluor_channels_ui <- renderUI({
    channels <- shared$fluor_channels
    if (is.null(channels)) return(tags$p(style = "color:#5A7A8A;font-size:12px;",
                                          "Load FCS files first."))
    tags$div(
      style = "display:flex;flex-wrap:wrap;gap:4px;",
      lapply(channels, function(ch) {
        tags$span(style = "background:#00B4D8;color:#0D1B2A;font-size:11px;padding:2px 7px;border-radius:10px;",
                  ch)
      })
    )
  })

  output$channel_selector <- renderUI({
    channels <- shared$channels
    if (is.null(channels)) return(tags$p(style = "color:#5A7A8A;font-size:12px;",
                                          "Load FCS files first."))
    checkboxGroupInput(ns("selected_channels"), label = NULL,
                       choices = channels, selected = channels)
  })

  observeEvent(input$selected_channels, {
    shared$channels <- input$selected_channels
  })

  # ── Marker editor ─────────────────────────────────────────────────────────
  output$marker_editor_ui <- renderUI({
    mm <- local$marker_map
    if (is.null(mm) || nrow(mm) == 0)
      return(tags$p(style = "color:#5A7A8A;font-size:12px;", "Load FCS files first."))

    rows <- lapply(seq_len(nrow(mm)), function(i) {
      ch <- mm$Channel[i]
      mk <- mm$Marker[i]
      safe_id <- gsub("[^a-zA-Z0-9]", "_", ch)
      fluidRow(
        column(5, tags$span(style = "font-size:12px;color:#B0C4D8;line-height:34px;", ch)),
        column(7, textInput(ns(paste0("marker_", safe_id)), label = NULL, value = mk))
      )
    })
    tagList(rows)
  })

  observeEvent(input$save_markers_btn, {
    req(local$flowset)
    mm  <- local$marker_map
    req(nrow(mm) > 0)

    new_markers <- sapply(mm$Channel, function(ch) {
      safe_id <- gsub("[^a-zA-Z0-9]", "_", ch)
      input[[paste0("marker_", safe_id)]] %||% ch
    })

    local$marker_map$Marker <- new_markers

    # Apply via cyto_markers_edit
    fs <- tryCatch({
      marker_list <- setNames(as.list(new_markers), mm$Channel)
      safe_cyto(
        CytoExploreR::cyto_markers_edit(local$flowset, markers = marker_list),
        "cyto_markers_edit failed"
      ) %||% local$flowset
    }, error = function(e) local$flowset)

    local$flowset      <- fs
    shared$raw_flowset <- fs
    showNotification("Marker names saved via cyto_markers_edit().", type = "message", duration = 3)
  })

  # ── FCS summary ──────────────────────────────────────────────────────────
  output$flowset_summary <- renderUI({
    fs <- local$flowset
    if (is.null(fs)) return(tags$p(style = "color:#5A7A8A;font-size:12px;", "No data loaded"))

    n_files  <- length(fs)
    all_ch   <- tryCatch(CytoExploreR::cyto_channels(fs),        error = function(e) colnames(fs))
    fluor_ch <- tryCatch(CytoExploreR::cyto_fluor_channels(fs),  error = function(e) character())
    total_cells <- tryCatch(sum(sapply(seq_len(n_files), function(i) nrow(exprs(fs[[i]])))),
                            error = function(e) NA_integer_)

    tagList(
      tags$div(style = "display:grid;grid-template-columns:1fr 1fr 1fr;gap:8px;",
        info_tile("FILES",      n_files,            "#00B4D8"),
        info_tile("ALL CH",     length(all_ch),     "#2EC4B6"),
        info_tile("FLUOR CH",   length(fluor_ch),   "#00B4D8")
      ),
      tags$div(style = "margin-top:8px;",
        info_tile("TOTAL CELLS",
                  if (is.na(total_cells)) "—" else format(total_cells, big.mark = ","),
                  "#00B4D8", full = TRUE)
      )
    )
  })

  output$fcs_preview <- renderUI({
    fs <- local$flowset
    if (is.null(fs)) return(tags$p(style = "color:#5A7A8A;font-size:12px;",
                                    "Load FCS files to see a preview."))

    # cyto_details() for experiment metadata
    details <- tryCatch(as.data.frame(CytoExploreR::cyto_details(fs)),
                        error = function(e) NULL)

    sample_info <- lapply(seq_len(min(6, length(fs))), function(i) {
      ff    <- fs[[i]]
      sname <- tryCatch(CytoExploreR::cyto_names(fs)[i], error = function(e) sampleNames(fs)[i])
      n     <- nrow(exprs(ff))
      kw    <- keyword(ff)
      list(name = basename(sname), cells = n,
           date = kw[["$DATE"]] %||% "—",
           cyt  = kw[["$CYT"]]  %||% "—")
    })

    rows <- lapply(sample_info, function(s) {
      tags$tr(
        tags$td(style = "padding:4px 8px;font-size:12px;color:#E0E0E0;", s$name),
        tags$td(style = "padding:4px 8px;font-size:12px;color:#2EC4B6;text-align:right;",
                format(s$cells, big.mark = ",")),
        tags$td(style = "padding:4px 8px;font-size:12px;color:#8899AA;", s$date),
        tags$td(style = "padding:4px 8px;font-size:12px;color:#8899AA;", s$cyt)
      )
    })

    tagList(
      tags$table(
        style = "width:100%;border-collapse:collapse;",
        tags$thead(tags$tr(
          lapply(c("File","Cells","Date","Cytometer"), function(h)
            tags$th(style = "padding:4px 8px;font-size:11px;color:#00B4D8;text-align:left;", h))
        )),
        tags$tbody(rows)
      ),
      if (length(fs) > 6)
        tags$p(style = "color:#5A7A8A;font-size:11px;margin-top:6px;",
               sprintf("… and %d more files", length(fs) - 6)),
      if (!is.null(details) && nrow(details) > 0)
        tags$p(style = "font-size:11px;color:#3A5068;margin-top:6px;",
               "cyto_details() metadata available. ", nrow(details), " rows.")
    )
  })
}

# Helpers
info_tile <- function(label, val, color, full = FALSE) {
  style <- if (full)
    "padding:8px;border-radius:4px;background:#243447;grid-column:1/-1;"
  else
    "padding:8px;border-radius:4px;background:#243447;"
  tags$div(style = style,
    tags$div(style = "font-size:10px;color:#5A7A8A;", label),
    tags$div(style = sprintf("font-size:18px;color:%s;font-weight:700;", color), val)
  )
}
