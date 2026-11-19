// Pure helpers for the read-only DPDK Dataplane status panel.
//
// The panel renders values returned by the node-manager `DpdkStatus` websocket
// request. Normalizing here keeps the DOM code thin and lets the node test
// suite pin the honest-empty / kind-label contract without a browser.

import {t} from "../i18n/index.js";

/**
 * Human label for a status kind. The cases mirror `DpdkStatusKind` on the Rust
 * side; anything unrecognized is reported as unknown instead of echoed.
 * @param {unknown} kind Raw kind from the payload.
 * @returns {string} The localized provenance label.
 */
export function dpdkKindLabel(kind) {
    switch (kind) {
        case "running":
            return t("config.tuning.dpdk.kind.running");
        case "effective":
            return t("config.tuning.dpdk.kind.effective");
        case "configured":
            return t("config.tuning.dpdk.kind.configured");
        default:
            return t("config.tuning.dpdk.kind.unknown");
    }
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
        return {ok: false, value: null, error: t("config.tuning.validation.dpdk_workers_whole")};
    }
    const value = Number.parseInt(trimmed, 10);
    if (value < 1) {
        return {ok: false, value: null, error: t("config.tuning.validation.dpdk_workers_minimum")};
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
