// The last-seen column's wording: one helper, one unit ladder, both locales.
//
// `helpers/builders.js` is a bundle module whose imports are extensionless, so
// Node cannot load it (the `*_contract` tests read it as text for the same
// reason). The part that decides the wording lives in `helpers/last_seen.mjs`
// and is importable on its own, which is what makes these assertions possible.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {formatLastSeen} from "./last_seen.mjs";
import {relativeTimeText} from "../i18n/relative_time.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

/** The globals `locale.mjs` reads, at the shape a served page has. */
function installBrowser(catalog) {
    globalThis.window = {LQOS_I18N: catalog};
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang: ""}, cookie: ""};
}

test.afterEach(() => {
    delete globalThis.window;
    delete globalThis.localStorage;
    delete globalThis.document;
});

test("a last-seen age renders through the shared relative-time helper", () => {
    installBrowser(EN);
    assert.equal(formatLastSeen(0), "-");
    assert.equal(formatLastSeen(undefined), "-");
    assert.equal(formatLastSeen(Number.NaN), "-");
    assert.equal(formatLastSeen(-1), "-");
    assert.equal(formatLastSeen(30 * 1e9), relativeTimeText(Date.now() / 1000 - 30));
});

test("the five-minute clamp is gone: an older sighting keeps its real unit", () => {
    installBrowser(EN);
    const threeHours = 3 * 3600 * 1e9;
    assert.equal(formatLastSeen(threeHours), relativeTimeText(Date.now() / 1000 - 3 * 3600));
    assert.equal(/>\s*5 Minutes/.test(formatLastSeen(threeHours)), false);
});

test("the same age reads as Spanish under the served es catalog", () => {
    installBrowser(ES);
    const twoDays = 2 * 86400 * 1e9;
    const text = formatLastSeen(twoDays);
    assert.equal(text, relativeTimeText(Date.now() / 1000 - 2 * 86400));
    // The wording is Intl's, so this only checks that no English suffix leaked
    // back in through a hand-built string.
    assert.equal(/\bago\b/.test(text), false);
});
