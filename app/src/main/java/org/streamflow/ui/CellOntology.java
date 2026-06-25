package org.streamflow.ui;

import java.util.LinkedHashMap;
import java.util.Locale;
import java.util.Map;

/**
 * Bundled immunology cell-type ontology (differentiator #20). Fuzzy-matches a gate name to a
 * canonical population and returns its defining markers + a typical frequency note, surfaced as a
 * tooltip on gating-tree nodes. A compact, curated subset (EuroFlow / OMIP conventions).
 */
public final class CellOntology {

    public record Entry(String canonical, String markers, String typical) {}

    private static final Map<String, Entry> BY_KEY = new LinkedHashMap<>();

    private static void add(String key, String canonical, String markers, String typical) {
        BY_KEY.put(key, new Entry(canonical, markers, typical));
    }

    static {
        add("lymph",      "Lymphocytes",        "low SSC, CD45+",                     "~20–40% of leukocytes");
        add("tcell",      "T cells",            "CD3+",                               "~60–80% of lymphocytes");
        add("cd4",        "CD4+ T helper",      "CD3+ CD4+ CD8−",                     "~25–60% of T cells");
        add("cd8",        "CD8+ cytotoxic T",   "CD3+ CD8+ CD4−",                     "~15–40% of T cells");
        add("treg",       "Regulatory T cells", "CD4+ CD25+ CD127low FoxP3+",         "~3–8% of CD4+ T cells");
        add("naive",      "Naïve T cells",      "CCR7+ CD45RA+",                      "~30–60% of T cells");
        add("memory",     "Memory T cells",     "CD45RO+ CCR7±",                      "~40–70% of T cells");
        add("bcell",      "B cells",            "CD19+ CD20+ CD3−",                   "~5–20% of lymphocytes");
        add("plasma",     "Plasmablasts",       "CD19+ CD27hi CD38hi",                "~0.1–2% of B cells");
        add("nk",         "NK cells",           "CD3− CD56+ (CD16±)",                 "~5–20% of lymphocytes");
        add("nkt",        "NKT cells",          "CD3+ CD56+",                         "~0.1–3% of lymphocytes");
        add("mono",       "Monocytes",          "CD14+ CD33+ HLA-DR+",                "~2–10% of leukocytes");
        add("classical",  "Classical monocytes","CD14++ CD16−",                       "~80–90% of monocytes");
        add("dc",         "Dendritic cells",    "HLA-DR+ lineage−",                   "~0.5–2% of leukocytes");
        add("neut",       "Neutrophils",        "CD15+ CD16+ high SSC",               "~40–70% of leukocytes");
        add("eos",        "Eosinophils",        "CD15+ CD16− SSC-high",               "~1–5% of leukocytes");
        add("baso",       "Basophils",          "CD123+ HLA-DR−",                     "~0.5–1% of leukocytes");
        add("live",       "Live cells",         "viability-dye−",                     "assay dependent");
        add("single",     "Single cells",       "FSC-H vs FSC-A diagonal",            "~85–98% of events");
        add("blast",      "Blasts",             "CD34+ CD45dim",                      "<5% (normal marrow)");
        add("stem",       "Hematopoietic stem", "CD34+ CD38− CD90+",                  "<0.1% of marrow");
    }

    /** Best fuzzy match for a gate name, or null. Matches on substring / token containment. */
    public static Entry match(String gateName) {
        if (gateName == null || gateName.isBlank()) return null;
        String g = gateName.toLowerCase(Locale.ROOT).replaceAll("[^a-z0-9]", "");
        for (Map.Entry<String, Entry> e : BY_KEY.entrySet()) {
            if (g.contains(e.getKey())) return e.getValue();
        }
        // a few common alternate spellings
        if (g.contains("helper")) return BY_KEY.get("cd4");
        if (g.contains("cytotox")) return BY_KEY.get("cd8");
        if (g.contains("singlet")) return BY_KEY.get("single");
        if (g.contains("viable")) return BY_KEY.get("live");
        return null;
    }

    private CellOntology() {}
}
