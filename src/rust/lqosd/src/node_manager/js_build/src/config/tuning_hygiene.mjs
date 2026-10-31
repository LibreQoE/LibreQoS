// Pure helpers for the optional `[tuning] placement_budget_mbps` editor.
// Dependency-free so the node-manager test suite can pin create/clear/preserve
// behaviour without a DOM. The bridge reads the same key from lqos.conf at
// startup; blank means the compiled 2.0G (pre-headroom) default.

/** Parse a form value. Blank -> null (delete); otherwise a positive integer. */
export function parsePlacementBudgetInput(value) {
    const raw = String(value ?? "").trim();
    if (raw === "") {
        return {ok: true, value: null};
    }
    if (!/^\d+$/.test(raw)) {
        return {ok: false, error: "Placement budget must be a positive integer number of Mbps"};
    }
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed) || parsed <= 0) {
        return {ok: false, error: "Placement budget must be a positive integer number of Mbps"};
    }
    return {ok: true, value: parsed};
}

/** Read the optional key from a whole config object. */
export function readPlacementBudgetMbps(config) {
    const tuning = config && typeof config.tuning === "object" && config.tuning
        ? config.tuning
        : {};
    const value = tuning.placement_budget_mbps;
    return Number.isSafeInteger(value) && value > 0 ? value : null;
}

/**
 * Carry the whole `[tuning]` object and set/delete ONLY the optional key.
 * Mutates and returns `config`. Never leaves a blank/0 key behind.
 */
export function applyPlacementBudgetMbps(config, value) {
    if (!config || typeof config !== "object") {
        return config;
    }
    const tuning = config.tuning && typeof config.tuning === "object"
        ? config.tuning
        : {};
    if (value === null || value === undefined) {
        delete tuning.placement_budget_mbps;
    } else {
        tuning.placement_budget_mbps = value;
    }
    config.tuning = tuning;
    return config;
}
