// `.64.4` (R1.2): the pure, import-free Executive heatmap LABEL builders.
//
// These live in their own module (re-exported by `executive_utils.js`) so the
// fit oracle and the `node --test` unit tests can import the SHIPPED builders
// without pulling in the WebSocket runtime: `executive_utils.js` uses
// extensionless relative imports that Node's ESM resolver cannot load, so a
// test cannot import it directly. No imports here on purpose — keep it that way.

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
    return `<span class="exec-type-mono exec-type-mono--${mono.slug}" title="${key}">${mono.letter}</span>`;
}

export const ROW_KIND_CAPTION = {
    Site: "sites",
    Circuit: "circuits",
    Asn: "ASNs",
};

/**
 * `.64` (v2): the panel caption DERIVED from the ACTUAL rendered rows' kinds
 * (the same predicate that selects them) — NOT hardcoded. `hideAsns` is dead
 * config and `top_rtt` is server-ranked across all kinds, so e.g. the Median
 * RTT panel can legitimately contain ASN rows. Returns "" when no kind is known.
 */
export function heatRowsCaption(rows) {
    const labels = [];
    for (const kind of ["Site", "Circuit", "Asn"]) {
        if (rows.some((row) => String(row?.entity_kind || "") === kind)) {
            labels.push(ROW_KIND_CAPTION[kind]);
        }
    }
    if (!labels.length) {
        return "";
    }
    if (labels.length === 1) {
        return labels[0];
    }
    return `${labels.slice(0, -1).join(", ")} & ${labels[labels.length - 1]}`;
}

/**
 * `.64.4` (R1.3): the SINGLE source for a heat row's `aria-label` value. The
 * badge WORD is preserved even when the visible monogram is dropped (an unknown
 * badge renders no chip), so an ASN row can never masquerade as a circuit. The
 * row builders wrap this in `aria-label="…"`; `beforeLatest` injects the metric
 * word (e.g. "RTT") while keeping the exact legacy strings.
 */
export function heatRowAriaLabel(label, badge, latestLabel, beforeLatest = "") {
    const metric = beforeLatest ? `${beforeLatest} ` : "";
    return `${label} ${badge || ""} latest ${metric}${latestLabel}`;
}
