// The language selector every page shares: one `<option>` per locale listed in
// the generated `i18n/locales.js`, with the active locale selected. Imported by
// the shell's `template.js` and by the two standalone page bundles, and
// unit-tested by `switcher.test.mjs`, so the control's behaviour lives in one
// tested place rather than in each page's init block.

import {locale, setLocale} from "./locale.mjs";

/**
 * The locales `i18n/locales.js` listed.
 * @param {object} [scope] The global the generated script assigned to.
 * @returns {Array<{tag: string, name: string, dir: string}>} Empty when the page
 *   did not load the generated list.
 */
export function listedLocales(scope = globalThis) {
    const listed = scope?.LQOS_I18N_LOCALES;
    return Array.isArray(listed) ? listed : [];
}

/**
 * Builds the selector's options. A list with fewer than two entries has nothing
 * to switch between, so the whole control is removed instead of leaving an
 * inert select behind. The page ships the wrapper hidden, so a browser that
 * never runs this module shows neither an empty select nor a focus stop; the
 * options are what make it worth revealing.
 *
 * The page also has to ship `#languageSelect`: the options are appended to that
 * select, and it carries the change listener. A page with the wrapper but no
 * select gets no control, no throw, and no diagnostic — the wrapper is left as
 * it shipped.
 * @param {object} [options]
 * @param {Document} [options.document] The document to build into.
 * @param {object} [options.scope] The global carrying `LQOS_I18N_LOCALES`.
 * @param {string} [options.active] The active locale tag; a tag the list does
 *   not carry falls back to the first listed locale.
 * @param {string} [options.containerId] The wrapper the control sits in. It
 *   ships hidden, is revealed once there are options, and is removed when there
 *   are none. The sidebar item by default; the standalone pages ship their own
 *   wrapper. It only matters when `#languageSelect` exists.
 * @param {(tag: string) => void} [options.onSelect] Persists the chosen tag.
 * @param {() => void} [options.reload] Runs after a selection that changed the locale.
 * @returns {number} The number of options rendered; 0 when the page ships no
 *   select or lists fewer than two locales.
 */
export function initLanguageSwitcher({
    document: doc = globalThis.document,
    scope = globalThis,
    active = locale(),
    containerId = "languageNav",
    onSelect = setLocale,
    reload = () => globalThis.location?.reload(),
} = {}) {
    if (!doc) return 0;
    const select = doc.getElementById("languageSelect");
    if (!select) return 0;
    const locales = listedLocales(scope);
    const container = doc.getElementById(containerId);
    if (locales.length < 2) {
        container?.remove();
        return 0;
    }
    // A browser shows the first option when none is marked selected, and
    // re-picking what is already on screen fires no change event; marking the
    // first listed locale for an active tag the list does not carry keeps the
    // displayed value the one a change acts on.
    const activeIndex = locales.findIndex((entry) => entry.tag === active);
    const chosenIndex = activeIndex < 0 ? 0 : activeIndex;
    locales.forEach((entry, index) => {
        const option = doc.createElement("option");
        option.value = entry.tag;
        // Each option reads in its own language's words; the list carries them.
        option.textContent = entry.name;
        if (index === chosenIndex) option.selected = true;
        select.appendChild(option);
    });
    // The wrapper ships hidden so a page without the bundle stays complete; it
    // is revealed only now that it has something to choose.
    if (container) container.hidden = false;
    // A change event only fires for an actual change, so the chosen value is
    // the new locale without a comparison against `active`.
    select.addEventListener("change", () => {
        onSelect(select.value);
        reload();
    });
    return locales.length;
}
