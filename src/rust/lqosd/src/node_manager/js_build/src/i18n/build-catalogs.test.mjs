// Chain tests for the catalog build: static2/i18n/*.json → the generated classic
// scripts → the runtime. The rejection half pins every validation
// build-catalogs.mjs owns, one mutated fixture per class.

import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {dirname, join, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t} from "./index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const JS_BUILD_DIR = resolve(HERE, "../..");
const CATALOG_DIR = resolve(JS_BUILD_DIR, "../static2/i18n");
const BUILDER = resolve(JS_BUILD_DIR, "i18n/build-catalogs.mjs");

const roots = [];

test.after(() => {
    for (const root of roots) rmSync(root, {recursive: true, force: true});
});

function readJson(path) {
    return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
    writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

/** A writable copy of the real catalogs, driven through the builder's flags. */
function fixture() {
    const root = mkdtempSync(join(tmpdir(), "lqos-i18n-"));
    roots.push(root);
    const catalogs = join(root, "catalogs");
    cpSync(CATALOG_DIR, catalogs, {recursive: true});
    return {root, catalogs, out: join(root, "out"), file: (name) => join(catalogs, name)};
}

function build(tree) {
    return spawnSync(
        process.execPath,
        [BUILDER, "--catalogs", tree.catalogs, "--out", tree.out],
        {encoding: "utf8"},
    );
}

function assertRejected(tree, expected) {
    const result = build(tree);
    assert.equal(result.status, 1, `expected the build to fail\nstdout: ${result.stdout}`);
    assert.match(result.stderr, /^\[i18n\] build-catalogs: /m);
    assert.match(result.stderr, expected);
}

/** Runs an emitted catalog script against `window`, as a <script> would. */
function load(window, file) {
    new Function("window", readFileSync(file, "utf8"))(window);
    return window;
}

test("the emitted scripts carry the JSON catalogs into the runtime", () => {
    const tree = fixture();
    // A key the served catalog lacks must render English, so drop one pre-build.
    const spanish = readJson(tree.file("es.json"));
    delete spanish.messages["common.nav.topology"];
    writeJson(tree.file("es.json"), spanish);

    const result = build(tree);
    assert.equal(result.status, 0, result.stderr);

    // (a) locale, direction and messages round-trip out of static2/i18n/*.json,
    // under the global the runtime reads for that role.
    const emitted = {};
    for (const tag of ["en", "es"]) emitted[tag] = load({}, join(tree.out, `${tag}.js`));
    assert.deepEqual(emitted.en.LQOS_I18N_EN, readJson(tree.file("en.json")));
    assert.deepEqual(emitted.es.LQOS_I18N, readJson(tree.file("es.json")));
    // (b) the fallback and the served catalog have their own names.
    assert.equal(emitted.en.LQOS_I18N, undefined, "the fallback must not claim the served catalog's global");
    assert.equal(emitted.es.LQOS_I18N_EN, undefined, "a served catalog must not overwrite the fallback");

    // (c) en then es, as the shell loads them: the dropped key renders English.
    const served = {};
    for (const tag of ["en", "es"]) load(served, join(tree.out, `${tag}.js`));
    globalThis.window = served;
    try {
        assert.equal(t("common.nav.dashboard"), "Panel");
        assert.equal(t("common.nav.topology"), "Topology");
    } finally {
        delete globalThis.window;
    }
});

test("the emitted locale list is the whole {tag, name, dir} projection", () => {
    const tree = fixture();
    const result = build(tree);
    assert.equal(result.status, 0, result.stderr);

    // The selector labels each option from `name`; a projection that drops a
    // field renders "undefined" with the tags still correct.
    const listed = readJson(tree.file("locales.json"));
    assert.deepEqual(load({}, join(tree.out, "locales.js")).LQOS_I18N_LOCALES, listed);
});

test("the served catalog wins whichever script the page emits last", () => {
    const tree = fixture();
    const result = build(tree);
    assert.equal(result.status, 0, result.stderr);

    // The shell loads the fallback first, but a hand-edited page, a new
    // entrypoint or a page that skips the placeholder must not silently read
    // English while `<html lang>` says `es`.
    const reversed = {};
    load(reversed, join(tree.out, "es.js"));
    load(reversed, join(tree.out, "en.js"));
    globalThis.window = reversed;
    try {
        assert.equal(reversed.LQOS_I18N.locale, "es");
        assert.equal(t("common.nav.dashboard"), "Panel");
    } finally {
        delete globalThis.window;
    }
});

test("the en-XA pseudo-locale is emitted from en.json and keeps literals intact", () => {
    const tree = fixture();
    const result = build(tree);
    assert.equal(result.status, 0, result.stderr);

    const pseudo = load({}, join(tree.out, "en-XA.js")).LQOS_I18N;
    const english = readJson(tree.file("en.json"));
    assert.equal(pseudo.locale, "en-XA");
    assert.deepEqual(
        Object.keys(pseudo.messages).sort(),
        Object.keys(english.messages).sort(),
        "the overflow check must cover every catalog key",
    );

    for (const [key, message] of Object.entries(english.messages)) {
        const source = typeof message === "string" ? [["", message]] : Object.entries(message);
        for (const [category, text] of source) {
            const translated = category === "" ? pseudo.messages[key] : pseudo.messages[key]?.[category];
            const label = `${key}${category === "" ? "" : `.${category}`}`;
            assert.equal(typeof translated, "string", `${label} must keep its message shape`);
            assert.notEqual(translated, text, `${label} must not read as English`);
            assert.ok(
                translated.length >= text.length * 1.25,
                `${label} must expand by at least 25% (${translated.length} vs ${text.length})`,
            );
        }
    }

    // The two literals the overflow check must exercise, not break.
    assert.match(pseudo.messages["common.confirm.delete"], /\{name\}/);
    assert.match(pseudo.messages["common.help.docs_link"], /<a href="\{url\}">/);
});

test("build-catalogs rejects a locale entry with an empty native name", () => {
    const tree = fixture();
    const locales = readJson(tree.file("locales.json"));
    locales[1].name = "  ";
    writeJson(tree.file("locales.json"), locales);
    assertRejected(tree, /"es" needs its native name/);
});

test("build-catalogs rejects a locale entry whose dir is not ltr or rtl", () => {
    const tree = fixture();
    const locales = readJson(tree.file("locales.json"));
    locales[1].dir = "left-to-right";
    writeJson(tree.file("locales.json"), locales);
    assertRejected(tree, /"es" dir must be "ltr" or "rtl"/);
});

test("build-catalogs rejects a locales.json entry that is not {tag,name,dir}", () => {
    const tree = fixture();
    const locales = readJson(tree.file("locales.json"));
    delete locales[1].dir;
    writeJson(tree.file("locales.json"), locales);
    assertRejected(tree, /every entry needs exactly "tag", "name" and "dir"/);
});

test("build-catalogs rejects a duplicate locale tag", () => {
    const tree = fixture();
    const locales = readJson(tree.file("locales.json"));
    locales.push({tag: "en", name: "English", dir: "ltr"});
    writeJson(tree.file("locales.json"), locales);
    assertRejected(tree, /duplicate locale tag "en"/);
});

test("build-catalogs rejects the en-XA pseudo-locale as a listed locale", () => {
    const tree = fixture();
    const locales = readJson(tree.file("locales.json"));
    locales.push({tag: "en-XA", name: "Pseudo", dir: "ltr"});
    writeJson(tree.file("locales.json"), locales);
    assertRejected(tree, /en-XA is a generated pseudo-locale/);
});

test("build-catalogs rejects a catalog that locales.json does not list", () => {
    const tree = fixture();
    const french = readJson(tree.file("es.json"));
    french.locale = "fr";
    writeJson(tree.file("fr.json"), french);
    assertRejected(tree, /locale "fr" is not listed in locales.json/);
});

// The other half of the pairing: a listed locale whose catalog was never added.
// It fails at the read, not in a later stage, because every consumer of
// `locales.json` assumes the file exists.
test("build-catalogs rejects a locales.json entry with no catalog beside it", () => {
    const tree = fixture();
    const locales = readJson(tree.file("locales.json"));
    writeJson(tree.file("locales.json"), [...locales, {tag: "fr", name: "Français", dir: "ltr"}]);
    assertRejected(tree, /cannot read .*fr\.json/);
});

test("build-catalogs rejects a catalog whose locale field disagrees with its file name", () => {
    const tree = fixture();
    const spanish = readJson(tree.file("es.json"));
    spanish.locale = "en";
    writeJson(tree.file("es.json"), spanish);
    assertRejected(tree, /"locale" must be "es", found "en"/);
});

test("build-catalogs rejects a catalog whose dir disagrees with locales.json", () => {
    const tree = fixture();
    const spanish = readJson(tree.file("es.json"));
    spanish.dir = "rtl";
    writeJson(tree.file("es.json"), spanish);
    assertRejected(tree, /"dir" must be "ltr" \(from locales.json\), found "rtl"/);
});

test("build-catalogs rejects a plural object without an other variant", () => {
    const tree = fixture();
    const spanish = readJson(tree.file("es.json"));
    delete spanish.messages["tree.limit.excess_devices"].other;
    writeJson(tree.file("es.json"), spanish);
    assertRejected(tree, /plural object needs an "other" variant/);
});

test("build-catalogs rejects an unknown plural category", () => {
    const tree = fixture();
    const spanish = readJson(tree.file("es.json"));
    spanish.messages["tree.limit.excess_devices"].lots = "{count} dispositivos";
    writeJson(tree.file("es.json"), spanish);
    assertRejected(tree, /unknown plural category "lots"/);
});
