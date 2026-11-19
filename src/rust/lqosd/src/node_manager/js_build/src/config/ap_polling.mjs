// Pure helpers for the AP Polling panel on the Data Sources page.
//
// The DOM wiring lives in `config_integration.js`; everything here is pure so
// `node --test` can exercise the labels, badges and validation without a
// browser. The HTML never carries English fallbacks for JS-rendered text: the
// panel reads these keys through `t()`.

/** Catalog keys for the endpoint's explicit unavailable reasons. */
export const AP_POLLING_UNAVAILABLE_KEYS = {
    ap_polling_not_started: "config.integration.ap_polling.unavailable.not_started",
    ap_polling_disabled: "config.integration.ap_polling.unavailable.disabled",
    first_sweep_pending: "config.integration.ap_polling.unavailable.pending",
    ap_polling_stopped: "config.integration.ap_polling.unavailable.stopped",
    request_failed: "config.integration.ap_polling.unavailable.request_failed",
};

const UNKNOWN_UNAVAILABLE_KEY = "config.integration.ap_polling.unavailable.unknown";

/**
 * The catalog key that explains why the table cannot show data.
 * @param {string[]|null|undefined} reasons `unavailable_reasons` from the endpoint.
 * @param {boolean} [missing] True when no document arrived at all (fetch failed).
 * @returns {string}
 */
export function unavailableKey(reasons, missing = false) {
    if (missing) return AP_POLLING_UNAVAILABLE_KEYS.request_failed;
    const first = Array.isArray(reasons) ? reasons[0] : null;
    return AP_POLLING_UNAVAILABLE_KEYS[first] || UNKNOWN_UNAVAILABLE_KEY;
}

/**
 * A document is unavailable when the endpoint says so or none was returned.
 * An explicit unavailable state is rendered, never an empty all-zero table.
 * @param {object|null|undefined} payload
 * @returns {boolean}
 */
export function isUnavailable(payload) {
    return !payload || payload.state === "unavailable";
}

const CAPACITY_SOURCES = ["phy", "per_sm_rate", "crm", "unknown"];

/**
 * Badge data for one `capacity_source` token. The token itself is an API
 * value, shown verbatim; the title is cataloged prose.
 * @param {string|null|undefined} source
 * @returns {{token: string, className: string, titleKey: string}}
 */
export function capacitySourceBadge(source) {
    const token = CAPACITY_SOURCES.includes(source) ? source : "unknown";
    const className = {
        phy: "text-bg-success",
        per_sm_rate: "text-bg-info",
        crm: "text-bg-primary",
        unknown: "text-bg-secondary",
    }[token];
    return {
        token,
        className,
        titleKey: `config.integration.ap_polling.source.${token}`,
    };
}

/**
 * Badge data for one device state. The API's `ok`/`failing`/`observing` are
 * rendered as catalog labels, because the row is operator prose.
 * @param {string|null|undefined} state
 * @returns {{token: string, className: string, labelKey: string}}
 */
export function statusBadge(state) {
    const token = ["ok", "failing", "observing"].includes(state) ? state : "observing";
    const className = {
        ok: "text-bg-success",
        failing: "text-bg-danger",
        observing: "text-bg-secondary",
    }[token];
    return {
        token,
        className,
        labelKey: `config.integration.ap_polling.state.${token}`,
    };
}

/**
 * Signed drift text: `+n` (the AP sees more stations than the CRM pins),
 * `-n`, `0`, or an em dash when unknown. The sign is data, not locale text.
 * @param {number|null|undefined} delta
 * @returns {string}
 */
export function driftText(delta) {
    if (delta === null || delta === undefined) return "—";
    const value = Number(delta);
    if (!Number.isFinite(value)) return "—";
    const rounded = Math.trunc(value);
    if (rounded > 0) return `+${rounded}`;
    return String(rounded);
}

/**
 * The drift's tone for the cell: positive is informational, negative means
 * the CRM pins more circuits than the AP reports.
 * @param {number|null|undefined} delta
 * @returns {string} A Bootstrap text utility class, or "" when unknown/zero.
 */
export function driftClass(delta) {
    if (delta === null || delta === undefined) return "";
    const value = Number(delta);
    if (!Number.isFinite(value) || value === 0) return "";
    return value > 0 ? "text-success" : "text-warning";
}

/**
 * `[ap_polling].interval_s` must be an integer of at least 30 seconds — the
 * same floor the config loader enforces.
 * @param {number|string|null|undefined} value
 * @returns {boolean}
 */
export function intervalValid(value) {
    const interval = Number(value);
    return Number.isInteger(interval) && interval >= 30;
}

/**
 * Fold the editor's `{model, factor}` rows into a `capacity_factor` map.
 *
 * A completely blank row is ignored (it is the "add" affordance's artifact).
 * A row with one half filled is an error rather than a silent drop. Factors
 * must satisfy `0 < factor <= 1`, and a model may appear once.
 * @param {{model: string, factor: string|number}[]} rows
 * @returns {{ok: boolean, factors: Record<string, number>, errorKey: string|null}}
 */
export function validateFactorRows(rows = []) {
    const factors = {};
    for (const row of rows) {
        const model = String(row?.model ?? "").trim();
        const raw = String(row?.factor ?? "").trim();
        if (!model && !raw) continue;
        if (!model) {
            return {ok: false, factors: {}, errorKey: "config.integration.ap_polling.validation.factor_model"};
        }
        if (Object.prototype.hasOwnProperty.call(factors, model)) {
            return {
                ok: false,
                factors: {},
                errorKey: "config.integration.ap_polling.validation.factor_duplicate",
            };
        }
        const factor = Number(raw);
        if (!Number.isFinite(factor) || factor <= 0 || factor > 1) {
            return {
                ok: false,
                factors: {},
                errorKey: "config.integration.ap_polling.validation.factor_range",
            };
        }
        factors[model] = factor;
    }
    return {ok: true, factors, errorKey: null};
}

/**
 * The panel's summary sentence parameters from an aggregate block.
 * @param {object|null|undefined} aggregate
 * @returns {{total: number, ok: number, failing: number, observing: number}}
 */
export function summaryCounts(aggregate) {
    const number = (value) => (Number.isFinite(Number(value)) ? Number(value) : 0);
    return {
        total: number(aggregate?.devices_total),
        ok: number(aggregate?.ok),
        failing: number(aggregate?.failing),
        observing: number(aggregate?.observing),
    };
}
