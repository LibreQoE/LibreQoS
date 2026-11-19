// HTML escapers for every string a template concatenates into `innerHTML`.
//
// They live outside `template.js`, which is a bundle entrypoint: importing it
// runs the shell's DOM initialization, so the escaping could not be unit-tested
// in place. Both escapers are safe for text and for double-quoted attribute
// values, so a value carrying `"` cannot close the attribute it sits in.

/**
 * Escapes the five HTML-significant characters for text and quoted attributes.
 * @param {string|number|null|undefined} text Value to escape.
 * @returns {string} "" for null/undefined, otherwise the escaped text.
 */
export function escapeAttr(text) {
    if (text === undefined || text === null) return "";
    return String(text)
        .replaceAll('&', '&amp;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&apos;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;');
}

/**
 * HTML text context. Same escapes as {@link escapeAttr}; the second name keeps
 * the call site readable about which context the value lands in.
 * @param {string|number|null|undefined} text Value to escape.
 * @returns {string}
 */
export function escapeHtml(text) {
    return escapeAttr(text);
}
