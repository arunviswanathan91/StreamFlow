package org.streamflow.ui;

import java.util.HashMap;
import java.util.Map;

/**
 * Session-wide detector-channel → biological-target aliases (e.g. BV711-A → CD4).
 * Shared via {@link AppContext} so every window shows "CD4 (BV711-A)" once set.
 * One panel per session, so a mapping applies to all loaded samples automatically.
 */
public final class ChannelAliases {

    private final Map<String, String> map = new HashMap<>();

    public void set(String channel, String target) {
        if (target == null || target.isBlank()) map.remove(channel);
        else map.put(channel, target.trim());
    }

    public String target(String channel) { return map.get(channel); }

    /** "CD4 (BV711-A)" when aliased, else the raw channel name. */
    public String label(String channel) {
        String t = map.get(channel);
        return (t == null || t.isBlank()) ? channel : t + " (" + channel + ")";
    }

    public Map<String, String> all() { return map; }
}
