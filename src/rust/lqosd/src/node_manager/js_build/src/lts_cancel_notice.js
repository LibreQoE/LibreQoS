// The cancellation disclosure: one catalog sentence, rendered wherever the UI
// promises "cancel any time" and on the licence screen that shows the cap it
// names. The trial page states the terms twice, the post-signup page once, and
// the License & Services page shows the cap, so the sentence is filled from one
// message instead of being authored four times.
//
// The number is a placeholder, not part of the copy: `t()` formats a numeric
// parameter with the active locale, so the same key renders "1,000" in English
// and "1000" in Spanish without the catalogs carrying a grouped value.
import {t} from "./i18n/index.js";

/**
 * The mapped-circuit cap an unlicensed deployment runs under — the cap
 * cancellation restores, and the number the notice quotes.
 *
 * It is deliberately not read from `window.mappedCircuitLimit`: the shell
 * publishes the deployment's *current* cap, a licence grant raises it from
 * `max_circuits`, and quoting a raised cap would say cancellation leaves the
 * operator on it. The engine's constant of record is
 * `lqos_bus::DEFAULT_MAPPED_CIRCUIT_LIMIT`.
 */
export const UNLICENSED_MAPPED_CIRCUIT_LIMIT = 1000;

// The elements that ask for the notice. They ship empty: the text depends on
// the active locale, so mark-up cannot hold it and `data-i18n` cannot render it.
const NOTICE_SELECTOR = "[data-cancel-cap-notice]";

/**
 * Writes the cancellation notice into every element marked
 * `data-cancel-cap-notice`.
 * @param {ParentNode} [root] Subtree to fill; defaults to the document.
 * @returns {number} How many elements were filled.
 */
export function applyCancelCapNotice(root = globalThis.document) {
    if (!root || typeof root.querySelectorAll !== "function") return 0;
    const text = t("trial.cancel_license_cap_notice", {limit: UNLICENSED_MAPPED_CIRCUIT_LIMIT});
    let applied = 0;
    for (const element of root.querySelectorAll(NOTICE_SELECTOR)) {
        element.textContent = text;
        applied += 1;
    }
    return applied;
}
