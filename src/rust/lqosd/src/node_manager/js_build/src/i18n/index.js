// i18n runtime for the node_manager UI.
//
// Catalogs are generated classic scripts (`js_build/out/i18n/<tag>.js`, built
// from `static2/i18n/<tag>.json`). The fallback locale's script publishes
// `window.LQOS_I18N_EN`; a page served another locale also loads that locale's
// script, which publishes `window.LQOS_I18N`. `locale.mjs` resolves the active
// catalog from those two names, so a page that never loads the served catalog
// renders English instead of taking the locale from the last script that ran.
//
// This module renders messages; `format.js` owns every Intl formatter it uses,
// so a number formatted inside a message and the same number formatted by a page
// helper agree. The only function here that touches the DOM is
// `applyDomTranslations()`; the active locale and the browser's stored choice
// live in `locale.mjs`.
//
// Every entry point names one rendering context, and a message may only be
// handed to the one that matches where its output lands:
//   - `t()`      plain text (`textContent`, `innerText`, a template's text run);
//                escapes nothing, so its output must not reach innerHTML.
//   - `tHtml()`  markup handed to `innerHTML`; allows `<strong>/<em>/<code>/<a>`
//                and escapes everything else, parameters included.
//   - `tAttr()`  a quoted HTML attribute value; escapes the whole rendered
//                message, so a translation carrying `"` cannot close the tag.

import {escapeAttr, escapeHtml} from "./escape.mjs";
import {formatNumber, pluralRules} from "./format.js";
import {fallbackCatalog, locale, servedCatalog} from "./locale.mjs";

// [data attribute, attribute written] — the attribute-less entry sets the text.
const DOM_KEY_ATTRIBUTES = [
    ["data-i18n", null],
    ["data-i18n-title", "title"],
    ["data-i18n-placeholder", "placeholder"],
    ["data-i18n-aria-label", "aria-label"],
    ["data-i18n-alt", "alt"],
];

// The only inline markup a catalog message may reach `innerHTML` with; every
// other tag in a message is escaped by tHtml().
const ALLOWED_MARKUP = /<\/?(?:strong|em|code)>|<\/a>|<a href="[^"]*">/g;
const PLACEHOLDER = /\{([A-Za-z0-9_]+)\}/g;

function messagesOf(catalog) {
    const messages = catalog?.messages;
    return messages && typeof messages === "object" ? messages : {};
}

function own(messages, key) {
    return Object.prototype.hasOwnProperty.call(messages, key);
}

/**
 * The message for `key`: active locale first, then the English fallback.
 * @returns {string|object|undefined} A string, a plural-category object, or undefined.
 */
function rawMessage(key) {
    const primary = messagesOf(servedCatalog());
    if (own(primary, key)) return primary[key];
    const fallback = messagesOf(fallbackCatalog());
    return own(fallback, key) ? fallback[key] : undefined;
}

/** The key text, with a warning only when the page asked for i18n debugging. */
function unavailable(key) {
    if (globalThis.window?.LQOS_I18N_DEBUG) {
        console.warn(`[i18n] no string message for "${key}" in ${locale()}; rendering the key`);
    }
    return key;
}

function formatValue(value) {
    if (typeof value === "number" && Number.isFinite(value)) {
        return formatNumber(value);
    }
    return String(value);
}

function escapeValue(value) {
    return escapeHtml(formatValue(value));
}

/**
 * Substitutes `{name}` placeholders; a name the caller did not supply, or
 * supplied as null/undefined, stays visible instead of rendering "undefined".
 * `renderText` handles the literal runs between placeholders, so a run without
 * parameters is still escaped when the caller needs HTML.
 */
function interpolate(template, params, renderValue, renderText) {
    let rendered = "";
    let cursor = 0;
    for (const match of template.matchAll(PLACEHOLDER)) {
        rendered += renderText(template.slice(cursor, match.index));
        const name = match[1];
        const value = params !== undefined && params !== null && Object.prototype.hasOwnProperty.call(params, name)
            ? params[name]
            : undefined;
        rendered += value === undefined || value === null ? renderText(match[0]) : renderValue(value);
        cursor = match.index + match[0].length;
    }
    return rendered + renderText(template.slice(cursor));
}

/** Applies the tag allowlist to a message, escaping every text run and parameter. */
function renderMarkup(template, params) {
    let rendered = "";
    let cursor = 0;
    for (const match of template.matchAll(ALLOWED_MARKUP)) {
        rendered += interpolate(template.slice(cursor, match.index), params, escapeValue, escapeHtml);
        // The allowlisted tag itself stays literal; only its placeholders are escaped.
        rendered += interpolate(match[0], params, escapeValue, (text) => text);
        cursor = match.index + match[0].length;
    }
    return rendered + interpolate(template.slice(cursor), params, escapeValue, escapeHtml);
}

/**
 * Message text with `{name}` placeholders filled in.
 * @param {string} key Catalog key.
 * @param {Record<string, string|number>} [params] Placeholder values; numbers are locale-formatted.
 * @returns {string} The rendered text, or the key when it cannot be resolved.
 */
export function t(key, params) {
    const message = rawMessage(key);
    if (typeof message !== "string") return unavailable(key);
    return interpolate(message, params, formatValue, (text) => text);
}

/**
 * Plural-aware message text; the CLDR category comes from `Intl.PluralRules`.
 * @param {string} key Catalog key of a plural object.
 * @param {number} count Value that selects the category; exposed to the message as `{count}`.
 * @param {Record<string, string|number>} [params] Additional placeholder values.
 * @returns {string} The rendered text, or the key when it cannot be resolved.
 */
export function tn(key, count, params) {
    const message = rawMessage(key);
    if (!message || typeof message !== "object") return unavailable(key);
    const numeric = Number(count);
    const category = pluralRules().select(numeric);
    const template = message[category] ?? message.other;
    if (typeof template !== "string") return unavailable(key);
    return interpolate(template, {...params, count: numeric}, formatValue, (text) => text);
}

/**
 * Message text with allowlisted inline markup, safe for `innerHTML`.
 * Parameters are escaped, so no parameter value can introduce a tag.
 *
 * Catalogs are build-time inputs, so the allowlist protects against a hostile
 * parameter, not against a catalog: a message's own `{url}` must come from a
 * trusted source, because `ALLOWED_MARKUP` accepts any `href` scheme.
 * @param {string} key Catalog key.
 * @param {Record<string, string|number>} [params] Placeholder values.
 * @returns {string} HTML with only `<strong>`, `<em>`, `<code>` and `<a href>` markup.
 */
export function tHtml(key, params) {
    const message = rawMessage(key);
    if (typeof message !== "string") return escapeValue(unavailable(key));
    return renderMarkup(message, params);
}

/**
 * Message text escaped for a double-quoted HTML attribute value.
 * @param {string} key Catalog key.
 * @param {Record<string, string|number>} [params] Placeholder values.
 * @returns {string} `t()` escaped, safe inside `attr="…"`.
 */
export function tAttr(key, params) {
    return escapeAttr(t(key, params));
}

/**
 * Translates the elements under `root` that carry a `data-i18n*` attribute.
 * Keys missing from the catalog keep the English text already in the page.
 *
 * A `data-i18n` element that has element children (an icon inside a button)
 * keeps them: its own non-whitespace text nodes are collapsed into the first one
 * and replaced with the message, so authored markup never has to move into a
 * catalog. An element whose text lives only in child elements is left alone.
 * @param {ParentNode} [root] Defaults to `document`.
 * @returns {number} How many elements were translated.
 */
export function applyDomTranslations(root = globalThis.document ?? null) {
    if (!root || typeof root.querySelectorAll !== "function") return 0;
    let applied = 0;
    for (const [dataAttribute, attribute] of DOM_KEY_ATTRIBUTES) {
        for (const element of root.querySelectorAll(`[${dataAttribute}]`)) {
            const key = element.getAttribute(dataAttribute);
            // Missing key or a plural-only message: keep the English already in the page.
            if (!key || typeof rawMessage(key) !== "string") continue;
            const text = t(key);
            if (attribute === null) {
                if (hasElementChildren(element)) {
                    if (!translateOwnText(element, text)) continue;
                } else {
                    element.textContent = text;
                }
            } else {
                element.setAttribute(attribute, text);
            }
            applied += 1;
        }
    }
    return applied;
}

/** True when the node owns at least one child element. */
function hasElementChildren(element) {
    return element.children !== undefined && element.children !== null && element.children.length > 0;
}

/**
 * Replaces the element's own text with `text`, keeping its child elements. The
 * leading and trailing whitespace of the replaced node is preserved: it is the
 * spacing between an icon and its label, not part of the message.
 * @returns {boolean} False when the element has no direct text node to replace.
 */
function translateOwnText(element, text) {
    let target = null;
    for (const node of element.childNodes ?? []) {
        // Text nodes are nodeType 3; whitespace nodes are the page's indentation.
        if (node.nodeType !== 3 || !node.data.trim()) continue;
        if (target === null) {
            target = node;
            const leading = node.data.match(/^\s*/)[0];
            const trailing = node.data.match(/\s*$/)[0];
            node.data = `${leading}${text}${trailing}`;
        } else {
            node.data = "";
        }
    }
    return target !== null;
}
