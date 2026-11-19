// Tests for the extractor gate: the migration ledger (a row states a target
// until it is done) and the plural probe set (a locale's own categories must be
// reachable). Both halves fail `--check` only when they are wrong.

import assert from "node:assert/strict";
import {spawnSync} from "node:child_process";
import {mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from "node:fs";
import {tmpdir} from "node:os";
import {dirname, join, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const JS_BUILD_DIR = resolve(HERE, "../..");
const EXTRACTOR = resolve(JS_BUILD_DIR, "i18n/extract.mjs");

const EN_LOCALE = {tag: "en", name: "English", dir: "ltr"};
const EN_CATALOG = {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree"}};
const SOURCE = 'const label = t("common.nav.tree");\n';

const roots = [];

test.after(() => {
    for (const root of roots) rmSync(root, {recursive: true, force: true});
});

function writeJson(path, value) {
    writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

/** A node_manager-shaped fixture: catalogs, one source file, migration ledger. */
function fixture({
    // The default source keys one message, so the fixture ships the row that
    // records it: the board gate fails a keyed file with no row.
    migration = {"js_build/src/template.js": {status: "done", expected: 1}},
    seeds = null,
    locales = [EN_LOCALE],
    catalogs = {en: EN_CATALOG},
    source = SOURCE,
    html = "<html lang=\"en\"></html>\n",
    entrypoints = null,
} = {}) {
    const root = mkdtempSync(join(tmpdir(), "lqos-i18n-extract-"));
    roots.push(root);
    const catalogDir = join(root, "static2/i18n");
    mkdirSync(catalogDir, {recursive: true});
    mkdirSync(join(root, "js_build/src/i18n"), {recursive: true});
    mkdirSync(join(root, "js_build/i18n"), {recursive: true});
    writeJson(join(catalogDir, "locales.json"), locales);
    for (const [tag, catalog] of Object.entries(catalogs)) writeJson(join(catalogDir, `${tag}.json`), catalog);
    writeFileSync(join(root, "static2/template.html"), html);
    writeFileSync(join(root, "js_build/src/template.js"), source);
    if (entrypoints) writeFileSync(join(root, "js_build/entrypoints.txt"), `${entrypoints.join("\n")}\n`);
    writeJson(join(root, "js_build/i18n/migration.json"), migration);
    if (seeds) writeJson(join(root, "js_build/i18n/seeds.json"), seeds);
    return root;
}

function check(root) {
    return spawnSync(process.execPath, [EXTRACTOR, "--check", "--root", root], {encoding: "utf8"});
}

function inventory(root) {
    return spawnSync(process.execPath, [EXTRACTOR, "--inventory", "--root", root], {encoding: "utf8"});
}

function writeBoard(root) {
    return spawnSync(process.execPath, [EXTRACTOR, "--write-board", "--root", root], {encoding: "utf8"});
}

test("a done migration row that keys its expected count passes", () => {
    const root = fixture({
        migration: {
            "js_build/src/template.js": {status: "done", expected: 1},
            "static2/template.html": {status: "done", expected: 0},
        },
    });
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
});

test("a pending row fails: the board may not record work that is still outstanding", () => {
    // `extract.test.mjs` used to declare a pending row legitimate, so a status
    // flip was invisible; the acceptance reads "every row done", and this is the
    // check that makes that readable from the gate instead of from the file.
    const root = fixture({
        migration: {
            "js_build/src/template.js": {status: "done", expected: 1},
            "static2/template.html": {status: "in-progress", expected: 63},
        },
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /migration\.json: "static2\/template\.html" is "in-progress", not done/);
});

test("a row that keys more than its expected count fails", () => {
    const root = fixture({migration: {"js_build/src/template.js": {status: "in-progress", expected: 0}}});
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /migration\.json: "js_build\/src\/template\.js" has 1 catalog keys but targets 0/);
});

test("a done row that keys fewer than its expected count fails", () => {
    const root = fixture({migration: {"js_build/src/template.js": {status: "done", expected: 2}}});
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /marked done but only 1 of 2 keys are migrated/);
});

test("a plural variant the locale really selects is not reported as impossible", () => {
    // Spanish selects "many" for millions: a probe set that stops at 21 would
    // reject this legitimate catalog.
    const root = fixture({
        locales: [EN_LOCALE, {tag: "es", name: "Español", dir: "ltr"}],
        source: 'const label = tn("tree.limit.excess_devices", 3);\n',
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {"tree.limit.excess_devices": {one: "{count} device", other: "{count} devices"}},
            },
            es: {
                locale: "es",
                dir: "ltr",
                messages: {"tree.limit.excess_devices": {many: "{count} dispositivos", other: "{count} dispositivos"}},
            },
        },
    });
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
});

// A call site and its message can disagree about a parameter name in a way no
// catalog check sees: `t("k", {elapsed})` against a message that names
// `{duration}` renders the placeholder literally, in every language. The gate
// reads object-literal arguments and compares their names against `en.json`.

test("a locale may not flatten a plural object into a string", () => {
    // The key sets and the placeholder names agree here; only the kind differs.
    // `tn()` on a flat string renders the raw key ("tree.limit.excess_devices"),
    // and no per-file contract reads a third locale, so this has to be generic.
    const root = fixture({
        locales: [EN_LOCALE, {tag: "es", name: "Español", dir: "ltr"}],
        source: 'const label = tn("tree.limit.excess_devices", 3);\n',
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {"tree.limit.excess_devices": {one: "{count} device", other: "{count} devices"}},
            },
            es: {locale: "es", dir: "ltr", messages: {"tree.limit.excess_devices": "{count} dispositivos"}},
        },
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(
        result.stdout,
        /es\.json "tree\.limit\.excess_devices" is a string where en\.json is a plural object; a locale must keep a message's kind/,
    );
});

test("a locale may not pluralize a plain string either", () => {
    // The other direction: `t()` on a plural object renders the key, so a locale
    // that invents categories for a one-line message breaks the same way.
    const root = fixture({
        locales: [EN_LOCALE, {tag: "es", name: "Español", dir: "ltr"}],
        source: 'const label = t("common.nav.tree");\n',
        catalogs: {
            en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree of {name}"}},
            es: {locale: "es", dir: "ltr", messages: {"common.nav.tree": {one: "Árbol de {name}", other: "Árboles de {name}"}}},
        },
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /es\.json "common\.nav\.tree" is a plural object where en\.json is a string/);
});

test("an extra plural category the locale selects stays allowed", () => {
    // Spanish legitimately adds `many`; the kind check must not read that as a
    // shape change.
    const root = fixture({
        locales: [EN_LOCALE, {tag: "es", name: "Español", dir: "ltr"}],
        source: 'const label = tn("tree.limit.excess_devices", 3);\n',
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {"tree.limit.excess_devices": {one: "{count} device", other: "{count} devices"}},
            },
            es: {
                locale: "es",
                dir: "ltr",
                messages: {
                    "tree.limit.excess_devices": {
                        one: "Dispositivo no mostrado: {count}",
                        many: "Dispositivos no mostrados: {count}",
                        other: "Dispositivos no mostrados: {count}",
                    },
                },
            },
        },
    });
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
});

test("a toFixed() string inside a message's parameters fails", () => {
    // REV-T1's class: `pct.toFixed(1)` reaches `t()` as a string, so Intl never
    // formats it and a Spanish sentence renders "12.5" beside "12,5 %".
    const root = fixture({
        source: 'const label = t("common.nav.tree", {percent: pct.toFixed(1)});\n',
        catalogs: {en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "{percent}% of total"}}},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /template\.js:1 t\("common\.nav\.tree"\) formats a parameter with toFixed\(\)/);

    // `formatNumber()` is the sanctioned path, and it satisfies the same gate.
    writeFileSync(
        join(root, "js_build/src/template.js"),
        'const label = t("common.nav.tree", {percent: formatNumber(pct, ONE_DECIMAL)});\n',
    );
    const fixed = check(root);
    assert.equal(fixed.status, 0, `${fixed.stdout}${fixed.stderr}`);
});

test("a t() result interpolated into an HTML attribute fails", () => {
    const root = fixture({source: 'const html = `<button title="${t("common.nav.tree")}">x</button>`;\n'});
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /template\.js:1 interpolates t\(\) into an HTML attribute; use tAttr\(\)/);

    writeFileSync(join(root, "js_build/src/template.js"), 'const html = `<button title="${tAttr("common.nav.tree")}">x</button>`;\n');
    const fixed = check(root);
    assert.equal(fixed.status, 0, `${fixed.stdout}${fixed.stderr}`);
});

test("an already-escaped value handed to tHtml() fails", () => {
    const root = fixture({
        source: 'const html = tHtml("common.nav.tree", {name: escapeHtml(name)});\n',
        catalogs: {en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree of {name}"}}},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /template\.js:1 passes an escaped value to tHtml\(\), which escapes parameters itself/);

    writeFileSync(join(root, "js_build/src/template.js"), 'const html = tHtml("common.nav.tree", {name});\n');
    const fixed = check(root);
    assert.equal(fixed.status, 0, `${fixed.stdout}${fixed.stderr}`);
});

test("an object-literal call site that supplies the message's placeholders passes", () => {
    const root = fixture({
        catalogs: {en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree of {name} ({count})"}}},
        source: 'const label = t("common.nav.tree", {name: site, count: 3});\n',
    });
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(
        result.stdout,
        /\[i18n\] params: 1 object-literal call site\(s\) checked against en\.json, 0 without parameters, 0 not checkable statically/,
    );
});

test("a parameter the message does not name fails", () => {
    const root = fixture({
        catalogs: {en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree at {duration}"}}},
        source: 'const label = t("common.nav.tree", {elapsed: "2.0s", remaining: "3.0s"});\n',
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(
        result.stdout,
        /template\.js:1 t\("common\.nav\.tree"\) disagrees with en\.json about the parameter names \(missing: duration; unexpected: elapsed, remaining\)/,
    );
});

test("a placeholder the call site omits fails", () => {
    const root = fixture({
        catalogs: {en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "{name} of {count}"}}},
        source: 'const label = t("common.nav.tree", {name: site});\n',
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /\(missing: count; unexpected: none\)/);
});

test("a tn() call site is read from the argument after the count", () => {
    const root = fixture({
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {"tree.limit.excess_devices": {one: "{count} device", other: "{count} devices"}},
            },
        },
        source: 'const label = tn("tree.limit.excess_devices", 3, {count: 3});\n',
    });
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(result.stdout, /1 object-literal call site\(s\) checked/);
});

test("a tn() call site that passes the count twice under another name fails", () => {
    const root = fixture({
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {"tree.limit.excess_devices": {one: "{count} device", other: "{count} devices"}},
            },
        },
        source: 'const label = tn("tree.limit.excess_devices", 3, {total: 3});\n',
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /disagrees with en\.json about the parameter names \(missing: count; unexpected: total\)/);
});

test("an argument that is not an object literal is reported, not guessed at", () => {
    const root = fixture({source: 'const label = t("common.nav.tree", range);\n'});
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(result.stdout, /\[i18n\] params: 0 object-literal call site\(s\) checked against en\.json, 0 without parameters, 1 not checkable statically/);
    assert.match(result.stdout, /template\.js:1 t\("common\.nav\.tree"\) passes `range` \(argument is not a static object literal\)/);
});

test("an object literal with a spread is unverifiable, so it is skipped rather than failed", () => {
    const root = fixture({source: 'const label = t("common.nav.tree", {...params, name: value});\n'});
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(result.stdout, /not checkable statically/);
});

test("a test file is not a UI consumer", () => {
    // The key is read only by an assertion; nothing renders it, so the gate must
    // still report it as unused.
    const root = fixture({
        source: 'const label = t("common.nav.tree");\n',
        catalogs: {
            en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree", "common.action.save": "Save"}},
        },
    });
    writeFileSync(join(root, "js_build/src/i18n/index.test.mjs"), 'assert.equal(t("common.action.save"), "Guardar");\n');
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /en\.json defines "common\.action\.save" but no page or module uses it/);
    assert.match(result.stdout, /1 test file\(s\) excluded/);
});

test("a seed row accounts for a key with no consumer, and names it in the report", () => {
    const root = fixture({
        source: 'const label = t("common.nav.tree");\n',
        catalogs: {
            en: {locale: "en", dir: "ltr", messages: {"common.nav.tree": "Tree", "common.action.save": "Save"}},
        },
        seeds: {"common.action.save": "Seed: the first migrated page with a save button consumes it"},
    });
    const result = check(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(result.stdout, /\[i18n\] seed: common\.action\.save — Seed: the first migrated page/);
});

test("a seed row whose key has gained a consumer fails, so the list can only shrink", () => {
    const root = fixture({
        source: 'const label = t("common.action.save");\n',
        catalogs: {
            en: {locale: "en", dir: "ltr", messages: {"common.action.save": "Save"}},
        },
        seeds: {"common.action.save": "Seed: the first migrated page with a save button consumes it"},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /seeds\.json lists "common\.action\.save".*now uses; delete the row/);
});

test("a seed row for a key no catalog defines fails", () => {
    const root = fixture({
        source: 'const label = t("common.nav.tree");\n',
        seeds: {"common.action.save": "Seed: a key that was renamed away"},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /seeds\.json lists "common\.action\.save", which en\.json does not define/);
});

test("a seeds.json that is not an object fails with its own message", () => {
    const root = fixture({seeds: []});
    const result = check(root);
    assert.equal(result.status, 1, `${result.stdout}${result.stderr}`);
    assert.match(result.stderr, /seeds\.json: expected an object of \{key: reason\}/);
});

test("a seed row whose reason is empty or not a string fails with its own message", () => {
    // Both halves of the guard: a blank reason and a non-string one.
    const root = fixture({
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {"common.nav.tree": "Tree", "common.action.save": "Save", "common.action.cancel": "Cancel"},
            },
        },
        seeds: {"common.action.save": "   ", "common.action.cancel": 7},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /seeds\.json: "common\.action\.save" needs a one-line reason/);
    assert.match(result.stdout, /seeds\.json: "common\.action\.cancel" needs a one-line reason/);
});

// The inventory and its ratchet: the board states what is left to translate per
// file, and a unit may move that number down but not up.

test("--inventory reports candidates per layer against the keys already in the file", () => {
    const root = fixture({
        html: "<h2>Shaped Devices</h2>\n<p>Two words</p>\n",
        source: 'const label = t("common.nav.tree");\nconst heading = "Live";\n',
    });
    const result = inventory(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(result.stdout, /inventory: 1 html \+ 1 js\/mjs first-party file\(s\); 3 candidate\(s\) in 2 file\(s\), 3 distinct/);
    assert.match(result.stdout, /static2\/template\.html\s+2\s+0\s+2\s+0\s+2/);
    // The key `t("common.nav.tree")` consumes, and the `"Live"` literal neither
    // a key nor a marker resolves: one candidate is still English.
    assert.match(result.stdout, /js_build\/src\/template\.js\s+0\s+1\s+1\s+1\s+1/);
});

test("--inventory reports a live literal even when the file carries more catalog keys", () => {
    // The shape that hid template.html's five literals: several keyed elements
    // and one text node still in English. A remaining count computed as
    // "candidates minus keys" answered 0 here (`1 - 3` clamps to 0), so this
    // fixture fails on that formula and passes only on the candidate count.
    const root = fixture({
        html: [
            "<nav>",
            '<a href="index.html" data-i18n="common.nav.tree">Tree</a>',
            '<a href="devices.html" data-i18n="common.nav.devices">Devices</a>',
            '<a href="help.html" data-i18n="common.nav.help">Help</a>',
            '<!-- translatable: false — "Sign in" is the provider flow name, not page copy -->',
            "<p>Sign in</p>",
            "<p>Reload required.</p>",
            "</nav>",
            "",
        ].join("\n"),
        // No JS key in the fixture: the row's three keys and one literal are
        // all the HTML contributes.
        source: "",
        catalogs: {
            en: {
                locale: "en",
                dir: "ltr",
                messages: {
                    "common.nav.tree": "Tree",
                    "common.nav.devices": "Devices",
                    "common.nav.help": "Help",
                },
            },
        },
    });
    const result = inventory(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    // One unkeyed, unmarked literal against three keys: remaining is 1, not 0.
    assert.match(result.stdout, /static2\/template\.html\s+1\s+0\s+1\s+3\s+1/);
    assert.match(result.stdout, /1 candidate\(s\) \(heuristic; candidates, not facts\), 3 keyed, 1 remaining/);
});

test("--inventory writes nothing", () => {
    const root = fixture({migration: {"js_build/src/template.js": {status: "pending", expected: 0}}});
    const before = readFileSync(join(root, "js_build/i18n/migration.json"), "utf8");
    inventory(root);
    assert.equal(readFileSync(join(root, "js_build/i18n/migration.json"), "utf8"), before);
});

test("--inventory fails under --check instead of reporting and exiting 0", () => {
    // The acceptance line used to read an exit code the extractor never set: an
    // `--inventory` run returned before the findings path, so `tee`'s status was
    // the only thing a pipeline could observe.
    const root = fixture({migration: {}, source: 'const heading = "Live";\n'});
    const plain = inventory(root);
    assert.equal(plain.status, 0, `${plain.stdout}${plain.stderr}`);
    assert.match(plain.stdout, /\[i18n\] note: .*no migration\.json row/);

    const gated = spawnSync(process.execPath, [EXTRACTOR, "--inventory", "--check", "--root", root], {encoding: "utf8"});
    assert.equal(gated.status, 1, gated.stdout);
    assert.match(gated.stdout, /\[i18n\] FAIL: .*no migration\.json row/);
});

test("a file may not add candidates above the board's recorded count", () => {
    const root = fixture({
        source: 'const heading = "Live";\n',
        migration: {"js_build/src/template.js": {status: "pending", expected: 0, candidates: 0}},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(
        result.stdout,
        /"js_build\/src\/template\.js" has 1 candidate\(s\) but the board records 0; a unit may not add untranslated literals/,
    );
});

test("a file whose candidates match the board passes, and so does a lower count", () => {
    const root = fixture({
        source: 'const heading = "Live";\n',
        catalogs: {en: {locale: "en", dir: "ltr", messages: {}}},
        migration: {"js_build/src/template.js": {status: "done", expected: 0, candidates: 1}},
    });
    const exact = check(root);
    assert.equal(exact.status, 0, `${exact.stdout}${exact.stderr}`);
    const source = join(root, "js_build/src/template.js");
    // A candidate the heuristic filters is not a translation: the count may drop.
    writeFileSync(source, 'const heading = "Mbps";\n');
    const lower = check(root);
    assert.equal(lower.status, 0, `${lower.stdout}${lower.stderr}`);
});

test("a board row without a candidate count is reported rather than trusted", () => {
    const root = fixture({
        source: 'const heading = "Live";\n',
        migration: {"js_build/src/template.js": {status: "pending", expected: 0}},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /has 1 candidate\(s\) and no recorded count; run --write-board/);
});

test("a first-party file with candidates and no row fails, so a page cannot escape the board", () => {
    const root = fixture({source: 'const heading = "Live";\n', migration: {}});
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(
        result.stdout,
        /js_build\/src\/template\.js has 1 candidate\(s\) and 0 key\(s\) and no migration\.json row; run --write-board/,
    );
});

test("a file that keys a message with no board row fails, even with no candidates left", () => {
    // The 19-key gap: three mapping modules keyed strings that no row accounted
    // for, so the board's "all rows done" claim covered fewer files than the
    // inventory measured.
    const root = fixture({migration: {}});
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(
        result.stdout,
        /js_build\/src\/template\.js has 0 candidate\(s\) and 1 key\(s\) and no migration\.json row/,
    );
});

test("--write-board adds pending rows with the measured count and keeps a done row's expected", () => {
    const root = fixture({
        html: "<h2>Shaped Devices</h2>\n",
        source: 'const heading = "Live";\n',
        entrypoints: ["template.js"],
        catalogs: {en: {locale: "en", dir: "ltr", messages: {}}},
        migration: {"static2/template.html": {status: "pending", expected: 0}},
    });
    const result = writeBoard(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    const board = JSON.parse(readFileSync(join(root, "js_build/i18n/migration.json"), "utf8"));
    assert.deepEqual(board["static2/template.html"], {status: "pending", expected: 0, candidates: 1});
    assert.deepEqual(board["js_build/src/template.js"], {status: "pending", expected: 1, candidates: 1});
    // Writing the board records the measurement; it never marks a file migrated,
    // so the gate reports the fresh rows until someone flips them.
    const after = check(root);
    assert.equal(after.status, 1, after.stdout);
    assert.match(after.stdout, /"js_build\/src\/template\.js" is "pending", not done/);
    assert.match(after.stdout, /"static2\/template\.html" is "pending", not done/);
});

test("--write-board drops a row whose file is gone and says so", () => {
    const root = fixture({
        migration: {
            "js_build/src/template.js": {status: "done", expected: 1},
            "static2/template.html": {status: "done", expected: 0},
            "js_build/src/removed.js": {status: "pending", expected: 3, candidates: 3},
        },
    });
    const result = writeBoard(root);
    assert.equal(result.status, 0, `${result.stdout}${result.stderr}`);
    assert.match(result.stdout, /dropped row js_build\/src\/removed\.js \(no such file\)/);
    assert.equal(check(root).status, 0);
});

test("a row naming no first-party file fails the check", () => {
    const root = fixture({migration: {"js_build/src/removed.js": {status: "pending", expected: 3, candidates: 3}}});
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /migration\.json: "js_build\/src\/removed\.js" names no first-party page or module/);
});

test("a translatable: false marker that names nothing reaches the check", () => {
    const root = fixture({
        source: '// translatable: false — "Gbps" is a unit\nconst heading = "Live";\n',
        migration: {"js_build/src/template.js": {status: "pending", expected: 0, candidates: 1}},
    });
    const result = check(root);
    assert.equal(result.status, 1, result.stdout);
    assert.match(result.stdout, /template\.js line 1: "translatable: false" names "Gbps", which is not a string in this file/);
});
