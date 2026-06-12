# StreamFLOW - ui.R

source("modules/mod_setup.R")
source("modules/mod_compensation.R")
source("modules/mod_transformation.R")
source("modules/mod_gating.R")
source("modules/mod_visualization.R")
source("modules/mod_dimredux.R")
source("modules/mod_statistics.R")
source("modules/mod_clustering.R")
source("modules/mod_popout.R")
source("modules/mod_workspace.R")

full_ui <- tagList(
  useShinyjs(),
  tags$head(
    tags$link(
      rel = "preconnect",
      href = "https://fonts.googleapis.com"
    ),
    tags$link(
      rel = "stylesheet",
      href = "https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700;800&display=swap"
    ),
    tags$style(HTML("
      /* ── Base Reset & Font ── */
      * { box-sizing: border-box; }
      body, html {
        font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif !important;
        background-color: #0D1B2A !important;
        color: #E0E0E0 !important;
        margin: 0; padding: 0;
        overflow: hidden;
      }

      /* ── Custom Title Bar ── */
      #custom-titlebar {
        position: fixed;
        top: 0; left: 0; right: 0;
        height: 36px;
        background-color: #081420;
        display: flex;
        align-items: center;
        justify-content: space-between;
        z-index: 9999;
        -webkit-app-region: drag;
        user-select: none;
        border-bottom: 1px solid #0A1F30;
      }
      #titlebar-left {
        display: flex;
        align-items: center;
        padding-left: 14px;
        gap: 8px;
        -webkit-app-region: drag;
      }
      #titlebar-logo {
        color: #00B4D8;
        font-size: 13px;
        font-weight: 700;
        letter-spacing: 2px;
      }
      #titlebar-right {
        display: flex;
        align-items: center;
        -webkit-app-region: no-drag;
      }
      .title-btn {
        width: 46px;
        height: 36px;
        border: none;
        background: transparent;
        color: #8899AA;
        font-size: 12px;
        cursor: pointer;
        display: flex;
        align-items: center;
        justify-content: center;
        transition: background 0.15s, color 0.15s;
        -webkit-app-region: no-drag;
      }
      .title-btn:hover { background: #1B2A3B; color: #E0E0E0; }
      .title-btn.close-btn:hover { background: #C42B1C; color: #fff; }

      /* ── Sidebar ── */
      .main-sidebar {
        margin-top: 36px !important;
        background-color: #081420 !important;
        border-right: 1px solid #0A1F30 !important;
        position: fixed !important;
      }
      .sidebar-menu > li > a {
        color: #8899AA !important;
        font-size: 13px;
        font-weight: 500;
        padding: 11px 16px !important;
        transition: all 0.15s;
        border-left: 3px solid transparent;
      }
      .sidebar-menu > li > a:hover,
      .sidebar-menu > li.active > a {
        color: #00B4D8 !important;
        background-color: #0D1B2A !important;
        border-left-color: #00B4D8 !important;
      }
      .sidebar-menu > li.header {
        color: #3A5068 !important;
        font-size: 10px !important;
        letter-spacing: 2px;
        padding: 12px 16px 4px !important;
      }

      /* Sidebar logo area */
      .logo-area {
        background-color: #081420 !important;
        border-bottom: 1px solid #0A1F30 !important;
        height: 50px !important;
      }
      .logo-area .logo-lg,
      .logo-area .logo-mini {
        color: #00B4D8 !important;
        font-weight: 800 !important;
        letter-spacing: 3px !important;
        font-size: 18px !important;
      }

      /* ── Content wrapper ── */
      .content-wrapper {
        background-color: #0D1B2A !important;
        margin-top: 36px !important;
        min-height: calc(100vh - 36px - 30px) !important;
        padding-bottom: 30px !important;
      }
      .content {
        padding: 16px !important;
      }

      /* ── Cards / Boxes ── */
      .box {
        background-color: #1B2A3B !important;
        border: 1px solid #243447 !important;
        border-top: 3px solid #00B4D8 !important;
        border-radius: 6px !important;
        box-shadow: 0 2px 12px rgba(0,0,0,0.4) !important;
      }
      .box-header {
        background-color: #1B2A3B !important;
        color: #E0E0E0 !important;
        border-bottom: 1px solid #243447 !important;
        padding: 10px 14px !important;
      }
      .box-title {
        font-size: 13px !important;
        font-weight: 600 !important;
        color: #00B4D8 !important;
        letter-spacing: 0.5px;
      }
      .box-body {
        padding: 14px !important;
        color: #E0E0E0 !important;
      }

      /* ── Buttons ── */
      .btn-primary, .btn-info {
        background-color: #00B4D8 !important;
        border-color: #00B4D8 !important;
        color: #0D1B2A !important;
        font-weight: 600 !important;
        font-size: 12px !important;
        border-radius: 4px !important;
        transition: all 0.15s !important;
      }
      .btn-primary:hover, .btn-info:hover {
        background-color: #0090B0 !important;
        border-color: #0090B0 !important;
      }
      .btn-success {
        background-color: #2EC4B6 !important;
        border-color: #2EC4B6 !important;
        color: #0D1B2A !important;
        font-weight: 600 !important;
        border-radius: 4px !important;
      }
      .btn-danger {
        background-color: #C0392B !important;
        border-color: #C0392B !important;
        border-radius: 4px !important;
      }
      .btn-default {
        background-color: #243447 !important;
        border-color: #2E4460 !important;
        color: #E0E0E0 !important;
        border-radius: 4px !important;
      }
      .btn-default:hover {
        background-color: #2E4460 !important;
        color: #E0E0E0 !important;
      }

      /* ── Form Controls ── */
      .form-control, .selectize-input {
        background-color: #243447 !important;
        border: 1px solid #2E4460 !important;
        color: #E0E0E0 !important;
        border-radius: 4px !important;
        font-size: 13px !important;
      }
      .form-control:focus, .selectize-input.focus {
        border-color: #00B4D8 !important;
        box-shadow: 0 0 0 2px rgba(0,180,216,0.2) !important;
      }
      .selectize-dropdown {
        background-color: #1B2A3B !important;
        border: 1px solid #2E4460 !important;
        color: #E0E0E0 !important;
      }
      .selectize-dropdown .option:hover,
      .selectize-dropdown .option.active {
        background-color: #243447 !important;
        color: #00B4D8 !important;
      }
      label {
        color: #B0C4D8 !important;
        font-size: 12px !important;
        font-weight: 500 !important;
      }

      /* ── Checkboxes & Radios ── */
      .checkbox label, .radio label {
        color: #E0E0E0 !important;
      }

      /* ── DataTables ── */
      .dataTables_wrapper { color: #E0E0E0 !important; }
      table.dataTable {
        background-color: #1B2A3B !important;
        color: #E0E0E0 !important;
        border: 1px solid #243447 !important;
      }
      table.dataTable thead th {
        background-color: #243447 !important;
        color: #00B4D8 !important;
        border-bottom: 1px solid #2E4460 !important;
        font-size: 12px;
        font-weight: 600;
      }
      table.dataTable tbody tr {
        background-color: #1B2A3B !important;
        color: #E0E0E0 !important;
      }
      table.dataTable tbody tr:hover {
        background-color: #243447 !important;
      }
      table.dataTable tbody tr.selected {
        background-color: #1A3A50 !important;
        color: #00B4D8 !important;
      }
      .dataTables_filter input,
      .dataTables_length select {
        background-color: #243447 !important;
        border: 1px solid #2E4460 !important;
        color: #E0E0E0 !important;
        border-radius: 4px !important;
      }
      .dataTables_info,
      .dataTables_paginate { color: #8899AA !important; }
      .paginate_button { color: #8899AA !important; }
      .paginate_button.current, .paginate_button:hover {
        color: #00B4D8 !important;
        background: #243447 !important;
        border-color: #2E4460 !important;
      }

      /* ── Sliders ── */
      .irs--shiny .irs-bar { background: #00B4D8; border-color: #00B4D8; }
      .irs--shiny .irs-handle { background: #00B4D8 !important; border-color: #0090B0 !important; }
      .irs--shiny .irs-from, .irs--shiny .irs-to, .irs--shiny .irs-single {
        background: #00B4D8 !important;
      }
      .irs--shiny .irs-line { background: #243447; border-color: #243447; }
      .irs--shiny .irs-grid-text { color: #8899AA; }
      .irs--shiny .irs-min, .irs--shiny .irs-max { color: #8899AA; }

      /* ── Nav Tabs ── */
      .nav-tabs { border-bottom: 1px solid #243447 !important; }
      .nav-tabs > li > a {
        color: #8899AA !important;
        background-color: transparent !important;
        border: 1px solid transparent !important;
        font-size: 13px;
      }
      .nav-tabs > li.active > a,
      .nav-tabs > li > a:hover {
        color: #00B4D8 !important;
        background-color: #1B2A3B !important;
        border-color: #243447 !important;
      }
      .tab-content { padding-top: 12px; }

      /* ── Status Bar ── */
      #status-bar {
        position: fixed;
        bottom: 0; left: 0; right: 0;
        height: 30px;
        background-color: #050E18;
        border-top: 1px solid #0A1F30;
        display: flex;
        align-items: center;
        padding: 0 12px;
        gap: 24px;
        z-index: 9998;
        font-size: 11px;
        color: #5A7A8A;
      }
      .status-item { display: flex; align-items: center; gap: 5px; }
      .status-dot {
        width: 6px; height: 6px;
        border-radius: 50%;
        background: #2EC4B6;
      }
      .status-dot.idle { background: #5A7A8A; }

      /* ── Notifications ── */
      .shiny-notification {
        background-color: #1B2A3B !important;
        border: 1px solid #2E4460 !important;
        color: #E0E0E0 !important;
        border-radius: 6px !important;
      }
      .shiny-notification-error {
        border-left: 4px solid #C0392B !important;
      }
      .shiny-notification-warning {
        border-left: 4px solid #F39C12 !important;
      }
      .shiny-notification-message {
        border-left: 4px solid #00B4D8 !important;
      }

      /* ── Progress bar ── */
      .progress { background-color: #243447 !important; }
      .progress-bar { background-color: #00B4D8 !important; }
      .shiny-progress-container { top: 36px !important; }
      .shiny-progress .progress { margin: 4px 0; }

      /* ── Scrollbar ── */
      ::-webkit-scrollbar { width: 6px; height: 6px; }
      ::-webkit-scrollbar-track { background: #0D1B2A; }
      ::-webkit-scrollbar-thumb { background: #2E4460; border-radius: 3px; }
      ::-webkit-scrollbar-thumb:hover { background: #00B4D8; }

      /* ── Spinner ── */
      .shiny-spinner-output-container .load-container .loader {
        border-top-color: #00B4D8 !important;
      }

      /* ── Well panels ── */
      .well {
        background-color: #243447 !important;
        border: 1px solid #2E4460 !important;
        border-radius: 4px !important;
      }

      /* ── rhandsontable ── */
      .handsontable { color: #E0E0E0 !important; }
      .handsontable td { background-color: #1B2A3B !important; color: #E0E0E0 !important; }
      .handsontable th { background-color: #243447 !important; color: #00B4D8 !important; }

      /* ── Plotly modebar ── */
      .modebar { background: transparent !important; }
      .modebar-btn path { fill: #5A7A8A !important; }
      .modebar-btn:hover path { fill: #00B4D8 !important; }

      /* ── Misc ── */
      hr { border-color: #243447 !important; }
      .shiny-output-error { color: #E74C3C !important; font-size: 12px; }
      .text-muted { color: #5A7A8A !important; }
      h4, h5, h6 { color: #00B4D8 !important; font-weight: 600; }

      /* Info box */
      .info-box {
        background: #1B2A3B !important;
        border-radius: 6px !important;
        box-shadow: none !important;
      }
      .info-box-icon { background: #243447 !important; }

      /* ── Dark-theme modals (all app modals) ── */
      /* Scoped to #shiny-modal to avoid affecting third-party widget modals */
      #shiny-modal .modal-content {
        background-color: #1B2A3B !important;
        border: 1px solid #2E4460 !important;
        color: #E0E0E0 !important;
      }
      #shiny-modal .modal-header {
        background-color: #243447 !important;
        border-bottom: 1px solid #2E4460 !important;
        color: #00B4D8 !important;
      }
      #shiny-modal .modal-header .modal-title { color: #00B4D8 !important; font-weight: 600; }
      #shiny-modal .modal-header .close { color: #E0E0E0 !important; opacity: 0.8; }
      #shiny-modal .modal-header .close:hover { color: #00B4D8 !important; opacity: 1; }
      #shiny-modal .modal-body {
        background-color: #1B2A3B !important;
        color: #E0E0E0 !important;
      }
      #shiny-modal .modal-footer {
        background-color: #243447 !important;
        border-top: 1px solid #2E4460 !important;
      }
      /* modal backdrop dimming */
      .modal-backdrop { background-color: #000 !important; opacity: 0.7 !important; }
    "))
  ),

  # Custom title bar
  tags$div(
    id = "custom-titlebar",
    tags$div(
      id = "titlebar-left",
      tags$span(id = "titlebar-logo", "StreamFLOW")
    ),
    tags$div(
      id = "titlebar-right",
      tags$button(
        class = "title-btn",
        id    = "btn-minimize",
        HTML("&#x2212;"),
        onclick = "if(window.electronAPI) window.electronAPI.minimize();"
      ),
      tags$button(
        class = "title-btn",
        id    = "btn-maximize",
        HTML("&#x25A1;"),
        onclick = "if(window.electronAPI) window.electronAPI.maximize();"
      ),
      tags$button(
        class = "title-btn close-btn",
        id    = "btn-close",
        HTML("&#x2715;"),
        onclick = "if(window.electronAPI) window.electronAPI.close();"
      )
    )
  ),

  dashboardPage(
    skin = "black",

    dashboardHeader(
      title = tags$span(
        style = "color: #00B4D8; font-weight: 800; letter-spacing: 2px;",
        "StreamFLOW"
      ),
      titleWidth = 240
    ),

    dashboardSidebar(
      width = 240,
      tags$div(
        style = "padding: 8px 12px; color: #3A5068; font-size: 10px; letter-spacing: 2px; text-transform: uppercase; margin-top: 4px;",
        "Analysis Pipeline"
      ),
      sidebarMenu(
        id = "sidebar_menu",
        menuItem("Setup",          tabName = "setup",          icon = icon("folder-open")),
        menuItem("Compensation",   tabName = "compensation",   icon = icon("sliders-h")),
        menuItem("Transformation", tabName = "transformation", icon = icon("exchange-alt")),
        menuItem("Gating",         tabName = "gating",         icon = icon("draw-polygon")),
        menuItem("Visualization",  tabName = "visualization",  icon = icon("chart-area")),
        menuItem("Dim. Reduction", tabName = "dimredux",       icon = icon("project-diagram")),
        menuItem("Clustering",     tabName = "clustering",     icon = icon("braille")),
        menuItem("Statistics",     tabName = "statistics",     icon = icon("table"))
      ),
      tags$hr(style = "border-color: #0A1F30; margin: 8px 0;"),
      tags$div(
        style = "padding: 6px 14px; font-size: 11px; color: #3A5068;",
        "Flow Cytometry Analysis Suite"
      )
    ),

    dashboardBody(
      tabItems(
        tabItem(tabName = "setup",          setupUI("setup")),
        tabItem(tabName = "compensation",   compensationUI("compensation")),
        tabItem(tabName = "transformation", transformationUI("transformation")),
        tabItem(tabName = "gating",         gatingUI("gating")),
        tabItem(tabName = "visualization",  visualizationUI("visualization")),
        tabItem(tabName = "dimredux",       dimreduxUI("dimredux")),
        tabItem(tabName = "clustering",     clusteringUI("clustering")),
        tabItem(tabName = "statistics",     statisticsUI("statistics"))
      )
    )
  ),

  # Hidden workspace save/load triggers (driven by the File menu)
  workspaceUI("workspace"),

  # Status bar
  tags$div(
    id = "status-bar",
    tags$div(
      class = "status-item",
      tags$div(class = "status-dot", id = "status-dot-main"),
      tags$span("Experiment:"),
      uiOutput("status_experiment", inline = TRUE)
    ),
    tags$div(
      class = "status-item",
      tags$span("Samples:"),
      uiOutput("status_samples", inline = TRUE)
    ),
    tags$div(
      class = "status-item",
      tags$span("Memory:"),
      uiOutput("status_memory", inline = TRUE)
    ),
    tags$div(
      class = "status-item",
      style = "margin-left: auto;",
      tags$span(r_version_string())
    )
  ),

  # JS bridge for Electron menu events
  tags$script(HTML("
    if (window.electronAPI) {
      window.electronAPI.onOpenFcsFolder(function() {
        Shiny.setInputValue('electron_menu_event', 'open_fcs_folder', {priority: 'event'});
      });
      window.electronAPI.onSaveSession(function() {
        Shiny.setInputValue('electron_menu_event', 'save_session', {priority: 'event'});
      });
      window.electronAPI.onExportResults(function() {
        Shiny.setInputValue('electron_menu_event', 'export_results', {priority: 'event'});
      });
      if (window.electronAPI.onOpenWorkspace) {
        window.electronAPI.onOpenWorkspace(function() {
          Shiny.setInputValue('electron_menu_event', 'open_workspace', {priority: 'event'});
        });
      }
      window.electronAPI.onToggleSidebar(function() {
        $('body').toggleClass('sidebar-collapse');
      });
    }

    // FlowJo-style pop-out: the R server asks the renderer to open a focused
    // single-sample window through Electron.
    if (window.Shiny) {
      Shiny.addCustomMessageHandler('streamflow_open_popout', function(msg) {
        if (window.electronAPI && window.electronAPI.openSampleWindow) {
          window.electronAPI.openSampleWindow(msg);
        }
      });
    }

    // ── Native file/folder pickers ──────────────────────────────────────────
    // Replace the broken shinyFiles modal with Electron's native dialogs. Each
    // helper posts the chosen path back to a Shiny input as {path, ts}. When the
    // app runs standalone in a browser/RStudio (no Electron), it sets
    // {path: null, error: 'no_electron'} so the server can warn instead of crash.
    window.streamflowPickFolder = function(inputId, title) {
      if (!window.electronAPI || !window.electronAPI.selectFolder) {
        Shiny.setInputValue(inputId, {path: null, error: 'no_electron'}, {priority: 'event'});
        return;
      }
      window.electronAPI.selectFolder({ title: title }).then(function(result) {
        if (!result.canceled && result.path) {
          Shiny.setInputValue(inputId, {path: result.path, ts: Date.now()}, {priority: 'event'});
        }
      });
    };

    window.streamflowPickFile = function(inputId, title, filters) {
      if (!window.electronAPI || !window.electronAPI.selectFile) {
        Shiny.setInputValue(inputId, {path: null, error: 'no_electron'}, {priority: 'event'});
        return;
      }
      window.electronAPI.selectFile({ title: title, filters: filters }).then(function(result) {
        if (!result.canceled && result.path) {
          Shiny.setInputValue(inputId, {path: result.path, ts: Date.now()}, {priority: 'event'});
        }
      });
    };

    window.streamflowSaveFile = function(inputId, title, filters, defaultPath) {
      if (!window.electronAPI || !window.electronAPI.saveFile) {
        Shiny.setInputValue(inputId, {path: null, error: 'no_electron'}, {priority: 'event'});
        return;
      }
      window.electronAPI.saveFile({ title: title, filters: filters, defaultPath: defaultPath }).then(function(result) {
        if (!result.canceled && result.path) {
          Shiny.setInputValue(inputId, {path: result.path, ts: Date.now()}, {priority: 'event'});
        }
      });
    };
  "))
)

# Serve the focused pop-out layout when Electron requests ?view=popout, otherwise
# the full dashboard. ui is a request-aware function so it can branch per session.
ui <- function(req) {
  qs <- shiny::parseQueryString(req$QUERY_STRING %||% "")
  if (identical(qs$view, "popout")) return(popout_ui(qs))
  full_ui
}
