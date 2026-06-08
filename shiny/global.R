# StreamFLOW - global.R
# Loads all libraries, parses CLI args, sets Shiny options

suppressPackageStartupMessages({
  library(shiny)
  library(shinydashboard)
  library(shinyFiles)
  library(shinyjs)
  library(shinycssloaders)
  library(DT)
  library(plotly)
  library(ggplot2)
  library(dplyr)
  library(tidyr)
  library(colourpicker)
  library(flowCore)
  library(flowWorkspace)
  library(openCyto)
  library(CytoExploreR)
})

# Parse command-line arguments to get the port number
args <- commandArgs(trailingOnly = TRUE)
shiny_port <- 3838L

if (length(args) >= 2) {
  port_idx <- which(args == "--port")
  if (length(port_idx) > 0 && port_idx < length(args)) {
    parsed <- suppressWarnings(as.integer(args[port_idx + 1]))
    if (!is.na(parsed) && parsed > 1024 && parsed < 65535) {
      shiny_port <- parsed
    }
  }
}

# Set Shiny options
options(shiny.port = shiny_port)
options(shiny.host = "127.0.0.1")
options(shiny.launch.browser = FALSE)

# Increase upload size limit for large FCS files (500MB)
options(shiny.maxRequestSize = 500 * 1024^2)

# ggplot2 dark theme consistent with the app
theme_streamflow <- function() {
  theme_minimal(base_size = 12) +
    theme(
      plot.background    = element_rect(fill = "#0D1B2A", color = NA),
      panel.background   = element_rect(fill = "#1B2A3B", color = NA),
      panel.grid.major   = element_line(color = "#243447", size = 0.4),
      panel.grid.minor   = element_line(color = "#1E2F40", size = 0.2),
      axis.text          = element_text(color = "#B0C4D8"),
      axis.title         = element_text(color = "#E0E0E0"),
      plot.title         = element_text(color = "#00B4D8", face = "bold"),
      legend.background  = element_rect(fill = "#1B2A3B", color = NA),
      legend.text        = element_text(color = "#E0E0E0"),
      legend.title       = element_text(color = "#00B4D8"),
      strip.background   = element_rect(fill = "#243447", color = NA),
      strip.text         = element_text(color = "#E0E0E0")
    )
}

# Standard plotly layout for dark theme
plotly_dark_layout <- function(p, title = NULL, xlab = NULL, ylab = NULL) {
  p %>%
    layout(
      title  = list(text = title, font = list(color = "#00B4D8", size = 14)),
      xaxis  = list(
        title      = list(text = xlab, font = list(color = "#E0E0E0")),
        tickfont   = list(color = "#B0C4D8"),
        gridcolor  = "#243447",
        zerolinecolor = "#243447",
        color      = "#E0E0E0"
      ),
      yaxis  = list(
        title      = list(text = ylab, font = list(color = "#E0E0E0")),
        tickfont   = list(color = "#B0C4D8"),
        gridcolor  = "#243447",
        zerolinecolor = "#243447",
        color      = "#E0E0E0"
      ),
      paper_bgcolor = "#0D1B2A",
      plot_bgcolor  = "#1B2A3B",
      legend = list(font = list(color = "#E0E0E0")),
      margin = list(l = 55, r = 20, t = 40, b = 55)
    )
}

# Helper: safe CytoExploreR call wrapper
safe_cyto <- function(expr, error_msg = "Operation failed") {
  tryCatch(
    expr,
    error = function(e) {
      message(sprintf("[StreamFLOW ERROR] %s: %s", error_msg, conditionMessage(e)))
      NULL
    },
    warning = function(w) {
      message(sprintf("[StreamFLOW WARN] %s: %s", error_msg, conditionMessage(w)))
      withCallingHandlers(expr, warning = function(w) invokeRestart("muffleWarning"))
    }
  )
}

# Helper: format file size
format_bytes <- function(bytes) {
  if (is.na(bytes) || bytes == 0) return("0 B")
  units <- c("B", "KB", "MB", "GB")
  idx   <- min(floor(log(bytes, 1024)), length(units) - 1)
  sprintf("%.1f %s", bytes / 1024^idx, units[idx + 1])
}

# Helper: get R version string
r_version_string <- function() {
  paste0("R ", R.Version()$major, ".", R.Version()$minor)
}
