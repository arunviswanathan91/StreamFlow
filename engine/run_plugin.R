#!/usr/bin/env Rscript
# StreamFLOW generic R-plugin runner.
#
#   Rscript run_plugin.R <plugin_dir> <request.json> <response.json>
#
# Contract (kept deliberately small so plugins stay easy to write and to vet):
#   * <plugin_dir>/plugin.json  declares  entry.script  and  entry.function
#   * the entry function is called as  fn(request)  where `request` is the parsed
#     request.json list, and must return a plain list that is JSON-serialisable.
#   * anything the function writes to stdout is captured as the plugin log; only the
#     JSON written to <response.json> is consumed by the app.
#
# On any error we still write a response with ok=FALSE + the message, so the Java side
# always gets structured output instead of having to parse a stack trace.

suppressWarnings(suppressMessages({
  ok_jsonlite <- requireNamespace("jsonlite", quietly = TRUE)
}))
if (!ok_jsonlite) {
  stop("The 'jsonlite' package is required to run StreamFLOW plugins.")
}

args <- commandArgs(trailingOnly = TRUE)
if (length(args) < 3) {
  stop("usage: run_plugin.R <plugin_dir> <request.json> <response.json>")
}
plugin_dir <- args[[1]]
request_path <- args[[2]]
response_path <- args[[3]]

write_response <- function(obj) {
  json <- jsonlite::toJSON(obj, auto_unbox = TRUE, null = "null", digits = NA)
  writeLines(json, con = response_path, useBytes = TRUE)
}

result <- tryCatch({
  manifest_path <- file.path(plugin_dir, "plugin.json")
  if (!file.exists(manifest_path)) stop("plugin.json not found in ", plugin_dir)
  manifest <- jsonlite::fromJSON(manifest_path, simplifyVector = FALSE)

  entry <- manifest$entry
  if (is.null(entry$script) || is.null(entry$`function`)) {
    stop("plugin.json must declare entry.script and entry.function")
  }

  script_path <- file.path(plugin_dir, entry$script)
  if (!file.exists(script_path)) stop("entry script not found: ", script_path)
  source(script_path, local = FALSE)

  fn_name <- entry$`function`
  if (!exists(fn_name, mode = "function")) {
    stop("entry function '", fn_name, "' not defined by ", entry$script)
  }
  fn <- get(fn_name, mode = "function")

  request <- jsonlite::fromJSON(request_path, simplifyVector = FALSE)
  out <- fn(request)
  if (!is.list(out)) stop("plugin entry function must return a list")

  c(list(ok = TRUE), out)
}, error = function(e) {
  list(ok = FALSE, error = conditionMessage(e))
})

write_response(result)
