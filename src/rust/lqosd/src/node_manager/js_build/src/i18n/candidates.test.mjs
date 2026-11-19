// Tests for the candidate scanner behind `--inventory`: the §1.3 heuristic in
// both directions (text is kept, plumbing is not) and the `translatable: false`
// marker that argues a candidate out.

import assert from "node:assert/strict";
import test from "node:test";
import {applyMarkers, scanCode, scanMarkup} from "../../i18n/candidates.mjs";

function texts(scan) {
    return scan.candidates.map((candidate) => candidate.text);
}

test("HTML: text nodes and the accessible attributes are candidates", () => {
    const scan = scanMarkup(
        [
            "<h2>Shaped Devices</h2>",
            '<input title="Search by name" aria-label="Search" placeholder="e.g. router" alt="x">',
            "<p>Two words</p>",
        ].join("\n"),
    );
    // Document order: the heading's text node, then the input's attributes.
    // `alt="x"` is one letter and carries no text; the placeholder acronym is.
    assert.deepEqual(texts(scan), ["Shaped Devices", "Search by name", "Search", "e.g. router", "Two words"]);
});

test("HTML: text an element already keys through data-i18n is the inline fallback, not a candidate", () => {
    const scan = scanMarkup(
        [
            '<h2 data-i18n="page.title">Shaped Devices</h2>',
            '<input data-i18n-placeholder="page.search" placeholder="Search devices">',
            '<p data-i18n-aria-label="page.help" aria-label="Help with devices">Bodies stay.</p>',
        ].join("\n"),
    );
    assert.deepEqual(texts(scan), ["Bodies stay."]);
});

test("HTML: markup, comments, scripts and styles are not text", () => {
    const scan = scanMarkup(
        [
            "<!-- a comment with words -->",
            "<script>const t = \"Script text\";</script>",
            "<style>.a { content: \"Style text\"; }</style>",
            "<div><span class='badge bg-success'></span></div>",
        ].join("\n"),
    );
    assert.deepEqual(texts(scan), []);
});

test("HTML: a template token is not text", () => {
    assert.deepEqual(texts(scanMarkup("<title>%%TITLE%%</title>")), []);
});

test("JS: template text survives interpolation and markup", () => {
    const scan = scanCode("el.innerHTML = `<span class=\"badge\">Updated ${minutes}s ago</span>`;\n");
    assert.deepEqual(texts(scan), ["Updated s ago"]);
});

test("JS: keys, paths, selectors, class lists and identifiers are not candidates", () => {
    const scan = scanCode(
        [
            'const key = t("common.nav.tree");',
            'const file = "i18n/es.json";',
            'const id = "#txtSearch";',
            'const cls = "btn btn-primary";',
            'const handler = "onMessage";',
            'const option = "2-digit";',
            'const unit = "Mbps";',
            'const label = "Download";',
        ].join("\n"),
    );
    assert.deepEqual(texts(scan), ["Download"]);
});

test("JS: a literal inside a comment or a regex is not a candidate", () => {
    const scan = scanCode(['// "Commented out label"', 'const quotes = /["\']/g;', 'const label = "Live";'].join("\n"));
    assert.deepEqual(texts(scan), ["Live"]);
});

test("JS: the same literal twice counts once per file at the caller", () => {
    const scan = scanCode('a.textContent = "Error"; b.textContent = "Error";\n');
    assert.deepEqual(texts(scan), ["Error", "Error"]);
    assert.equal(new Set(texts(scan)).size, 1);
});

test("a translatable: false marker takes its literal out and keeps the rest", () => {
    const scan = scanCode(['// translatable: false — "Mbps" is a unit', 'const a = "Mbps";', 'const b = "Live";'].join("\n"));
    const result = applyMarkers(scan);
    assert.deepEqual(result.candidates.map((candidate) => candidate.text), ["Live"]);
    assert.deepEqual(result.findings, []);
});

test("a marker without a reason is a finding", () => {
    const scan = scanCode(['// translatable: false — "Mbps"', 'const a = "Mbps";'].join("\n"));
    const result = applyMarkers(scan);
    assert.deepEqual(result.findings, ['line 1: "translatable: false" needs a reason ("— <why>")']);
});

test("a marker that names nothing in the file is a finding", () => {
    const scan = scanCode(['// translatable: false — "Gbps" is a unit', 'const a = "Live";'].join("\n"));
    const result = applyMarkers(scan, (literal) => JSON.stringify(literal));
    assert.deepEqual(result.findings, ['line 1: "translatable: false" names "Gbps", which is not a string in this file']);
});

test("an HTML comment can carry the marker too", () => {
    const scan = scanMarkup(['<!-- translatable: false — "libreqos.example" is a host name -->', "<p>libreqos.example</p>"].join("\n"));
    const result = applyMarkers(scan);
    assert.deepEqual(result.candidates, []);
    assert.deepEqual(result.findings, []);
});

test("the classifier keeps a doubtful literal rather than dropping a real one", () => {
    // Both classes are false positives a reviewer can see and mark; a dropped
    // real string would make the denominator lie.
    const scan = scanCode(['const a = "hover focus";', 'const b = "Enter";'].join("\n"));
    assert.deepEqual(texts(scan), ["hover focus", "Enter"]);
});
