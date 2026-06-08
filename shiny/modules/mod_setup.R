# StreamFLOW - mod_setup.R
# FCS file loading and sample annotation module

# ── UI ────────────────────────────────────────────────────────────────────────
setupUI <- function(id) {
  ns <- NS(id)
  tagList(
    fluidRow(
      # Left panel: folder selection and load controls
      column(4,
        box(
          title = "Data Import", width = NULL, solidHeader = TRUE,
          tags$div(style = "margin-bottom: 10px;",
            shinyDirButton(
              ns("fcs_folder"),
              label       = "Browse FCS Folder",
              title       = "Select folder containing FCS files",
              class       = "btn btn-default btn-block",
              icon        = icon("folder-open")
            )
          ),
          uiOutput(ns("selected_folder_label")),
          tags$hr(),
          checkboxInput(ns("recursive"), "Include sub-folders", value = FALSE),
          actionButton(
            ns("load_fcs_btn"),
            label = tagList(icon("upload"), " Load FCS Files"),
            class = "btn btn-primary btn-block"
          ),
          tags$hr(),
          uiOutput(ns("flowset_summary"))
        ),

        box(
          title = "Channel Selection", width = NULL, solidHeader = TRUE,
          uiOutput(ns("channel_selector")),
          tags$small(class = "text-muted", "Uncheck channels to exclude from analysis.")
        )
      ),

      # Right panel: annotation table
      column(8,
        box(
          title = "Sample Annotation", width = NULL, solidHeader = TRUE,
          fluidRow(
            column(12,
              tags$div(
                style = "display: flex; gap: 8px; flex-wrap: wrap; margin-bottom: 10px;",
                actionButton(ns("add_row"),    tagList(icon("plus"),   " Add Row"),    class = "btn btn-default btn-sm"),
                actionButton(ns("remove_row"), tagList(icon("minus"),  " Remove Row"), class = "btn btn-danger btn-sm"),
                actionButton(ns("save_annot"), tagList(icon("save"),   " Save"),       class = "btn btn-success btn-sm"),
                shinyFilesButton(
                  ns("load_annot"),
                  label = tagList(icon("file-csv"), " Load CSV"),
                  title = "Select annotation CSV",
                  multiple = FALSE,
                  class = "btn btn-default btn-sm"
                )
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

  local_state <- reactiveValues(
    folder_path = NULL,
    flowset     = NULL,
    annotation  = data.frame(
      SampleName = character(),
      Group      = character(),
      Treatment  = character(),
      Replicate  = character(),
      TimePoint  = character(),
      stringsAsFactors = FALSE
    )
  )

  # ── Filesystem volumes ──────────────────────────────────────────────────
  volumes <- c(Home = path.expand("~"), getVolumes()())

  shinyDirChoose(input, "fcs_folder", roots = volumes, session = session,
                 restrictions = system.file(package = "base"))

  shinyFileChoose(input, "load_annot", roots = volumes, session = session,
                  filetypes = c("csv"))

  # ── Folder selection ────────────────────────────────────────────────────
  observeEvent(input$fcs_folder, {
    req(is.list(input$fcs_folder))
    path <- parseDirPath(volumes, input$fcs_folder)
    if (length(path) > 0) {
      local_state$folder_path <- path
      shared$fcs_folder <- path
    }
  })

  output$selected_folder_label <- renderUI({
    if (!is.null(local_state$folder_path)) {
      tags$p(
        style = "font-size: 11px; color: #2EC4B6; word-break: break-all; margin: 6px 0;",
        icon("check-circle"), " ", local_state$folder_path
      )
    } else {
      tags$p(style = "font-size: 11px; color: #5A7A8A; margin: 6px 0;", "No folder selected")
    }
  })

  # ── Load FCS files ──────────────────────────────────────────────────────
  observeEvent(input$load_fcs_btn, {
    req(local_state$folder_path)
    folder <- local_state$folder_path

    fcs_files <- list.files(
      folder,
      pattern   = "\\.fcs$",
      recursive = input$recursive,
      full.names = TRUE,
      ignore.case = TRUE
    )

    if (length(fcs_files) == 0) {
      showNotification("No FCS files found in the selected folder.", type = "warning", duration = 5)
      return()
    }

    shared$status <- "busy"

    withProgress(message = "Loading FCS files...", value = 0, {
      fs <- tryCatch({
        incProgress(0.1, detail = sprintf("Reading %d files...", length(fcs_files)))

        # Validate files first
        valid_files <- character()
        for (f in fcs_files) {
          tryCatch({
            hdr <- read.FCSheader(f)
            valid_files <- c(valid_files, f)
          }, error = function(e) {
            showNotification(
              sprintf("Skipping corrupt file: %s", basename(f)),
              type = "warning", duration = 4
            )
          })
        }

        if (length(valid_files) == 0) {
          showNotification("No valid FCS files could be loaded.", type = "error", duration = 5)
          return(NULL)
        }

        incProgress(0.4, detail = "Parsing flowSet...")
        flowCore::read.flowSet(files = valid_files, transformation = FALSE)
      }, error = function(e) {
        showNotification(
          paste("Error loading FCS files:", conditionMessage(e)),
          type = "error", duration = 8
        )
        NULL
      })

      if (!is.null(fs)) {
        incProgress(0.3, detail = "Updating annotation table...")
        local_state$flowset <- fs
        shared$raw_flowset  <- fs
        shared$n_samples    <- length(fs)
        shared$experiment_name <- basename(folder)

        # Auto-generate annotation from sample names
        sample_names <- sampleNames(fs)
        local_state$annotation <- data.frame(
          SampleName = sample_names,
          Group      = rep("Group1", length(sample_names)),
          Treatment  = rep("Control", length(sample_names)),
          Replicate  = as.character(seq_along(sample_names)),
          TimePoint  = rep("T0", length(sample_names)),
          stringsAsFactors = FALSE
        )
        shared$annotation <- local_state$annotation

        # Default channels (exclude scatter unless user wants them)
        all_channels <- colnames(fs)
        scatter_pat  <- "^(FSC|SSC)"
        scatter_ch   <- grep(scatter_pat, all_channels, value = TRUE, ignore.case = TRUE)
        fluor_ch     <- setdiff(all_channels, scatter_ch)
        shared$channels <- all_channels

        incProgress(0.2, detail = "Done!")
        showNotification(
          sprintf("Loaded %d FCS files successfully.", length(fs)),
          type = "message", duration = 4
        )
      }
    })

    shared$status <- "idle"
  })

  # ── Annotation table ────────────────────────────────────────────────────
  output$annotation_table <- renderDT({
    req(nrow(local_state$annotation) > 0)
    datatable(
      local_state$annotation,
      editable  = list(target = "cell", disable = list(columns = 0)),
      selection = "single",
      rownames  = FALSE,
      options   = list(
        pageLength = 15,
        dom        = "tip",
        scrollX    = TRUE,
        columnDefs = list(list(className = "dt-center", targets = "_all"))
      ),
      class = "compact stripe hover"
    )
  }, server = TRUE)

  # Cell edit handler
  observeEvent(input$annotation_table_cell_edit, {
    info <- input$annotation_table_cell_edit
    df   <- local_state$annotation
    df[info$row, info$col + 1] <- info$value
    local_state$annotation <- df
    shared$annotation      <- df
  })

  # Add row
  observeEvent(input$add_row, {
    new_row <- data.frame(
      SampleName = "NewSample",
      Group      = "Group1",
      Treatment  = "Control",
      Replicate  = "1",
      TimePoint  = "T0",
      stringsAsFactors = FALSE
    )
    local_state$annotation <- rbind(local_state$annotation, new_row)
    shared$annotation      <- local_state$annotation
  })

  # Remove row
  observeEvent(input$remove_row, {
    sel <- input$annotation_table_rows_selected
    if (!is.null(sel) && length(sel) > 0) {
      local_state$annotation <- local_state$annotation[-sel, , drop = FALSE]
      shared$annotation      <- local_state$annotation
    } else {
      showNotification("Select a row to remove.", type = "warning", duration = 3)
    }
  })

  # Save annotation
  observeEvent(input$save_annot, {
    req(local_state$folder_path)
    out_path <- file.path(local_state$folder_path, "sample_annotation.csv")
    tryCatch({
      write.csv(local_state$annotation, out_path, row.names = FALSE)
      showNotification(paste("Annotation saved to", out_path), type = "message", duration = 4)
    }, error = function(e) {
      showNotification(paste("Failed to save:", conditionMessage(e)), type = "error", duration = 5)
    })
  })

  # Load annotation CSV
  observeEvent(input$load_annot, {
    req(is.list(input$load_annot))
    file_info <- parseFilePaths(volumes, input$load_annot)
    req(nrow(file_info) > 0)
    tryCatch({
      df <- read.csv(as.character(file_info$datapath), stringsAsFactors = FALSE)
      local_state$annotation <- df
      shared$annotation      <- df
      showNotification("Annotation loaded.", type = "message", duration = 3)
    }, error = function(e) {
      showNotification(paste("Failed to load CSV:", conditionMessage(e)), type = "error", duration = 5)
    })
  })

  # ── flowSet summary ─────────────────────────────────────────────────────
  output$flowset_summary <- renderUI({
    fs <- local_state$flowset
    if (is.null(fs)) {
      return(tags$p(style = "color: #5A7A8A; font-size: 12px;", "No data loaded"))
    }
    n_files  <- length(fs)
    channels <- colnames(fs)
    total_cells <- tryCatch(
      sum(sapply(seq_len(n_files), function(i) nrow(exprs(fs[[i]])))),
      error = function(e) NA_integer_
    )
    tagList(
      tags$div(style = "display: grid; grid-template-columns: 1fr 1fr; gap: 8px;",
        tags$div(class = "info-box",
          style = "padding: 8px; border-radius: 4px; background: #243447;",
          tags$div(style = "font-size: 10px; color: #5A7A8A;", "FILES"),
          tags$div(style = "font-size: 20px; color: #00B4D8; font-weight: 700;", n_files)
        ),
        tags$div(class = "info-box",
          style = "padding: 8px; border-radius: 4px; background: #243447;",
          tags$div(style = "font-size: 10px; color: #5A7A8A;", "CHANNELS"),
          tags$div(style = "font-size: 20px; color: #2EC4B6; font-weight: 700;", length(channels))
        ),
        tags$div(
          class = "info-box",
          style = "padding: 8px; border-radius: 4px; background: #243447; grid-column: 1 / -1;",
          tags$div(style = "font-size: 10px; color: #5A7A8A;", "TOTAL CELLS"),
          tags$div(
            style = "font-size: 20px; color: #00B4D8; font-weight: 700;",
            if (is.na(total_cells)) "—" else format(total_cells, big.mark = ",")
          )
        )
      )
    )
  })

  # ── Channel selector ────────────────────────────────────────────────────
  output$channel_selector <- renderUI({
    fs <- local_state$flowset
    if (is.null(fs)) return(tags$p(style = "color: #5A7A8A; font-size: 12px;", "Load FCS files first."))
    channels <- colnames(fs)
    checkboxGroupInput(
      ns("selected_channels"),
      label   = NULL,
      choices  = channels,
      selected = channels
    )
  })

  observeEvent(input$selected_channels, {
    shared$channels <- input$selected_channels
  })

  # ── FCS preview ─────────────────────────────────────────────────────────
  output$fcs_preview <- renderUI({
    fs <- local_state$flowset
    if (is.null(fs)) return(tags$p(style = "color: #5A7A8A; font-size: 12px;", "Load FCS files to see a preview."))

    sample_info <- tryCatch({
      lapply(seq_len(min(5, length(fs))), function(i) {
        ff   <- fs[[i]]
        name <- sampleNames(fs)[i]
        n    <- nrow(exprs(ff))
        kw   <- keyword(ff)
        date <- kw[["$DATE"]] %||% "—"
        cyt  <- kw[["$CYT"]]  %||% "—"
        list(name = basename(name), cells = n, date = date, cyt = cyt)
      })
    }, error = function(e) list())

    rows <- lapply(sample_info, function(s) {
      tags$tr(
        tags$td(style = "padding: 4px 8px; font-size: 12px; color: #E0E0E0;", s$name),
        tags$td(style = "padding: 4px 8px; font-size: 12px; color: #2EC4B6; text-align: right;",
                format(s$cells, big.mark = ",")),
        tags$td(style = "padding: 4px 8px; font-size: 12px; color: #8899AA;", s$date),
        tags$td(style = "padding: 4px 8px; font-size: 12px; color: #8899AA;", s$cyt)
      )
    })

    tagList(
      tags$table(
        style = "width: 100%; border-collapse: collapse;",
        tags$thead(
          tags$tr(
            tags$th(style = "padding: 4px 8px; font-size: 11px; color: #00B4D8; text-align: left;", "File"),
            tags$th(style = "padding: 4px 8px; font-size: 11px; color: #00B4D8; text-align: right;", "Cells"),
            tags$th(style = "padding: 4px 8px; font-size: 11px; color: #00B4D8;", "Date"),
            tags$th(style = "padding: 4px 8px; font-size: 11px; color: #00B4D8;", "Cytometer")
          )
        ),
        tags$tbody(rows)
      ),
      if (length(fs) > 5) {
        tags$p(style = "color: #5A7A8A; font-size: 11px; margin-top: 6px;",
               sprintf("... and %d more files", length(fs) - 5))
      }
    )
  })
}

# Null-coalescing operator
`%||%` <- function(a, b) if (!is.null(a) && !is.na(a) && nchar(a) > 0) a else b
