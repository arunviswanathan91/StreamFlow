package org.streamflow.ui;

import javafx.fxml.FXML;
import javafx.scene.Group;
import javafx.scene.PerspectiveCamera;
import javafx.scene.SceneAntialiasing;
import javafx.scene.SubScene;
import javafx.scene.control.Button;
import javafx.scene.control.ComboBox;
import javafx.scene.control.Label;
import javafx.scene.layout.StackPane;
import javafx.scene.paint.Color;
import javafx.scene.paint.PhongMaterial;
import javafx.scene.shape.Box;
import javafx.scene.transform.Rotate;
import javafx.scene.transform.Translate;

import com.fasterxml.jackson.databind.ObjectMapper;
import com.fasterxml.jackson.databind.node.ObjectNode;

import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;

/**
 * 3-D scatter (differentiator #12). Renders three channels of a sample's cached events as an
 * orbitable point cloud via a JavaFX {@link SubScene} + {@link PerspectiveCamera}. Points are
 * subsampled to 50k and coloured by Z (depth) using the shared pseudocolour ramp.
 */
public class Scatter3DController implements ContextAware {

    private static final ObjectMapper JSON = new ObjectMapper();
    private static final int CAP = 50000;
    private static final double S = 150;       // half-extent of the normalised cube

    @FXML private ComboBox<String> sampleCombo, xCombo, yCombo, zCombo;
    @FXML private Button refreshButton, renderButton;
    @FXML private Label statusLabel;
    @FXML private StackPane host;

    private AppContext ctx;
    private final Rotate rotX = new Rotate(-20, Rotate.X_AXIS);
    private final Rotate rotY = new Rotate(-30, Rotate.Y_AXIS);
    private final Translate camZ = new Translate(0, 0, -900);
    private double anchorX, anchorY, anchorRX, anchorRY;

    @FXML
    public void initialize() {
        sampleCombo.getSelectionModel().selectedItemProperty().addListener(
                (o, a, b) -> { if (b != null) ensureData(b, () -> populateChannels(b)); });
        setDisabled(true);
    }

    @Override
    public void init(AppContext context) {
        this.ctx = context;
        setDisabled(false);
        refresh();
    }

    @FXML private void onRefresh() { refresh(); }

    private void refresh() {
        if (ctx == null) return;
        // List every loaded sample; events are fetched on demand (no need to pre-open a graph window).
        List<String> all = new ArrayList<>(ctx.workspace().sampleNames());
        sampleCombo.getItems().setAll(all);
        if (!all.isEmpty()) sampleCombo.getSelectionModel().selectFirst();
        statusLabel.setText(all.isEmpty()
                ? "Load FCS first (File ▸ Load FCS…)."
                : all.size() + " sample(s). Pick X/Y/Z channels and click Render.");
    }

    /** Ensure the sample's events are cached (fetch from the engine if needed), then run {@code onReady}. */
    private void ensureData(String sample, Runnable onReady) {
        if (ctx == null || sample == null) return;
        EventData cached = ctx.workspace().data(sample);
        if (cached != null && cached.rows() > 0) { onReady.run(); return; }
        statusLabel.setText("Loading " + sample + "…");
        ObjectNode args = JSON.createObjectNode();
        args.put("sample", sample);
        ctx.jobs().run(ctx.bridge().command("get_events", args), r -> {
            try {
                List<String> chans = new ArrayList<>();
                r.path("channels").forEach(n -> chans.add(n.asText()));
                Path bin = Paths.get(r.path("file").asText());
                EventData d = EventData.read(bin, chans, r.path("rows").asInt(), r.path("cols").asInt());
                try { Files.deleteIfExists(bin); } catch (Exception ignored) {}
                ctx.workspace().putData(sample, d);
                onReady.run();
            } catch (Exception ex) {
                statusLabel.setText("Could not load events: " + ex.getMessage());
            }
        });
    }

    private void populateChannels(String sample) {
        EventData d = ctx.workspace().data(sample);
        if (d == null) return;
        List<String> ch = d.channels();
        xCombo.getItems().setAll(ch); yCombo.getItems().setAll(ch); zCombo.getItems().setAll(ch);
        select(xCombo, ch, "FSC-A", 0);
        select(yCombo, ch, "SSC-A", Math.min(1, ch.size() - 1));
        select(zCombo, ch, null, Math.min(2, ch.size() - 1));
    }

    private void select(ComboBox<String> combo, List<String> ch, String pref, int fallback) {
        if (pref != null) for (String c : ch) if (c.equalsIgnoreCase(pref)) { combo.getSelectionModel().select(c); return; }
        if (fallback >= 0 && fallback < ch.size()) combo.getSelectionModel().select(fallback);
    }

    @FXML
    private void onRender() {
        if (ctx == null || sampleCombo.getValue() == null) return;
        EventData d = ctx.workspace().data(sampleCombo.getValue());
        if (d == null) { ensureData(sampleCombo.getValue(), this::onRender); return; }
        int xc = d.indexOf(xCombo.getValue()), yc = d.indexOf(yCombo.getValue()), zc = d.indexOf(zCombo.getValue());
        if (xc < 0 || yc < 0 || zc < 0) { statusLabel.setText("Pick three channels."); return; }

        double[] xr = d.range(xc), yr = d.range(yc), zr = d.range(zc);
        int n = d.rows();
        int[] idx;
        if (n <= CAP) { idx = new int[n]; for (int i = 0; i < n; i++) idx[i] = i; }
        else { idx = new int[CAP]; Random rng = new Random(7); for (int i = 0; i < CAP; i++) idx[i] = rng.nextInt(n); }

        // shared materials, one per Z-depth colour bin (keeps node setup cheap)
        int bins = 24;
        PhongMaterial[] mats = new PhongMaterial[bins];
        for (int b = 0; b < bins; b++) {
            mats[b] = new PhongMaterial(rampColor((double) b / (bins - 1)));
        }

        Group cloud = new Group();
        List<javafx.scene.Node> nodes = new ArrayList<>(idx.length);
        for (int k : idx) {
            double nx = norm(d.get(k, xc), xr) * 2 * S - S;
            double ny = -(norm(d.get(k, yc), yr) * 2 * S - S);   // flip Y for screen-up
            double nz = norm(d.get(k, zc), zr) * 2 * S - S;
            Box pt = new Box(2.2, 2.2, 2.2);
            pt.setMaterial(mats[(int) Math.min(bins - 1, norm(d.get(k, zc), zr) * (bins - 1))]);
            pt.setTranslateX(nx); pt.setTranslateY(ny); pt.setTranslateZ(nz);
            nodes.add(pt);
        }
        cloud.getChildren().addAll(nodes);
        cloud.getChildren().add(axes());
        cloud.getTransforms().addAll(rotX, rotY);

        Group world = new Group(cloud);
        SubScene sub = new SubScene(world, host.getWidth() > 0 ? host.getWidth() : 800,
                host.getHeight() > 0 ? host.getHeight() : 600, true, SceneAntialiasing.BALANCED);
        sub.setFill(Color.web("#0D1B2A"));
        PerspectiveCamera cam = new PerspectiveCamera(true);
        cam.setNearClip(0.1); cam.setFarClip(5000);
        cam.getTransforms().add(camZ);
        sub.setCamera(cam);
        sub.widthProperty().bind(host.widthProperty());
        sub.heightProperty().bind(host.heightProperty());

        sub.setOnMousePressed(e -> { anchorX = e.getSceneX(); anchorY = e.getSceneY(); anchorRX = rotX.getAngle(); anchorRY = rotY.getAngle(); });
        sub.setOnMouseDragged(e -> {
            rotY.setAngle(anchorRY + (e.getSceneX() - anchorX) * 0.3);
            rotX.setAngle(anchorRX - (e.getSceneY() - anchorY) * 0.3);
        });
        sub.setOnScroll(e -> camZ.setZ(Math.max(-4000, Math.min(-200, camZ.getZ() + e.getDeltaY()))));

        host.getChildren().setAll(sub);
        statusLabel.setText(String.format("%s — %,d of %,d events. Drag to rotate, scroll to zoom.",
                sampleCombo.getValue(), idx.length, n));
        if (ctx != null) ctx.auditLog().add(AuditLog.Type.ANALYSIS, sampleCombo.getValue(),
                String.format("3D scatter %s/%s/%s", xCombo.getValue(), yCombo.getValue(), zCombo.getValue()));
    }

    private Group axes() {
        Group g = new Group();
        g.getChildren().addAll(
                axisBox(2 * S, 1, 1, Color.web("#E74C3C"), -S, S, -S),   // X (red)
                axisBox(1, 2 * S, 1, Color.web("#2ECC71"), -S, S - 2 * S, -S), // Y (green)
                axisBox(1, 1, 2 * S, Color.web("#3498DB"), -S, S, -S));   // Z (blue)
        return g;
    }
    private Box axisBox(double w, double h, double d, Color c, double tx, double ty, double tz) {
        Box b = new Box(w, h, d);
        b.setMaterial(new PhongMaterial(c));
        b.setTranslateX(tx + w / 2); b.setTranslateY(ty - h / 2); b.setTranslateZ(tz + d / 2);
        return b;
    }

    private static double norm(double v, double[] r) {
        double lo = r[0], hi = r[1];
        if (hi <= lo) return 0.5;
        return Math.max(0, Math.min(1, (v - lo) / (hi - lo)));
    }

    /** Blue→cyan→green→yellow→red ramp matching the 2-D pseudocolour feel. */
    private static Color rampColor(double t) {
        Color[] stops = {Color.web("#0000CC"), Color.web("#00CCFF"), Color.web("#00CC44"),
                Color.web("#FFFF00"), Color.web("#FF0000")};
        double x = t * (stops.length - 1);
        int s = (int) Math.floor(x);
        return stops[s].interpolate(stops[Math.min(s + 1, stops.length - 1)], x - s);
    }

    private void setDisabled(boolean d) {
        sampleCombo.setDisable(d); xCombo.setDisable(d); yCombo.setDisable(d); zCombo.setDisable(d);
        refreshButton.setDisable(d); renderButton.setDisable(d);
    }
}
