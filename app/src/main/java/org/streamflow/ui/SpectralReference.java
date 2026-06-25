package org.streamflow.ui;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * Bundled fluorochrome emission reference (differentiator #19). Matches detector/channel names to
 * known fluorochromes and flags pairs whose emission peaks are close enough that spillover is
 * likely — a panel-design sanity check absent from FlowJo.
 */
public final class SpectralReference {

    /** emission-peak nm by fluorochrome (compact common-reagent subset). */
    private static final Map<String, Integer> EMISSION = new LinkedHashMap<>();
    private static void add(String name, int em) { EMISSION.put(name.toLowerCase(Locale.ROOT), em); }
    static {
        add("FITC", 519);  add("AlexaFluor488", 519); add("AF488", 519); add("CFSE", 517); add("GFP", 509);
        add("PE", 578);    add("PE-CF594", 615);      add("PE-Dazzle594", 615);
        add("PerCP", 677); add("PerCP-Cy5.5", 695);   add("PE-Cy5", 667);
        add("PE-Cy7", 780);
        add("APC", 660);   add("AlexaFluor647", 668); add("AF647", 668);
        add("AlexaFluor700", 723); add("AF700", 723);
        add("APC-Cy7", 780); add("APC-H7", 780); add("APC-Fire750", 787);
        add("BV421", 421); add("PacificBlue", 455); add("BV510", 510); add("BV570", 570);
        add("BV605", 605); add("BV650", 650); add("BV711", 711); add("BV785", 785);
        add("BUV395", 395); add("BUV737", 737);
        add("PI", 617); add("7-AAD", 647); add("DAPI", 461);
    }

    public record Conflict(String chanA, String chanB, String fluorA, String fluorB, int emA, int emB) {}

    /** Match a channel/alias label to a known fluorochrome name, or null. */
    public static String matchFluor(String channelOrAlias) {
        if (channelOrAlias == null) return null;
        String c = channelOrAlias.toLowerCase(Locale.ROOT).replaceAll("[\\s\\-]", "");
        for (String f : EMISSION.keySet()) {
            String fk = f.replaceAll("[\\s\\-]", "");
            if (c.contains(fk)) return f;
        }
        return null;
    }

    public static Integer emission(String fluor) {
        return fluor == null ? null : EMISSION.get(fluor.toLowerCase(Locale.ROOT));
    }

    /**
     * Flag channel pairs whose matched fluorochromes emit within {@code thresholdNm} of each other.
     * @param channelLabels labels to test (raw channel names and/or aliases).
     */
    public static List<Conflict> conflicts(List<String> channelLabels, int thresholdNm) {
        // resolve each label to a fluorochrome + emission
        List<String> chans = new ArrayList<>();
        List<String> fluors = new ArrayList<>();
        List<Integer> ems = new ArrayList<>();
        for (String c : channelLabels) {
            String f = matchFluor(c);
            Integer em = emission(f);
            if (f != null && em != null) { chans.add(c); fluors.add(f); ems.add(em); }
        }
        List<Conflict> out = new ArrayList<>();
        for (int i = 0; i < ems.size(); i++)
            for (int j = i + 1; j < ems.size(); j++)
                if (Math.abs(ems.get(i) - ems.get(j)) <= thresholdNm)
                    out.add(new Conflict(chans.get(i), chans.get(j), fluors.get(i), fluors.get(j), ems.get(i), ems.get(j)));
        out.sort((a, b) -> Integer.compare(Math.abs(a.emA() - a.emB()), Math.abs(b.emA() - b.emB())));
        return out;
    }

    private SpectralReference() {}
}
