// The shipped catalogs, for the modules whose labels come from them.
//
// The i18n runtime resolves a message from the served catalog, then the English
// one, then falls back to the KEY itself. Tests use all three facts:
//   * installing the English catalog keeps a module's established wording under
//     assertion,
//   * installing the Spanish catalog proves the label follows the page,
//   * installing an EMPTY catalog turns every render into its catalog key, which
//     is how a test proves a server code maps to a key that both locales define
//     without keeping a second copy of the mapping.
//
// A page loads the catalogs as generated classic scripts; these helpers install
// the same globals on `globalThis.window`.

import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {FALLBACK_LOCALE} from "../src/i18n/locale.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../static2/i18n");

export const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
export const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

/**
 * Installs the catalogs the way a page's generated scripts do.
 * @param {object|null} [served] The active locale's catalog; null loads English only.
 */
export function installCatalogs(served = null) {
    globalThis.window = served
        ? {LQOS_I18N_EN: EN, LQOS_I18N: served}
        : {LQOS_I18N_EN: EN};
}

/** Installs catalogs that define nothing, so a render returns its catalog key. */
export function installKeyEchoCatalogs() {
    globalThis.window = {LQOS_I18N_EN: {locale: FALLBACK_LOCALE, dir: "ltr", messages: {}}};
}

/**
 * Installs an English catalog that defines only `keys`. The named messages keep
 * their placeholders interpolated while every nested lookup still renders its
 * key, which is how a test observes a label that reaches the screen as a
 * parameter of another message.
 * @param {string[]} keys Catalog keys to define.
 */
export function installTemplatesOnly(keys) {
    const messages = {};
    for (const key of keys) {
        if (EN.messages[key] !== undefined) messages[key] = EN.messages[key];
    }
    globalThis.window = {LQOS_I18N_EN: {locale: FALLBACK_LOCALE, dir: "ltr", messages}};
}

/** Removes the catalogs, so the next test starts from "the page loaded none". */
export function clearCatalogs() {
    delete globalThis.window;
}
