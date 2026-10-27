// Pure helpers for the read-only DPDK Dataplane status panel.
//
// The panel renders values returned by the node-manager `DpdkStatus` websocket
// request. Normalizing here keeps the DOM code thin and lets the node test
// suite pin the honest-empty / kind-label contract without a browser.

/** Provenance class labels (must mirror `DpdkStatusKind` on the Rust side). */
export const DPDK_KIND_LABELS = Object.freeze({
    running: "Running",
    effective: "Effective",
    configured: "Configured",
});

/**
 * Human label for a status kind.
 * @param {unknown} kind Raw kind from the payload.
 * @returns {string} A label; "Unknown" for anything unrecognized.
 */
export function dpdkKindLabel(kind) {
    return DPDK_KIND_LABELS[kind] || "Unknown";
}

/**
 * Validate the editable DPDK worker count.
 * @param {unknown} raw Raw input value.
 * @returns {{ok: boolean, value: number|null, error?: string}} Blank becomes
 *   `null` (leave the installer-derived default in place).
 */
export function normalizeWorkerCount(raw) {
    const trimmed = String(raw ?? "").trim();
    if (trimmed === "") {
        return {ok: true, value: null};
    }
    if (!/^\d+$/.test(trimmed)) {
        return {ok: false, value: null, error: "Worker count must be a whole number of 1 or more."};
    }
    const value = Number.parseInt(trimmed, 10);
    if (value < 1) {
        return {ok: false, value: null, error: "Worker count must be 1 or more."};
    }
    return {ok: true, value};
}

/**
 * Defensively normalize a `DpdkStatus` payload into renderable rows.
 * @param {unknown} payload The `data` from `DpdkStatusResult`.
 * @returns {Array<{id: string, label: string, value: string|null, kind: string, source: string, note: string|null}>}
 */
export function normalizeDpdkStatus(payload) {
    const fields = Array.isArray(payload?.fields) ? payload.fields : [];
    return fields.map((field) => ({
        id: String(field?.id ?? ""),
        label: String(field?.label ?? ""),
        value: typeof field?.value === "string" ? field.value : null,
        kind: dpdkKindLabel(field?.kind),
        source: String(field?.source ?? ""),
        note: typeof field?.note === "string" ? field.note : null,
    }));
}
