package org.streamflow.ui;

import javafx.application.Platform;
import javafx.collections.FXCollections;
import javafx.collections.ObservableList;

import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;

/**
 * Session-wide append-only audit log. Every gating or analysis operation
 * records a timestamped {@link Entry}. The list is observable so the
 * {@link AnalysisLogController} can bind to it directly.
 *
 * <p>Thread-safe: {@link #add} can be called from any thread.
 */
public final class AuditLog {

    private static final DateTimeFormatter FMT =
            DateTimeFormatter.ofPattern("HH:mm:ss");

    public enum Type { GATE, ANALYSIS, EXPORT, TRANSFORM, COMPENSATION, OTHER }

    public record Entry(String time, Type type, String sample, String detail) {}

    private final ObservableList<Entry> entries =
            FXCollections.observableArrayList();

    /** Append an entry. Safe to call from any thread. */
    public void add(Type type, String sample, String detail) {
        Entry e = new Entry(LocalDateTime.now().format(FMT), type,
                            sample == null ? "" : sample,
                            detail == null ? "" : detail);
        if (Platform.isFxApplicationThread()) {
            entries.add(e);
        } else {
            Platform.runLater(() -> entries.add(e));
        }
    }

    /** Observable list — bind directly to a TableView. */
    public ObservableList<Entry> entries() { return entries; }

    public void clear() {
        if (Platform.isFxApplicationThread()) entries.clear();
        else Platform.runLater(entries::clear);
    }

    // ---- workspace persistence (#32b) ---------------------------------------

    /** Serialise the log to a JSON array of {time,type,sample,detail} for the .sfw. */
    public com.fasterxml.jackson.databind.node.ArrayNode toJson(com.fasterxml.jackson.databind.ObjectMapper mapper) {
        com.fasterxml.jackson.databind.node.ArrayNode arr = mapper.createArrayNode();
        for (Entry e : entries) {
            com.fasterxml.jackson.databind.node.ObjectNode n = arr.addObject();
            n.put("time", e.time());
            n.put("type", e.type().name());
            n.put("sample", e.sample());
            n.put("detail", e.detail());
        }
        return arr;
    }

    /** Replace the log with entries restored from a loaded .sfw "audit_log" node (preserving timestamps). */
    public void restore(com.fasterxml.jackson.databind.JsonNode arr) {
        Runnable apply = () -> {
            entries.clear();
            if (arr != null && arr.isArray()) {
                for (com.fasterxml.jackson.databind.JsonNode n : arr) {
                    Type t;
                    try { t = Type.valueOf(n.path("type").asText("OTHER")); }
                    catch (IllegalArgumentException ex) { t = Type.OTHER; }
                    entries.add(new Entry(n.path("time").asText(""), t,
                            n.path("sample").asText(""), n.path("detail").asText("")));
                }
            }
        };
        if (Platform.isFxApplicationThread()) apply.run(); else Platform.runLater(apply);
    }

    /**
     * Generate a journal-style methods paragraph from the logged operations.
     * Aggregates gate + analysis events into prose.
     */
    public String generateMethodsText() {
        if (entries.isEmpty()) return "No operations have been logged yet.";

        long gateCount     = entries.stream().filter(e -> e.type() == Type.GATE).count();
        long analysisCount = entries.stream().filter(e -> e.type() == Type.ANALYSIS).count();
        long transformCount= entries.stream().filter(e -> e.type() == Type.TRANSFORM).count();
        long compCount     = entries.stream().filter(e -> e.type() == Type.COMPENSATION).count();

        StringBuilder sb = new StringBuilder();
        sb.append("Flow cytometry data were acquired and analysed using StreamFLOW. ");

        if (compCount > 0) {
            sb.append("Compensation was applied using a spillover matrix derived from ")
              .append("single-stain controls embedded in the FCS files. ");
        }
        if (transformCount > 0) {
            sb.append("Channel data were transformed (logicle or arcsinh) to linearise ")
              .append("the fluorescence scale. ");
        }
        if (gateCount > 0) {
            sb.append(String.format(
                "Sequential manual gates (%d total) were applied to identify populations ", gateCount));
            // collect gate details
            entries.stream()
                   .filter(e -> e.type() == Type.GATE && !e.detail().isBlank())
                   .map(Entry::detail)
                   .distinct()
                   .limit(8)
                   .forEach(d -> sb.append("(").append(d).append(") "));
            sb.append(". ");
        }
        if (analysisCount > 0) {
            sb.append(String.format(
                "Statistical and functional analyses (%d module runs) were performed ", analysisCount));
            entries.stream()
                   .filter(e -> e.type() == Type.ANALYSIS && !e.detail().isBlank())
                   .map(Entry::detail)
                   .distinct()
                   .limit(5)
                   .forEach(d -> sb.append("including ").append(d).append("; "));
            sb.append("using built-in StreamFLOW modules. ");
        }
        sb.append("Data are presented as percentages of the parent gate unless otherwise stated.");
        return sb.toString().replaceAll(";\\s*\\.", ".").replaceAll("\\s{2,}", " ").trim();
    }
}
