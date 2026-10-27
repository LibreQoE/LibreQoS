// Pure helpers for the v3 queues-page hygiene rules.
//
// The DPDK pipeline supports exactly one default SQM: `cake diffserv4`. Kernel
// era values migrated from v2 configs (`fq_codel`, the ack-filter CAKE variant)
// are normalized on save and flagged so the page can warn. Keeping this logic
// in a dependency-free module lets the node test suite pin the reconciliation
// without a DOM.

/** The only default SQM the v3 DPDK pipeline supports. */
export const SUPPORTED_DEFAULT_SQM = "cake diffserv4";

/** The compile mode whose queue-sizing knobs are still operator-tunable. */
export const FLAT_COMPILE_MODE = "flat";

/**
 * Whether the flat-mode queue-sizing knobs apply for a config.
 * @param {unknown} config The whole node config (or any object).
 * @returns {boolean} True only when `topology.compile_mode === "flat"`.
 */
export function isFlatCompileMode(config) {
    return config?.topology?.compile_mode === FLAT_COMPILE_MODE;
}

/**
 * Normalize a stored `default_sqm` value to the supported v3 value.
 *
 * @param {unknown} value Raw config value.
 * @returns {{value: string, legacy: boolean, original: string}} The supported
 *   value, whether the input was a non-empty legacy/unknown value, and the
 *   trimmed original.
 */
export function normalizeDefaultSqm(value) {
    const original = typeof value === "string" ? value.trim() : "";
    const legacy = original.length > 0 && original !== SUPPORTED_DEFAULT_SQM;
    return {value: SUPPORTED_DEFAULT_SQM, legacy, original};
}
