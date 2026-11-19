// `.64.4` (R1.2): the pure Executive heatmap LABEL builders.
//
// These live in their own module (re-exported by `executive_utils.js`) so the
// fit oracle and the `node --test` unit tests can import the SHIPPED builders
// without pulling in the WebSocket runtime: `executive_utils.js` uses
// extensionless relative imports that Node's ESM resolver cannot load, so a
// test cannot import it directly. The i18n renderer is the one import the
// module takes: it is Node-safe, so a contract test can still render these
// labels in both locales.

import {t} from "./i18n/index.js";

export const TYPE_MONO = {
    Site: {letter: "S", slug: "site"},
    Circuit: {letter: "C", slug: "circuit"},
    ASN: {letter: "A", slug: "asn"},
};

/**
 * `.64` (v2): the inline type monogram (S/C/A) for a heat row, DERIVED from the
 * badge WORD (the single type source — the same word feeds the row aria-label
 * and `redactClass`). A `title` carries the type for hover; the monogram is NOT
 * `aria-hidden` (the `#12`-required behaviour). Unknown/absent word (e.g.
 * "Global" on the Global Heatmap rows, or an empty kind) falls back to NO
 * monogram — the visible chip is dropped while the aria-label keeps the word.
 */
export function typeMonoMarkup(badge) {
    const key = String(badge || "");
    const mono = TYPE_MONO[key];
    if (!mono) {
        return "";
    }
    return `<span class="exec-type-mono exec-type-mono--${mono.slug}" title="${entityKindLabel(key)}">${mono.letter}</span>`;
}

/**
 * The translated name of an entity-kind badge WORD.
 *
 * The word is also the type IDENTITY — `badgeForEntityKind` emits it,
 * `redactClass` and `TYPE_MONO` compare it, and the caption switches on it — so
 * it stays an English token in the data and only the rendered label translates.
 * A word with no translation (`ASN`, whose acronym reads the same in both
 * locales, or `Global`) passes through unchanged.
 * @param {string} badge Entity-kind badge word.
 * @returns {string}
 */
function entityKindLabel(badge) {
    // translatable: false — "Site" "Circuit" "Asn" — the entity-kind tokens the server emits and every switch in this module compares, never rendered as-is
    switch (String(badge || "")) {
    case "Site":
        return t("topology.node_kind.site");
    case "Circuit":
        return t("common.table.circuit");
    default:
        return String(badge || "");
    }
}

/**
 * The caption word for one rendered entity kind. One literal `t()` call per
 * branch: the catalog extractor reads keys, it does not run code, so a table of
 * keys would read as unused.
 * @param {string} kind Entity kind as the server reports it.
 * @returns {string}
 */
function kindCaption(kind) {
    switch (kind) {
        case "Site": return t("executive.heat.kind.sites");
        case "Circuit": return t("executive.heat.kind.circuits");
        case "Asn": return t("executive.heat.kind.asn");
        default: return "";
    }
}

/**
 * `.64` (v2): the panel caption DERIVED from the ACTUAL rendered rows' kinds
 * (the same predicate that selects them) — NOT hardcoded. `hideAsns` is dead
 * config and `top_rtt` is server-ranked across all kinds, so e.g. the Median
 * RTT panel can legitimately contain ASN rows. Returns "" when no kind is known.
 */
export function heatRowsCaption(rows) {
    const labels = ["Site", "Circuit", "Asn"]
        .filter((kind) => rows.some((row) => String(row?.entity_kind || "") === kind))
        .map(kindCaption);
    if (!labels.length) {
        return "";
    }
    if (labels.length === 1) {
        return labels[0];
    }
    if (labels.length === 2) {
        return t("executive.heat.caption.two", {first: labels[0], second: labels[1]});
    }
    return t("executive.heat.caption.three", {
        first: labels[0],
        second: labels[1],
        third: labels[2],
    });
}

/**
 * `.64.4` (R1.3): the SINGLE source for a heat row's `aria-label` value. The
 * badge WORD is preserved as the type identity even when the visible monogram is
 * dropped (an unknown badge renders no chip), so an ASN row can never masquerade
 * as a circuit; the word and the "latest" frame are rendered through the
 * catalog, and `beforeLatest` injects the metric word (e.g. "RTT").
 */
export function heatRowAriaLabel(label, badge, latestLabel, beforeLatest = "") {
    const metric = beforeLatest ? `${beforeLatest} ` : "";
    return t("executive.heat.aria.row", {
        label,
        kind: entityKindLabel(badge),
        metric,
        value: latestLabel,
    });
}
