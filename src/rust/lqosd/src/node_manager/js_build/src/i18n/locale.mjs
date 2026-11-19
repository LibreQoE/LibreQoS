// The active UI locale and the browser's persisted choice.
//
// `index.js` renders messages; this module answers which locale they render in,
// and owns the two places the answer is stored: localStorage and the `lqos_lang`
// cookie, which the server reads back to pick the catalog it serves. The served
// catalog is the first answer, because the server has already resolved the tag
// and written it into the page.

// The one fallback constant: the catalog every page loads and the language used
// when nothing else resolves. `locales.json` must list it (the catalog build
// checks that), and no other module spells a locale tag itself — the hardcoding
// guard in `locale_hardcoding.test.mjs` enforces both directions.
export const FALLBACK_LOCALE = "en";
// translatable: false — "= ;path=/;max-age= ;SameSite=Lax" is the cookie attribute frame the browser parses, not copy
const STORAGE_KEY = "uiLanguage";
const COOKIE_NAME = "lqos_lang";
const COOKIE_MAX_AGE_SECONDS = 31536000;

function browserWindow() {
    return typeof window === "undefined" ? null : window;
}

function browserDocument() {
    return typeof document === "undefined" ? null : document;
}

/** One global a generated catalog wrote, or null when the page did not load it. */
function catalogGlobal(name) {
    const value = browserWindow()?.[name];
    return value && typeof value === "object" ? value : null;
}

/**
 * The catalog the page loaded, or null when it loaded none.
 *
 * The fallback locale's script publishes `LQOS_I18N_EN` and every other catalog
 * `LQOS_I18N`, so the active catalog is named by the build instead of being
 * whichever catalog script ran last.
 * @returns {object|null}
 */
export function servedCatalog() {
    return catalogGlobal("LQOS_I18N") ?? catalogGlobal("LQOS_I18N_EN");
}

/** The English catalog every page loads, or null when the page loaded none. */
export function fallbackCatalog() {
    return catalogGlobal("LQOS_I18N_EN");
}

/** The locale the browser stored, or "" when storage is empty or blocked. */
export function storedLocale() {
    try {
        const value = globalThis.localStorage?.getItem(STORAGE_KEY);
        return typeof value === "string" ? value.trim() : "";
    } catch {
        return "";
    }
}

/**
 * Active UI locale: the served catalog's locale, then the stored browser
 * choice, then the page's `<html lang>`, then English.
 * @returns {string}
 */
export function locale() {
    const served = servedCatalog()?.locale;
    if (typeof served === "string" && served.trim()) return served.trim();
    const stored = storedLocale();
    if (stored) return stored;
    const documentLang = browserDocument()?.documentElement?.lang;
    if (typeof documentLang === "string" && documentLang.trim()) return documentLang.trim();
    return FALLBACK_LOCALE;
}

/**
 * Persists a locale choice for the browser (localStorage) and the server
 * (cookie). Callers reload; this never does.
 * @param {string} tag Locale tag such as "es".
 */
export function setLocale(tag) {
    const value = String(tag ?? "").trim();
    if (!value) return;
    try {
        globalThis.localStorage?.setItem(STORAGE_KEY, value);
    } catch {
        // Storage can be blocked; the cookie still carries the choice to the server.
    }
    const doc = browserDocument();
    if (doc) {
        doc.cookie = `${COOKIE_NAME}=${encodeURIComponent(value)};path=/;max-age=${COOKIE_MAX_AGE_SECONDS};SameSite=Lax`;
    }
}
