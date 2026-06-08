# StreamFLOW - app.R
# Entry point: sources global, ui, server and launches the app

source("global.R")
source("ui.R")
source("server.R")

shinyApp(ui = ui, server = server)
