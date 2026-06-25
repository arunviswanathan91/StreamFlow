package org.streamflow.ui;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;
import javafx.application.Platform;
import javafx.fxml.FXML;
import javafx.scene.control.Button;
import javafx.scene.control.TextArea;

import java.time.LocalTime;
import java.time.format.DateTimeFormatter;

/**
 * Developer / engine console — the Phase 0 bridge probes (ping, version, a long
 * cancellable job, error, noisy stdout). Kept as a module for diagnostics.
 */
public class DevConsoleController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final DateTimeFormatter TS = DateTimeFormatter.ofPattern("HH:mm:ss");

    @FXML private TextArea outputArea;
    @FXML private Button pingButton;
    @FXML private Button versionButton;
    @FXML private Button sleepButton;
    @FXML private Button errorButton;
    @FXML private Button noisyButton;

    private AppContext ctx;

    @FXML
    public void initialize() {
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        log("Bridge connected. Start 'Sleep' then switch tabs — the UI stays live.");
    }

    @FXML private void onPing() { run("ping", null, n -> "pong from engine"); }
    @FXML private void onVersion() { run("version", null, n -> n.path("version").asText("?") + " (" + n.path("engine").asText("") + ")"); }
    @FXML private void onSleep() {
        ObjectNode a = JSON.createObjectNode();
        a.put("steps", 20).put("step_ms", 250);
        run("sleep", a, n -> "slept " + n.path("slept_ms").asInt() + " ms");
    }
    @FXML private void onError() { run("boom", null, n -> "(unreachable)"); }
    @FXML private void onNoisy() { run("noisy", null, n -> "JSON stream stayed clean (sink ok)"); }

    private interface Fmt { String f(JsonNode n); }

    private void run(String cmd, JsonNode args, Fmt fmt) {
        if (ctx == null) { log("Engine not ready."); return; }
        log("→ " + cmd);
        ctx.jobs().run(ctx.bridge().command(cmd, args), result -> log(cmd + " → " + fmt.f(result)));
    }

    private void setDisabled(boolean d) {
        for (Button b : new Button[]{pingButton, versionButton, sleepButton, errorButton, noisyButton}) {
            if (b != null) b.setDisable(d);
        }
    }

    private void log(String msg) {
        String line = "[" + LocalTime.now().format(TS) + "] " + msg + "\n";
        if (Platform.isFxApplicationThread()) outputArea.appendText(line);
        else Platform.runLater(() -> outputArea.appendText(line));
    }
}
