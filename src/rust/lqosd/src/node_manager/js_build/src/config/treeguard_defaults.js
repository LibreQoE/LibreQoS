export function defaultTreeguardConfig() {
    return {
        enabled: true,
        dry_run: false,
        tick_seconds: 1,
        cpu: {
            mode: "traffic_rtt_only",
            cpu_high_pct: 75,
            cpu_low_pct: 55,
        },
        links: {
            enabled: true,
            all_nodes: true,
            nodes: [],
            idle_util_pct: 2.0,
            idle_min_minutes: 15,
            rtt_missing_seconds: 120,
            unvirtualize_util_pct: 5.0,
            min_state_dwell_minutes: 30,
            max_link_changes_per_hour: 4,
            reload_cooldown_minutes: 10,
            top_level_auto_virtualize: true,
            top_level_safe_util_pct: 85.0,
        },
        circuits: {
            enabled: true,
            all_circuits: true,
            circuits: [],
            switching_enabled: true,
            independent_directions: true,
            idle_util_pct: 2.0,
            idle_min_minutes: 15,
            rtt_missing_seconds: 120,
            upgrade_util_pct: 5.0,
            min_switch_dwell_minutes: 30,
            max_switches_per_hour: 4,
            persist_sqm_overrides: true,
        },
        qoo: {
            enabled: true,
            min_score: 80.0,
        },
    };
}

export function ensureTreeguardConfig(config) {
    const defaults = defaultTreeguardConfig();
    if (!config || typeof config !== "object") {
        return defaults;
    }

    const current = config.treeguard || {};
    return {
        ...defaults,
        ...current,
        cpu: {
            ...defaults.cpu,
            ...(current.cpu || {}),
        },
        links: {
            ...defaults.links,
            ...(current.links || {}),
        },
        circuits: {
            ...defaults.circuits,
            ...(current.circuits || {}),
        },
        qoo: {
            ...defaults.qoo,
            ...(current.qoo || {}),
        },
    };
}
