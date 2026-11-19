// The one place that writes translated markup into a keyed element.
//
// A paragraph that wraps a literal file name, command, example or scheme in
// `<code>`/`<strong>` cannot be a `data-i18n` element: the DOM applier writes
// text, so the markup would be lost, and splitting the sentence into fragments
// would lose the sentence. The config pages therefore render those paragraphs
// from `tHtml` at module init. The DOM mechanics live here so the null-check and
// the `innerHTML` write exist once; the KEYS stay literal at each call site,
// because `extract.mjs` credits a catalog key only when it is a literal argument
// of `t(`/`tn(`/`tHtml(`.

/**
 * Writes each rendered message into the element with the matching id.
 *
 * Missing elements are skipped rather than fatal: the template ships every
 * panel, but a page only renders the paragraphs of the section it shows.
 * @param {Record<string, string>} messages Element id -> rendered markup.
 * @param {Document} [root] Document to search; defaults to the page's `document`.
 * @returns {void}
 */
export function renderMarkupText(messages, root = globalThis.document) {
    if (!root || typeof root.getElementById !== "function") return;
    for (const [id, markup] of Object.entries(messages)) {
        const element = root.getElementById(id);
        if (element) element.innerHTML = markup;
    }
}
