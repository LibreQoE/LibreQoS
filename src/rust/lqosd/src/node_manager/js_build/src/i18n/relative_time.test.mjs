// Unit tests for the relative-time ladder. Node built-ins only; the fake
// `window` below is the minimum `i18n/index.js` reads to resolve a locale.
//
// The value expectations are derived from the same `Intl.RelativeTimeFormat`
// call the helper makes, so an ICU release that renames "sec." to "seconds"
// cannot fail the suite without a behaviour change. The only anchored goldens
// are "now" and "ahora": `numeric: "auto"` takes those two words from the
// bundled ICU rather than from the pattern, and there is no value/unit pair to
// derive them from.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {relativeTimeText} from "./relative_time.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

// The helper's options; the expectation below must use the same ones.
const SHORT = {style: "short", numeric: "auto"};

/** The short Intl text for one offset, as the helper would render it. */
function shortText(tag, value, unit) {
    return new Intl.RelativeTimeFormat(tag, SHORT).format(value, unit);
}

// A fixed clock keeps the assertions independent of the host's wall time.
const NOW_MS = 1_800_000_000_000;
const NOW_SECONDS = NOW_MS / 1000;

/** A timestamp `deltaSeconds` before the fixed clock. */
function ago(deltaSeconds) {
    return NOW_SECONDS - deltaSeconds;
}

function useCatalog(catalog) {
    globalThis.window = {LQOS_I18N: catalog};
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang: ""}, cookie: ""};
}

test.afterEach(() => {
    delete globalThis.window;
    delete globalThis.localStorage;
    delete globalThis.document;
});

test("English steps units at the round", () => {
    useCatalog(EN);
    const tag = EN.locale;
    assert.equal(relativeTimeText(ago(0), NOW_MS), "now");
    assert.equal(relativeTimeText(ago(5), NOW_MS), shortText(tag, -5, "second"));
    assert.equal(relativeTimeText(ago(59), NOW_MS), shortText(tag, -59, "second"));
    assert.equal(relativeTimeText(ago(60), NOW_MS), shortText(tag, -1, "minute"));
    assert.equal(relativeTimeText(ago(90), NOW_MS), shortText(tag, -2, "minute"));
    assert.equal(relativeTimeText(ago(3 * 3600), NOW_MS), shortText(tag, -3, "hour"));
    assert.equal(relativeTimeText(ago(3 * 86400), NOW_MS), shortText(tag, -3, "day"));
});

test("Spanish steps units at the round", () => {
    useCatalog(ES);
    const tag = ES.locale;
    assert.equal(relativeTimeText(ago(0), NOW_MS), "ahora");
    assert.equal(relativeTimeText(ago(5), NOW_MS), shortText(tag, -5, "second"));
    assert.equal(relativeTimeText(ago(90), NOW_MS), shortText(tag, -2, "minute"));
    assert.equal(relativeTimeText(ago(3 * 3600), NOW_MS), shortText(tag, -3, "hour"));
    assert.equal(relativeTimeText(ago(3 * 86400), NOW_MS), shortText(tag, -3, "day"));
});

test("a missing, zero or unparseable timestamp has no text of its own", () => {
    useCatalog(EN);
    for (const value of [undefined, null, 0, -5, Number.NaN, Number.POSITIVE_INFINITY, "not a time"]) {
        assert.equal(relativeTimeText(value, NOW_MS), "", `timestamp ${String(value)}`);
    }
});

test("a timestamp ahead of the clock reads as now instead of a future phrase", () => {
    useCatalog(EN);
    assert.equal(relativeTimeText(NOW_SECONDS + 30, NOW_MS), "now");
    useCatalog(ES);
    assert.equal(relativeTimeText(NOW_SECONDS + 30, NOW_MS), "ahora");
});
