// The shared markup renderer the config pages use for `<code>`/`<strong>` prose.
//
// A keyed element cannot carry the markup: `data-i18n` writes text. These tests
// pin the two properties that made `tHtml` the right source — the catalog's
// allowlisted tags survive, and every value it interpolates is escaped — plus
// the skip that lets one template serve every page.

import assert from "node:assert/strict";
import test from "node:test";

import {tHtml} from "../i18n/index.js";
import {clearCatalogs, installCatalogs} from "../../i18n/test_catalogs.mjs";
import {renderMarkupText} from "./markup_text.mjs";

const CATALOG = {
    locale: "en",
    dir: "ltr",
    messages: {
        "test.markup": "Read <code>{path}</code> and keep <strong>this</strong>",
    },
};

function fakeDocument(elements) {
    return {
        getElementById(id) {
            return elements[id] ?? null;
        },
    };
}

test.afterEach(() => {
    clearCatalogs();
});

test("writes a catalog message with allowlisted markup as markup", () => {
    installCatalogs(CATALOG);
    const elements = {note: {innerHTML: ""}};
    renderMarkupText({note: tHtml("test.markup", {path: "lqos.conf"})}, fakeDocument(elements));
    assert.equal(elements.note.innerHTML, "Read <code>lqos.conf</code> and keep <strong>this</strong>");
});

test("escapes the values interpolated into that markup", () => {
    installCatalogs(CATALOG);
    const elements = {note: {innerHTML: ""}};
    renderMarkupText({note: tHtml("test.markup", {path: '<b>&"x"</b>'})}, fakeDocument(elements));
    assert.equal(
        elements.note.innerHTML,
        "Read <code>&lt;b&gt;&amp;&quot;x&quot;&lt;/b&gt;</code> and keep <strong>this</strong>",
    );
});

test("skips an element the page does not render and writes the rest", () => {
    installCatalogs(CATALOG);
    const elements = {present: {innerHTML: ""}};
    renderMarkupText(
        {absent: tHtml("test.markup", {path: "a"}), present: tHtml("test.markup", {path: "b"})},
        fakeDocument(elements),
    );
    assert.equal(elements.present.innerHTML, "Read <code>b</code> and keep <strong>this</strong>");
});
