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
          tags$div(style = "margin-bottom: 10px;",
            shinyDirButton(
              ns("fcs_folder"),
              label = "Browse FCS Folder",
              title = "Select folder containing FCS files",
              class = "btn btn-default btn-block",
              icon  = icon("folder-open")
            )
          ),
          uiOutput(ns("selected_folder_label")),
          tags$hr(),
          checkboxInput(ns("recursive"), "Include sub-folders",    value = FALSE),
          checkboxInput(ns("do_clean"),  "Run flowAI quality check (cyto_clean)", value = FALSE),
          checkboxInput(ns("do_barcode"),"Barcode samples (cyto_barcode)",        value = FALSE),
          checkboxInput(ns("parse_names"),"Auto-parse names into experiment details", value = FALSE),
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
                shinyFilesButton(
                  ns("load_annot"),
                  label    = tagList(icon("file-csv"), " Load CSV"),
                  title    = "Select annotation CSV",
                  multiple = FALSE,
                  class    = "btn btn-default btn-sm"
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

  # Robust volume detection — getVolumes()() can silently return nothing
  # inside Electron's packaged R-Portable environment on Windows.
  volumes <- tryCatch({
    vols <- c(Home = path.expand("~"), getVolumes()())
    # If only Home came back, scan drive letters manually
    if (length(vols) <= 1) {
      drives <- setNames(paste0(LETTERS, ":/"), paste0(LETTERS, ":"))
      drives <- drives[file.exists(drives)]
      c(Home = path.expand("~"),
        Desktop = file.path(path.expand("~"), "Desktop"),
        drives)
    } else {
      vols
    }
  }, error = function(e) {
    drives <- setNames(paste0(LETTERS, ":/"), paste0(LETTERS, ":"))
    drives <- drives[file.exists(drives)]
    c(Home = path.expand("~"),
      Desktop = file.path(path.expand("~"), "Desktop"),
      drives)
  })

  shinyDirChoose(input,  "fcs_folder", roots = volumes, session = session,
                 allowDirCreate = FALSE)
  shinyFileChoose(input, "load_annot", roots = volumes, session = session,
                  filetypes = c("csv"))

  local <- reactiveValues(
    folder_path = NULL,
    flowset     = NULL,
    annotation  = data.frame(
      SampleName = character(), Group = character(),
      Treatment  = character(), Replicate = character(),
      TimePoint  = character(), stringsAsFactors = FALSE
    ),
    marker_map  = data.frame(Channel = character(), Marker = character(),
                             stringsAsFactors = FALSE)
  )

  # ── Folder selection ──────────────────────────────────────────────────────
  observeEvent(input$fcs_folder, {
    req(is.list(input$fcs_folder))
    p <- parseDirPath(volumes, input$fcs_folder)
    if (length(p) > 0) {
      local$folder_path <- p
      shared$fcs_folder <- p
    }
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

    withProgress(message = "Loading FCS files…", value = 0, {

      incProgress(0.05, detail = "Scanning for .fcs files…")
      fcs_files <- list.files(folder, pattern = "\\.fcs$",
                              recursive  = isTRUE(input$recursive),
                              full.names = TRUE, ignore.case = TRUE)

      if (length(fcs_files) == 0) {
        showNotification("No FCS files found in the selected folder.",
                         type = "warning", duration = 5)
        shared$status <- "idle"
        return()
      }

      # ── cyto_load: load FCS into ncdfFlowSet ───────────────────────────────
      incProgress(0.15, detail = sprintf("cyto_load(): reading %d files…", length(fcs_files)))
      fs <- tryCatch(
        safe_cyto(
          CytoExploreR::cyto_load(fcs_files),
          "cyto_load failed"
        ),
        error = function(e) {
          showNotification(paste("cyto_load error:", conditionMessage(e)),
                           type = "error", duration = 8)
          NULL
        }
      )

      # Fallback to flowCore if cyto_load unavailable
      if (is.null(fs)) {
        incProgress(0, detail = "Falling back to flowCore::read.flowSet()…")
        valid_files <- Filter(function(f) {
          tryCatch({ read.FCSheader(f); TRUE }, error = function(e) {
            showNotification(paste("Skipping:", basename(f)), type = "warning", duration = 3)
            FALSE
          })
        }, fcs_files)
        if (length(valid_files) == 0) { shared$status <- "idle"; return() }
        fs <- tryCatch(flowCore::read.flowSet(files = valid_files, transformation = FALSE),
                       error = function(e) {
                         showNotification(paste("Load error:", conditionMessage(e)),
                                          type = "error", duration = 8); NULL })
      }
      req(fs)
      incProgress(0.15)

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

      # ── cyto_names_parse: parse details from file names ───────────────────
      annot_df <- if (isTRUE(input$parse_names)) {
        tryCatch(
          {
            parsed <- safe_cyto(CytoExploreR::cyto_names_parse(fs), "cyto_names_parse failed")
            if (is.data.frame(parsed)) parsed else NULL
          },
          error = function(e) NULL
        )
      } else NULL

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

      local$flowset    <- fs
      local$annotation <- annot_df
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
      shared$markers <- if (!is.null(markers)) markers else all_ch

      incProgress(0.1, detail = "Done!")
      showNotification(
        sprintf("Loaded %d FCS files via cyto_load(). %d fluorescent channels detected.",
                length(fs), length(fluor_ch)),
        type = "message", duration = 4
      )
    })

    shared$status <- "idle"
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

  observeEvent(input$load_annot, {
    req(is.list(input$load_annot))
    fi <- parseFilePaths(volumes, input$load_annot)
    req(nrow(fi) > 0)
    tryCatch({
      df <- read.csv(as.character(fi$datapath), stringsAsFactors = FALSE)
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
  style <- if (full) {
    "padding:8px;border-radius:4px;background:#243447;"
  } else {
    "padding:8px;border-radius:4px;background:#243447;"
  }
  tags$div(style = style,
    tags$div(style = "font-size:10px;color:#5A7A8A;", label),
    tags$div(style = sprintf("font-size:18px;color:%s;font-weight:700;", color), val)
  )
}
