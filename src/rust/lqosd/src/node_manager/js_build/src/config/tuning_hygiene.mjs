// Pure helpers for the optional `[tuning] worker_capacity_mbps` editor.
// No DOM dependency, so the node-manager test suite can pin create/clear/migrate
// behaviour without a browser. The bridge reads the same key from lqos.conf at
// startup; blank means the compiled 1,125 Mbps effective-capacity default.

import {t} from "../i18n/index.js";

/** Parse a form value. Blank -> null (delete/default); otherwise a positive integer. */
export function parseWorkerCapacityInput(value) {
    const raw = String(value ?? "").trim();
    if (raw === "") {
        return {ok: true, value: null};
    }
    const error = t("config.tuning.validation.worker_capacity");
    if (!/^\d+$/.test(raw)) {
        return {ok: false, error};
    }
    const parsed = Number(raw);
    if (!Number.isSafeInteger(parsed) || parsed <= 0) {
        return {ok: false, error};
    }
    return {ok: true, value: parsed};
}

/** Read the optional key from a whole config object (legacy alias migrates). */
export function readWorkerCapacityMbps(config) {
    const tuning = config && typeof config.tuning === "object" && config.tuning
        ? config.tuning
        : {};
    for (const key of ["worker_capacity_mbps", "placement_budget_mbps"]) {
        const value = tuning[key];
        if (Number.isSafeInteger(value) && value > 0) {
            return value;
        }
    }
    return null;
}

/**
 * Carry the whole `[tuning]` object and set/delete ONLY the canonical key.
 * Also removes the deprecated `placement_budget_mbps` alias so a UI save
 * completes the one-release migration. Never leaves a blank/0 key behind.
 */
export function applyWorkerCapacityMbps(config, value) {
    if (!config || typeof config !== "object") {
        return config;
    }
    const tuning = config.tuning && typeof config.tuning === "object"
        ? config.tuning
        : {};
    delete tuning.placement_budget_mbps;
    if (value === null || value === undefined) {
        delete tuning.worker_capacity_mbps;
    } else {
        tuning.worker_capacity_mbps = value;
    }
    config.tuning = tuning;
    return config;
}
