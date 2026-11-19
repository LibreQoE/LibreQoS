// Display names for the country picker on the Insight trial page.
//
// `lts_trial.js` carries 237 ISO 3166-1 alpha-2 codes from the payment
// provider's allow-list; the label of each option is the engine's CLDR region
// name for the active locale (`Intl.DisplayNames`), not a catalog entry. The
// catalogs would duplicate data the engine already ships, and 84 of the 237
// English names are spelled exactly like the Spanish one, so a catalog copy
// would need an `IDENTICAL_BY_DESIGN` row per name.
//
// The wording is CLDR data, which `i18n/GLOSSARY.md` §4 already treats as an
// acceptable source of visible text for numbers, dates and relative times.

import {intlTag} from "./i18n/intl.js";
import {locale} from "./i18n/locale.mjs";

const REGION_CODE = /^[A-Z]{2}$/;
// One formatter per tag: `Intl.DisplayNames` costs more to build than the lookups
// a single render makes, and the picker renders every code once per page load.
const formatters = new Map();

function formatterFor(tag) {
    let formatter = formatters.get(tag);
    if (formatter === undefined) {
        formatter = typeof Intl.DisplayNames === "function" ? new Intl.DisplayNames([tag], {type: "region"}) : null;
        formatters.set(tag, formatter);
    }
    return formatter;
}

/**
 * The region name for an ISO 3166-1 alpha-2 code in the active UI locale.
 *
 * A code the engine has no name for, or a browser old enough to lack
 * `Intl.DisplayNames`, yields the upper-case code: the option still carries the
 * value the form submits.
 * @param {string} code Two-letter region code, any case.
 * @returns {string}
 */
export function regionName(code) {
    const upper = String(code ?? "").trim().toUpperCase();
    if (!REGION_CODE.test(upper)) return upper;
    const name = formatterFor(intlTag(locale()))?.of(upper);
    return typeof name === "string" && name.trim() ? name : upper;
}
