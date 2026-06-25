# StreamFLOW — headless R compute engine
# =============================================================================
# Replaces the Shiny server. Runs as a persistent child process spawned by the
# JavaFX app and speaks newline-delimited JSON over stdin/stdout.
#
#   Java  -> R (stdin) : {"id":N,"cmd":"...","args":{...}}
#   R -> Java (stdout) : {"type":"progress","id":N,"frac":..,"msg":".."}
#                        {"type":"result","id":N, ...}
#                        {"type":"error","id":N,"message":"..","trace":[..]}
#                        {"type":"ready"}              (emitted once at startup)
#
# Design rules (see plan: "Biggest risks"):
#   * stdout is reserved EXCLUSIVELY for JSON. All stray output (cat/print from
#     Bioconductor internals, warnings, messages) is sunk to stderr so it can
#     never corrupt the protocol stream.
#   * The single persistent global STATE env holds the GatingSet/flowSet across
#     calls — the engine is the one stateful R session for the app's lifetime.
#   * Orphan protection: a blocking read on stdin returns EOF the moment the
#     Java parent dies (even on a hard JVM segfault the OS closes the pipe), so
#     the engine self-terminates and never lingers as a zombie.
#   * Cooperative cancellation: long jobs poll a per-job flag file under the
#     control dir; Java drops the flag to request a stop. (Windows has no SIGINT.)
# =============================================================================

# --- stdout hijack: do this FIRST, before anything can print -----------------
# `sink(stderr(), type = "output")` redirects every print()/cat()/auto-print to
# stderr. `json_out` keeps a private handle to the REAL stdout for JSON only.
json_out <- stdout()
sink(stderr(), type = "output")
options(warn = 1)                  # surface warnings immediately, on stderr
Sys.setenv(R_CLI_NUM_COLORS = "1") # discourage ANSI colour codes in any output

suppressWarnings(suppressPackageStartupMessages({
  ok_jsonlite <- requireNamespace("jsonlite", quietly = TRUE)
}))
if (!ok_jsonlite) {
  # Without jsonlite we cannot speak the protocol at all. Emit a hand-rolled
  # error object (the only place we ever build JSON by hand) and exit.
  writeLines('{"type":"fatal","message":"jsonlite is not installed in the R library"}', json_out)
  flush(json_out)
  quit(save = "no", status = 1)
}

# --- argument / environment parsing ------------------------------------------
args <- commandArgs(trailingOnly = TRUE)
arg_value <- function(flag, default = NULL) {
  i <- which(args == flag)
  if (length(i) && i[1] < length(args)) args[i[1] + 1L] else default
}
CONTROL_DIR <- arg_value("--control-dir", tempfile("sfctl_"))
if (!dir.exists(CONTROL_DIR)) dir.create(CONTROL_DIR, recursive = TRUE, showWarnings = FALSE)

# --- global persistent state -------------------------------------------------
# The engine equivalent of Shiny's `shared` reactiveValues. Plain environment so
# the existing pure helpers (save_workspace/load_workspace) can be reused as-is.
STATE <- new.env(parent = emptyenv())
STATE$status <- "idle"

`%||%` <- function(a, b) if (!is.null(a) && length(a) > 0 && !all(is.na(a))) a else b

# --- protocol writers (the ONLY functions that touch json_out) ---------------
send <- function(obj) {
  line <- jsonlite::toJSON(obj, auto_unbox = TRUE, null = "null", na = "null", digits = NA)
  writeLines(line, json_out)
  flush(json_out)
}
send_ready    <- function() send(list(type = "ready", pid = Sys.getpid(), r = R.version.string))
send_pong     <- function(id) send(list(type = "pong", id = id))
send_result   <- function(id, payload = NULL) send(c(list(type = "result", id = id), payload %||% list()))
send_progress <- function(id, frac, msg = NULL) send(list(type = "progress", id = id, frac = frac, msg = msg))
send_error    <- function(id, message, trace = NULL)
  send(list(type = "error", id = id, message = message, trace = trace %||% list()))

# --- cooperative cancellation ------------------------------------------------
cancel_flag_path <- function(id) file.path(CONTROL_DIR, paste0("cancel_", id, ".flag"))
is_cancelled <- function(id) file.exists(cancel_flag_path(id))
clear_cancel <- function(id) suppressWarnings(unlink(cancel_flag_path(id)))
# Raised by a long job when it observes its cancel flag; caught by the dispatcher
# and reported as a clean cancellation rather than an error. Inherits "error" so
# stop() unwinds it normally and tryCatch routes it to the cancel handler (which
# is listed before the generic error handler).
cancelledCondition <- function(id)
  structure(class = c("streamflow_cancelled", "error", "condition"),
            list(message = sprintf("Job %s cancelled", id), call = NULL))

# =============================================================================
# Command handlers. Each takes (id, args) and returns a payload list (sent as a
# `result`) or calls send_* itself. Phase 0 ships the protocol/lifecycle probes;
# later phases register the real flow-cytometry commands here.
# =============================================================================
COMMANDS <- new.env(parent = emptyenv())

COMMANDS$ping <- function(id, args) {
  send_pong(id)
  invisible(NULL)            # pong already sent; no result frame
}

COMMANDS$echo <- function(id, args) {
  list(echo = args)
}

COMMANDS$r_version <- function(id, args) {
  list(version = R.version.string,
       major = R.version$major, minor = R.version$minor,
       platform = R.version$platform)
}

# Simulated long, cancellable, progress-emitting job — used by the Phase 0
# conformance test to prove the async UI never freezes and that cooperative
# cancel works. `steps` x `step_ms` controls duration.
COMMANDS$sleep <- function(id, args) {
  steps   <- as.integer(args$steps %||% 10L)
  step_ms <- as.numeric(args$step_ms %||% 200)
  for (i in seq_len(steps)) {
    if (is_cancelled(id)) stop(cancelledCondition(id))
    Sys.sleep(step_ms / 1000)
    send_progress(id, i / steps, sprintf("step %d/%d", i, steps))
  }
  list(slept_ms = steps * step_ms)
}

# Deliberately raise, to verify error replies carry a real message + traceback.
COMMANDS$boom <- function(id, args) {
  stop("intentional failure for protocol test")
}

# Deliberately print/cat to stdout, to prove the sink() keeps JSON uncorrupted.
COMMANDS$noisy <- function(id, args) {
  cat("this cat() must NOT reach the JSON stream\n")
  print("this print() must NOT reach the JSON stream either")
  message("this message() goes to stderr (fine)")
  list(noisy = TRUE)
}

# --- load feature command modules --------------------------------------------
# Each file in engine/commands/ registers handlers into COMMANDS. They are
# sourced into the global env so they can see COMMANDS/STATE/send_*/%||%.
# Modules that need heavy packages (flowCore etc.) load them lazily inside their
# handlers, so the engine still starts (and Phase 0 probes work) without them.
engine_dir <- {
  fa <- grep("^--file=", commandArgs(trailingOnly = FALSE), value = TRUE)
  if (length(fa)) dirname(normalizePath(sub("^--file=", "", fa[1]))) else getwd()
}
cmd_dir <- file.path(engine_dir, "commands")
if (dir.exists(cmd_dir)) {
  for (f in list.files(cmd_dir, pattern = "\\.R$", full.names = TRUE)) {
    tryCatch(source(f, local = FALSE),
             error = function(e) message(sprintf("[engine] failed to load %s: %s",
                                                  basename(f), conditionMessage(e))))
  }
}

# --- dispatch ----------------------------------------------------------------
capture_trace <- function() {
  tb <- tryCatch(utils::limitedLabels(sys.calls()), error = function(e) character(0))
  if (length(tb) == 0) tb <- as.character(geterrmessage())
  utils::head(rev(tb), 20L)
}

dispatch <- function(req) {
  id  <- req$id %||% NA
  cmd <- req$cmd
  if (is.null(cmd) || !nzchar(cmd)) { send_error(id, "Request has no 'cmd'"); return() }
  handler <- COMMANDS[[cmd]]
  if (is.null(handler)) { send_error(id, sprintf("Unknown command: %s", cmd)); return() }

  STATE$status <- "busy"
  on.exit({ STATE$status <- "idle"; clear_cancel(id) }, add = TRUE)

  withCallingHandlers(
    tryCatch({
      payload <- handler(id, req$args)
      if (!is.null(payload)) send_result(id, payload)
    },
    streamflow_cancelled = function(c) {
      send(list(type = "cancelled", id = id))
    },
    error = function(e) {
      send_error(id, conditionMessage(e), capture_trace())
    }),
    warning = function(w) {
      message(sprintf("[engine warn] %s", conditionMessage(w)))
      invokeRestart("muffleWarning")
    }
  )
  invisible(NULL)
}

# --- main read-eval loop -----------------------------------------------------
# Blocking line reads. readLines() returning length 0 means EOF: the Java parent
# has closed the pipe (clean exit OR crash) — self-terminate immediately.
con <- file("stdin", open = "rt", blocking = TRUE)
on.exit(close(con), add = TRUE)

send_ready()

repeat {
  line <- tryCatch(readLines(con, n = 1L, warn = FALSE), error = function(e) character(0))
  if (length(line) == 0L) break          # EOF -> parent gone -> exit
  line <- trimws(line)
  if (!nzchar(line)) next
  if (identical(line, "__shutdown__")) break

  req <- tryCatch(jsonlite::fromJSON(line, simplifyVector = TRUE, simplifyDataFrame = FALSE),
                  error = function(e) NULL)
  if (is.null(req)) { send_error(NA, "Malformed JSON request"); next }
  dispatch(req)
}

quit(save = "no", status = 0)
