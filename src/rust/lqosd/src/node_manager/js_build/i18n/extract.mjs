#!/usr/bin/env node
// i18n coverage gate for the node_manager UI.
//
// Scans `static2/**/*.html` for `data-i18n*` keys and `js_build/src/**` for
// literal `t()`, `tn()` and `tHtml()` keys, then reports:
//   - keys the UI uses but `en.json` does not define,
//   - `en.json` keys nothing uses (except `server.*` and `common.state.*`,
//     which render only for states that are not always on screen, and the seed
//     rows in `seeds.json`, which name the page that will consume them),
//   - plural variants of one message that disagree about placeholders,
//   - keys whose placeholders differ between the locales defining them,
//   - a call site whose object-literal parameters disagree with its message's
//     placeholders in `en.json` (a parameter the message never names, or a
//     placeholder the call site never supplies),
//   - plural categories the locale can never select,
//   - migration.json rows that drifted: keys past the target, or a done row that
//     does not key its target,
//   - a file whose migratable-literal count rose above the board's recorded
//     count: a unit may translate strings, never add new ones,
//   - a first-party file with candidates and no board row, so a new page cannot
//     ship outside the ledger,
//   - a `translatable: false` marker that names no candidate in its file.
//
// `*.test.mjs`/`*.test.js` are excluded from the usage scan: an assertion is not
// a screen, and counting one as a consumer is what let five keys and `tHtml()`
// pass with no caller. A key with no UI consumer belongs in `seeds.json`, which
// states which page will render it; a seed row whose key has gained a consumer
// fails, so the seed list can only shrink.
//
// `--check` exits 1 when any of that is reported. Without it the same findings
// print as a report, so the count table stays usable mid-migration.
//
// `--root <dir>` points the scan at a fixture laid out like the node_manager
// directory (`static2/…` and `js_build/src/…`); the default is the real tree.
//
// `--inventory` prints the migration denominator instead of the key table: per
// file, the heuristic candidate count from `candidates.mjs` beside the catalog
// keys the file already uses, sorted by what is left to translate. `remaining`
// is the unresolved candidate count: the scanner leaves keyed text and
// attributes out, `applyMarkers` drops the not-text ones, and the `keyed` column
// is context, never a subtrahend. `--write-board` refreshes `migration.json`
// from that measurement — a row's `candidates` is a ceiling, so the only
// legitimate move is down. Both modes report; they do not run the gate, so a
// board refresh never hides a finding.

import {existsSync, readFileSync, readdirSync, writeFileSync} from "node:fs";
import {dirname, relative, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {applyMarkers, scanCode, scanMarkup} from "./candidates.mjs";
import {FALLBACK_LOCALE} from "../src/i18n/locale.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const JS_BUILD_DIR = resolve(HERE, "..");
const NODE_MANAGER_DIR = resolve(JS_BUILD_DIR, "..");
const FALLBACK_ROOT = NODE_MANAGER_DIR;

// States that are not on screen most of the time: the key can be correct while
// no page currently renders it.
const ALLOWED_UNUSED = [/^server\./, /^common\.state\./];
const MIGRATION_STATUSES = new Set(["pending", "in-progress", "done"]);
// A key an assertion reads is not a key a page renders.
const TEST_FILE = /\.test\.(?:js|mjs)$/;
// 1_000_000 and above matter: Spanish selects "many" for millions, so a probe
// set that stops at 21 would report a legitimate `many` variant as impossible.
const PLURAL_PROBES = [0, 1, 2, 5, 11, 21, 1_000_000, 1_000_000_000];
const HTML_KEY = /data-i18n(?:-title|-placeholder|-aria-label|-alt)?\s*=\s*["']([^"']+)["']/g;
const JS_KEY = /\b(tHtml|tAttr|tn|t)\(\s*["']([^"']+)["']/g;
// Keys the Rust side emits: `template.rs` builds `data-i18n` markup itself and
// passes each sidebar status label to `status_item_html` as an
// `(English text, catalog key)` pair. Both shapes are keys this scan has to see
// — otherwise a typo'd key keeps its inline English and only a Rust test
// notices. The pair is anchored to the call site so the test's own table of
// expected keys cannot stand in for the emitter.
const RUST_KEY = /(?:data-i18n(?:-title|-placeholder|-aria-label|-alt)?\s*=\s*["']|status_item_html\([^;]*?\(\s*"[^"]*"\s*,\s*")([a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+)["']/g;
const PLACEHOLDER = /\{([A-Za-z0-9_]+)\}/g;
const CHECK = process.argv.includes("--check");
const INVENTORY = process.argv.includes("--inventory");
const WRITE_BOARD = process.argv.includes("--write-board");
// First-party means "we wrote it and it must be translated". A vendored copy
// kept in-tree is neither, and counting one would move the denominator without
// moving the work. A new vendored file shows up as candidates, which is the
// visible failure; adding it here is how the list grows.
const VENDORED_FILES = new Set(["js_build/src/lq_js_common/helpers/cbor-x.js"]);

// Scan roots: the real node_manager tree unless `--root` points at a fixture.
let ROOT = FALLBACK_ROOT;
let STATIC_DIR = resolve(ROOT, "static2");
let SOURCE_DIR = resolve(JS_BUILD_DIR, "src");
let CATALOG_DIR = resolve(STATIC_DIR, "i18n");
let MIGRATION_PATH = resolve(HERE, "migration.json");
let SEED_PATH = resolve(HERE, "seeds.json");
let ENTRYPOINTS_PATH = resolve(JS_BUILD_DIR, "entrypoints.txt");

function configure() {
    const index = process.argv.indexOf("--root");
    if (index === -1) return;
    const value = process.argv[index + 1];
    if (!value || value.startsWith("--")) fail("--root needs a directory");
    ROOT = resolve(value);
    STATIC_DIR = resolve(ROOT, "static2");
    SOURCE_DIR = resolve(ROOT, "js_build/src");
    CATALOG_DIR = resolve(STATIC_DIR, "i18n");
    MIGRATION_PATH = resolve(ROOT, "js_build/i18n/migration.json");
    SEED_PATH = resolve(ROOT, "js_build/i18n/seeds.json");
    ENTRYPOINTS_PATH = resolve(ROOT, "js_build/entrypoints.txt");
}

const findings = [];
const catalogs = new Map();

function fail(message) {
    console.error(`[i18n] extract: ${message}`);
    process.exit(1);
}

function displayPath(path) {
    const relativePath = relative(ROOT, path);
    return relativePath.startsWith("..") ? path : relativePath;
}

function readJson(path) {
    if (!existsSync(path)) fail(`missing ${displayPath(path)}`);
    try {
        return JSON.parse(readFileSync(path, "utf8"));
    } catch (error) {
        fail(`invalid JSON in ${displayPath(path)}: ${error.message}`);
    }
}

function walk(directory) {
    const files = [];
    for (const entry of readdirSync(directory, {withFileTypes: true})) {
        const path = resolve(directory, entry.name);
        if (entry.isDirectory()) files.push(...walk(path));
        else files.push(path);
    }
    return files;
}

/** @returns {Map<string, Set<string>>} file → catalog keys that file mentions. */
function scanFiles(files, pattern, group = 1) {
    const counts = new Map();
    for (const file of files) {
        const keys = new Set();
        for (const match of readFileSync(file, "utf8").matchAll(pattern)) {
            keys.add(match[group].trim());
        }
        if (keys.size > 0) counts.set(file, keys);
    }
    return counts;
}

function placeholdersOf(text) {
    return [...new Set([...text.matchAll(PLACEHOLDER)].map((match) => match[1]))].sort();
}

function messageTexts(message) {
    return typeof message === "string" ? [message] : Object.values(message);
}

/** How a message is shaped: one string, or an object of plural categories. */
function messageKind(message) {
    return typeof message === "string" ? "string" : "plural object";
}

/** Index just past the quoted run starting at `index`; quotes never nest. */
function skipQuoted(text, index) {
    const quote = text[index];
    let cursor = index + 1;
    while (cursor < text.length) {
        if (text[cursor] === "\\") {
            cursor += 2;
            continue;
        }
        if (text[cursor] === quote) return cursor + 1;
        cursor += 1;
    }
    return text.length;
}

/** `text` split on `separator`, ignoring separators inside quotes or brackets. */
function splitTopLevel(text, separator) {
    const parts = [];
    let depth = 0;
    let start = 0;
    let index = 0;
    while (index < text.length) {
        const char = text[index];
        if (char === '"' || char === "'" || char === "`") {
            index = skipQuoted(text, index);
            continue;
        }
        if ("([{".includes(char)) depth += 1;
        else if (")]}".includes(char)) depth -= 1;
        else if (char === separator && depth === 0) {
            parts.push(text.slice(start, index));
            start = index + 1;
        }
        index += 1;
    }
    parts.push(text.slice(start));
    return parts;
}

/**
 * The arguments of a call after its first one, as source text. `start` is the
 * index just past the first argument; the call's own closing parenthesis is the
 * first `)` at depth 0.
 * @returns {string[]}
 */
function callArguments(source, start) {
    const args = [];
    let depth = 0;
    let current = "";
    let index = start;
    while (index < source.length && /\s/.test(source[index])) index += 1;
    if (source[index] === ",") index += 1;
    while (index < source.length) {
        const char = source[index];
        if (char === '"' || char === "'" || char === "`") {
            const end = skipQuoted(source, index);
            current += source.slice(index, end);
            index = end;
            continue;
        }
        if (char === ")" && depth === 0) {
            args.push(current);
            return args;
        }
        if (char === "," && depth === 0) {
            args.push(current);
            current = "";
            index += 1;
            continue;
        }
        if ("([{".includes(char)) depth += 1;
        else if (")]}".includes(char)) depth -= 1;
        current += char;
        index += 1;
    }
    return args;
}

/**
 * The parameter names an object-literal argument supplies, or null when the
 * argument cannot be read statically: not an object literal, a spread, a
 * computed key, or a method. Callers report null as unverifiable, never as a
 * finding — guessing a name here would be exactly the bug this check exists
 * to catch.
 * @returns {Set<string>|null}
 */
function objectLiteralNames(argument) {
    const text = argument.trim();
    if (!text.startsWith("{") || !text.endsWith("}")) return null;
    const names = new Set();
    for (const entry of splitTopLevel(text.slice(1, -1), ",")) {
        const shape = entry.trim();
        if (!shape) continue;
        if (shape.startsWith("...")) return null;
        const named = /^(["']?)([A-Za-z_$][A-Za-z0-9_$]*)\1\s*:/.exec(shape);
        if (named) {
            names.add(named[2]);
            continue;
        }
        if (/^[A-Za-z_$][A-Za-z0-9_$]*$/.test(shape)) {
            names.add(shape);
            continue;
        }
        return null;
    }
    return names;
}

function lineOf(source, index) {
    return source.slice(0, index).split("\n").length;
}

function loadCatalogs(locales) {
    for (const {tag} of locales) {
        const catalog = readJson(resolve(CATALOG_DIR, `${tag}.json`));
        if (!catalog || typeof catalog !== "object" || typeof catalog.messages !== "object" || !catalog.messages) {
            fail(`static2/i18n/${tag}.json: expected a catalog with a "messages" object`);
        }
        catalogs.set(tag, catalog);
    }
}

function englishMessages() {
    return catalogs.get(FALLBACK_LOCALE)?.messages ?? {};
}

function checkUsage(used, usedIn, seeds) {
    const english = englishMessages();
    for (const key of used) {
        if (Object.prototype.hasOwnProperty.call(english, key)) continue;
        findings.push(`"${key}" is used in ${displayPath(usedIn.get(key))} but missing from en.json`);
    }
    for (const key of seeds.keys()) {
        if (!Object.prototype.hasOwnProperty.call(english, key)) {
            findings.push(`seeds.json lists "${key}", which en.json does not define`);
        } else if (used.has(key)) {
            findings.push(`seeds.json lists "${key}", which ${displayPath(usedIn.get(key))} now uses; delete the row`);
        }
    }
    for (const key of Object.keys(english)) {
        if (used.has(key) || seeds.has(key) || ALLOWED_UNUSED.some((pattern) => pattern.test(key))) continue;
        findings.push(`en.json defines "${key}" but no page or module uses it`);
    }
}

/**
 * The seed rows: catalog keys that ship before the page that will render them.
 * A row's value is the reason it has no consumer yet.
 * @returns {Map<string, string>}
 */
function readSeeds() {
    if (!existsSync(SEED_PATH)) return new Map();
    const seeds = readJson(SEED_PATH);
    if (!seeds || typeof seeds !== "object" || Array.isArray(seeds)) {
        fail(`${displayPath(SEED_PATH)}: expected an object of {key: reason}`);
    }
    for (const [key, reason] of Object.entries(seeds)) {
        if (typeof reason !== "string" || !reason.trim()) {
            findings.push(`seeds.json: "${key}" needs a one-line reason`);
        }
    }
    return new Map(Object.entries(seeds));
}

function printSeeds(seeds) {
    for (const [key, reason] of seeds) console.log(`[i18n] seed: ${key} — ${reason}`);
}

function checkPlaceholders() {
    for (const [tag, catalog] of catalogs) {
        for (const [key, message] of Object.entries(catalog.messages)) {
            const variants = new Set(messageTexts(message).map((text) => placeholdersOf(text).join(",") || "none"));
            if (variants.size > 1) {
                findings.push(`${tag}.json "${key}": plural variants use different placeholders (${[...variants].join(" | ")})`);
            }
        }
    }
    const english = englishMessages();
    for (const [tag, catalog] of catalogs) {
        if (tag === FALLBACK_LOCALE) continue;
        for (const [key, message] of Object.entries(catalog.messages)) {
            if (!Object.prototype.hasOwnProperty.call(english, key)) continue;
            // The key sets and the placeholder names can agree while a locale
            // changes the message's kind: a flattened plural object makes `tn()`
            // render the raw key, and a pluralized plain string makes `t()` do
            // the same. Extra plural *categories* stay allowed (es adds `many`).
            if (typeof message !== typeof english[key]) {
                findings.push(
                    `${tag}.json "${key}" is a ${messageKind(message)} where en.json is a ${messageKind(english[key])}; ` +
                        `a locale must keep a message's kind`,
                );
                continue;
            }
            const translated = new Set(messageTexts(message).flatMap(placeholdersOf));
            const source = new Set(messageTexts(english[key]).flatMap(placeholdersOf));
            const missing = [...source].filter((name) => !translated.has(name));
            const extra = [...translated].filter((name) => !source.has(name));
            if (missing.length > 0 || extra.length > 0) {
                findings.push(
                    `${tag}.json "${key}" placeholders differ from en.json (missing: ${missing.join(", ") || "none"}; unexpected: ${extra.join(", ") || "none"})`,
                );
            }
        }
    }
}

/**
 * The mirror of `checkPlaceholders`. That function proves the locales agree on
 * a message's placeholders; this one proves each call site supplies them. A
 * translation that renames a placeholder, or a call site that passes a name the
 * message never had, renders `{name}` literally on screen and no catalog check
 * can see it — the mismatch is between the call and the message.
 *
 * Only an object-literal argument is read: a variable, a spread, or a computed
 * key is reported as unverifiable and skipped, never guessed at.
 * @param {string[]} files The non-test modules that render messages.
 */
function checkCallParameters(files) {
    const english = englishMessages();
    const summary = {checked: 0, withoutParams: 0, skipped: []};
    for (const file of files) {
        const source = readFileSync(file, "utf8");
        for (const match of source.matchAll(JS_KEY)) {
            const [whole, call, rawKey] = match;
            const key = rawKey.trim();
            // `tn(key, count, params)` carries the count before the parameters.
            const paramsIndex = call === "tn" ? 1 : 0;
            const argument = (callArguments(source, match.index + whole.length)[paramsIndex] ?? "").trim();
            const where = `${displayPath(file)}:${lineOf(source, match.index)}`;
            // A stringified value never reaches `t()`'s number formatting, so the
            // decimal mark, grouping and unit spacing come from the JS number
            // instead of Intl. No catalog check can see this: it is a property of
            // the call site, and it looks translated in English.
            if (argument.includes("toFixed(")) {
                findings.push(
                    `${where} ${call}("${key}") formats a parameter with toFixed(); pass a number or formatNumber(...) so the locale renders it`,
                );
            }
            if (!argument) {
                summary.withoutParams += 1;
                continue;
            }
            const names = objectLiteralNames(argument);
            if (!names) {
                const shown = argument.replace(/\s+/g, " ");
                summary.skipped.push(`${where} ${call}("${key}") passes \`${shown.length > 60 ? `${shown.slice(0, 60)}…` : shown}\``);
                continue;
            }
            // A key en.json does not define is `checkUsage`'s finding, not this one.
            if (!Object.prototype.hasOwnProperty.call(english, key)) continue;
            const expected = new Set(messageTexts(english[key]).flatMap(placeholdersOf));
            // `tn` always supplies the count; a message may name it without the
            // call site passing it again.
            if (call === "tn") expected.add("count");
            summary.checked += 1;
            const missing = [...expected].filter((name) => !names.has(name)).sort();
            const extra = [...names].filter((name) => !expected.has(name)).sort();
            if (missing.length === 0 && extra.length === 0) continue;
            findings.push(
                `${where} ${call}("${key}") disagrees with en.json about the parameter names ` +
                    `(missing: ${missing.join(", ") || "none"}; unexpected: ${extra.join(", ") || "none"})`,
            );
        }
    }
    return summary;
}

function printCallParameters(summary) {
    console.log(
        `[i18n] params: ${summary.checked} object-literal call site(s) checked against en.json, ` +
            `${summary.withoutParams} without parameters, ${summary.skipped.length} not checkable statically`,
    );
    for (const line of summary.skipped) console.log(`[i18n] params: skipped ${line} (argument is not a static object literal)`);
}

// A rendered message lands in one of three contexts, and `src/i18n/index.js`
// gives each its own entry point: `t()` text, `tHtml()` markup, `tAttr()` an
// attribute value. Two shapes defeat that split, and neither is visible to the
// catalog checks — both look translated in English:
//   - a `t()` result interpolated straight into an attribute value: `t()`
//     escapes nothing, so a translation carrying `"` or `&` breaks the tag for
//     that locale;
//   - `tHtml()` handed an already-escaped parameter: it escapes parameters
//     itself, so the value renders `&amp;amp;` ("Smith &amp; Sons").
const ATTRIBUTE_CONTEXT = /\b(?:alt|title|placeholder|aria-label)\s*=\s*(?:"\$\{t\(|'\$\{t\(|\$\{t\()/;
const DOUBLE_ESCAPE = /\btHtml\([^;{}]*\{[^}]*\bescape(?:Html|Attr)\(/;

/** @param {string[]} files The non-test modules that render messages. */
function checkRenderContexts(files) {
    for (const file of files) {
        const lines = readFileSync(file, "utf8").split("\n");
        for (const [index, line] of lines.entries()) {
            if (ATTRIBUTE_CONTEXT.test(line)) {
                findings.push(
                    `${displayPath(file)}:${index + 1} interpolates t() into an HTML attribute; use tAttr() so a translation cannot close the tag`,
                );
            }
            if (DOUBLE_ESCAPE.test(line)) {
                findings.push(
                    `${displayPath(file)}:${index + 1} passes an escaped value to tHtml(), which escapes parameters itself`,
                );
            }
        }
    }
}

function checkPluralCategories() {
    for (const [tag, catalog] of catalogs) {
        let rules;
        try {
            rules = new Intl.PluralRules(tag);
        } catch {
            findings.push(`locales.json lists "${tag}", which is not a locale tag Intl accepts`);
            continue;
        }
        const selectable = [...new Set(PLURAL_PROBES.map((count) => rules.select(count)))];
        for (const [key, message] of Object.entries(catalog.messages)) {
            if (typeof message === "string") continue;
            for (const category of Object.keys(message)) {
                if (selectable.includes(category)) continue;
                findings.push(`${tag}.json "${key}": "${category}" is not a plural category ${tag} selects (${selectable.join(", ")})`);
            }
        }
    }
}

/**
 * The first-party surfaces the inventory measures: every page in `static2/` and
 * every non-test module the bundles load.
 * @returns {{htmlFiles: string[], jsFiles: string[]}}
 */
function firstPartyFiles() {
    const htmlFiles = readdirSync(STATIC_DIR, {withFileTypes: true})
        .filter((entry) => entry.isFile() && entry.name.endsWith(".html"))
        .map((entry) => resolve(STATIC_DIR, entry.name));
    const jsFiles = walk(SOURCE_DIR).filter(
        (file) =>
            (file.endsWith(".js") || file.endsWith(".mjs")) &&
            !TEST_FILE.test(file) &&
            !VENDORED_FILES.has(displayPath(file)),
    );
    return {htmlFiles, jsFiles};
}

/** The pages the build bundles, from `entrypoints.txt`. */
function entrypointPaths() {
    if (!existsSync(ENTRYPOINTS_PATH)) return [];
    return readFileSync(ENTRYPOINTS_PATH, "utf8")
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((name) => resolve(SOURCE_DIR, name));
}

/**
 * Candidate counts and the texts behind them, per first-party file. The count is
 * distinct texts within one file: the ratchet asks whether this file gained a
 * literal, and the same word on twenty pages is twenty pages' worth of work.
 * @param {Map<string, Set<string>>} htmlCounts catalog keys per HTML file
 * @param {Map<string, Set<string>>} jsCounts catalog keys per JS file
 */
function buildInventory(htmlCounts, jsCounts) {
    const {htmlFiles, jsFiles} = firstPartyFiles();
    const rows = [];
    const texts = new Set();
    const findings = [];
    let candidates = 0;
    let keyed = 0;
    let remaining = 0;

    for (const [files, isHtml] of [[htmlFiles, true], [jsFiles, false]]) {
        for (const file of files) {
            const source = readFileSync(file, "utf8");
            const scan = isHtml ? scanMarkup(source) : scanCode(source);
            const result = applyMarkers(scan, (literal) => JSON.stringify(literal));
            for (const finding of result.findings) findings.push(`${displayPath(file)} ${finding}`);
            const distinct = new Set(result.candidates.map((candidate) => candidate.text));
            for (const text of distinct) texts.add(text);
            const fileKeys = (isHtml ? htmlCounts : jsCounts).get(file)?.size ?? 0;
            const total = distinct.size;
            // `scanMarkup` leaves keyed text and attributes out of the candidate
            // list already, and `applyMarkers` drops what the file marks
            // not-text, so every surviving candidate is a literal still waiting
            // for one of those two resolutions. `keyed` counts a different unit
            // (catalog keys the file mentions, one attribute each): subtracting
            // it from the candidates reported a page's live literals as zero
            // whenever it carried more keys than leftover text, which is how
            // template.html's five literals read as none.
            rows.push({
                file,
                relative: displayPath(file),
                html: isHtml ? total : 0,
                js: isHtml ? 0 : total,
                total,
                keyed: fileKeys,
                remaining: total,
            });
            candidates += total;
            keyed += fileKeys;
            remaining += total;
        }
    }

    return {rows, findings, htmlCount: htmlFiles.length, jsCount: jsFiles.length, candidates, keyed, remaining, distinct: texts.size};
}

function printInventory(inventory) {
    const rows = inventory.rows
        .filter((row) => row.total > 0 || row.keyed > 0)
        .sort((left, right) => right.remaining - left.remaining || left.relative.localeCompare(right.relative));
    const width = rows.reduce((max, row) => Math.max(max, row.relative.length), "file".length);
    console.log(
        `[i18n] inventory: ${inventory.htmlCount} html + ${inventory.jsCount} js/mjs first-party file(s); ` +
            `${inventory.candidates} candidate(s) in ${rows.filter((row) => row.total > 0).length} file(s), ` +
            `${inventory.distinct} distinct`,
    );
    console.log(`${"file".padEnd(width)}  html    js  total  keyed  remaining`);
    for (const row of rows) {
        console.log(
            `${row.relative.padEnd(width)}  ${String(row.html).padStart(4)}  ${String(row.js).padStart(4)}` +
                `  ${String(row.total).padStart(5)}  ${String(row.keyed).padStart(5)}  ${String(row.remaining).padStart(9)}`,
        );
    }
    console.log(
        `[i18n] inventory: ${inventory.candidates} candidate(s) (heuristic; candidates, not facts), ` +
            `${inventory.keyed} keyed, ${inventory.remaining} remaining`,
    );
    for (const finding of inventory.findings) console.log(`[i18n] note: ${finding}`);
}

function serializeBoard(board) {
    const keys = [...board.keys()].sort();
    const lines = keys.map((key, index) => {
        const entry = board.get(key);
        const fields = [`"status": ${JSON.stringify(entry.status)}`, `"expected": ${entry.expected}`];
        if (Number.isInteger(entry.candidates)) fields.push(`"candidates": ${entry.candidates}`);
        return `  ${JSON.stringify(key)}: {${fields.join(", ")}}${index === keys.length - 1 ? "" : ","}`;
    });
    return `{\n${lines.join("\n")}\n}\n`;
}

/**
 * Rewrites `migration.json` from the measurement. A row's `candidates` is the
 * ceiling the gate enforces, so refreshing it is how the ratchet moves: down as
 * a unit translates, up only when the board is deliberately rewritten. Existing
 * rows keep `status` and `expected` — a done row still states the key count it
 * must hit — and new rows start `pending` with the measured count as their
 * target.
 */
function writeBoard(inventory) {
    const previous = existsSync(MIGRATION_PATH) ? readJson(MIGRATION_PATH) : {};
    const entrypoints = new Set(entrypointPaths().map((file) => displayPath(file)));
    const board = new Map();
    const added = [];
    const raised = [];
    for (const row of inventory.rows) {
        const isPage = row.relative.startsWith("static2/");
        // Every row the gate can require must be writable: pages, bundle
        // entrypoints, and any module the inventory reports (candidates or keys).
        if (!isPage && !entrypoints.has(row.relative) && row.total === 0 && row.keyed === 0) continue;
        const before = previous[row.relative];
        if (before) {
            // Keep the row's target, but never leave it below what the file
            // already keys: a pending row whose target is stale would fail the
            // gate for a reason the writer created. A `done` row is left alone —
            // the gate's own message says what it must be.
            let expected = before.expected;
            if (before.status !== "done" && Number.isInteger(expected) && row.keyed > expected) {
                raised.push(`${row.relative} target ${expected} → ${row.keyed} (the file already keys that many)`);
                expected = row.keyed;
            }
            board.set(row.relative, {status: before.status, expected, candidates: row.total});
        } else {
            // A new row starts pending with the measured candidate count as its
            // target. A file that already keys strings before it has a row (the
            // standalone pages) keeps a target at least as large as its keys, or
            // the existing "keys past the target" gate would fail on day one.
            const target = Math.max(row.total, row.keyed);
            board.set(row.relative, {status: "pending", expected: target, candidates: row.total});
            added.push(row.relative);
        }
    }
    const dropped = [];
    for (const [file, entry] of Object.entries(previous)) {
        if (board.has(file)) continue;
        if (!existsSync(resolve(ROOT, file))) {
            dropped.push(file);
            continue;
        }
        board.set(file, {status: entry.status, expected: entry.expected, candidates: entry.candidates});
    }
    writeFileSync(MIGRATION_PATH, serializeBoard(board), "utf8");
    console.log(`[i18n] board: ${board.size} row(s) written to ${displayPath(MIGRATION_PATH)}`);
    for (const file of added) console.log(`[i18n] board: new row ${file}`);
    for (const line of raised) console.log(`[i18n] board: raised ${line}`);
    for (const file of dropped) console.log(`[i18n] board: dropped row ${file} (no such file)`);
}

function checkMigration(htmlCounts, jsCounts, inventory) {
    const migration = readJson(MIGRATION_PATH);
    const measured = new Map(inventory.rows.map((row) => [row.relative, row]));
    const rows = [];
    for (const [file, entry] of Object.entries(migration)) {
        if (!entry || typeof entry !== "object") {
            findings.push(`migration.json: "${file}" must map to {status, expected}`);
            continue;
        }
        if (!MIGRATION_STATUSES.has(entry.status)) {
            findings.push(`migration.json: "${file}" has unknown status "${entry.status}"`);
        }
        if (!Number.isInteger(entry.expected) || entry.expected < 0) {
            findings.push(`migration.json: "${file}" needs a non-negative integer "expected" count`);
        }
        const counts = file.startsWith("static2/") ? htmlCounts : jsCounts;
        const keyed = counts.get(resolve(ROOT, file))?.size ?? 0;
        // A row is a target until it is done: more keys than the target means the
        // target was never raised, and a done row must key exactly its target.
        if (Number.isInteger(entry.expected)) {
            if (keyed > entry.expected) {
                findings.push(`migration.json: "${file}" has ${keyed} catalog keys but targets ${entry.expected}`);
            } else if (entry.status === "done" && keyed !== entry.expected) {
                findings.push(`migration.json: "${file}" is marked done but only ${keyed} of ${entry.expected} keys are migrated`);
            }
        }
        const row = measured.get(file);
        if (!row) {
            findings.push(`migration.json: "${file}" names no first-party page or module`);
        } else if (entry.candidates === undefined) {
            if (row.total > 0) {
                findings.push(`migration.json: "${file}" has ${row.total} candidate(s) and no recorded count; run --write-board`);
            }
        } else if (!Number.isInteger(entry.candidates) || entry.candidates < 0) {
            findings.push(`migration.json: "${file}" needs a non-negative integer "candidates" count`);
        } else if (row.total > entry.candidates) {
            findings.push(
                `migration.json: "${file}" has ${row.total} candidate(s) but the board records ${entry.candidates}; a unit may not add untranslated literals`,
            );
        }
        rows.push(`[i18n] migration: ${file} ${entry.status} — ${keyed} keyed / ${entry.expected} expected`);
    }
    // Every file the inventory reports needs a row: with candidates it would be
    // translated without a target to prove against, and with keys but no row the
    // "all rows done" claim counts files that are not on the board at all.
    for (const row of inventory.rows) {
        if (row.total === 0 && row.keyed === 0) continue;
        if (Object.prototype.hasOwnProperty.call(migration, row.relative)) continue;
        findings.push(
            `${row.relative} has ${row.total} candidate(s) and ${row.keyed} key(s) and no migration.json row; run --write-board`,
        );
    }
    // A pending or in-progress row is work the ledger still claims. The
    // acceptance reads "every row done", so a status that is not done has to
    // fail here instead of being noticed by a human reading the file.
    for (const [file, entry] of Object.entries(migration)) {
        if (entry?.status === "done") continue;
        findings.push(
            `migration.json: "${file}" is ${JSON.stringify(entry?.status)}, not done; the board records work that is still outstanding`,
        );
    }
    return rows;
}

function printTable(htmlCounts, jsCounts) {
    const rows = [];
    for (const [file, keys] of htmlCounts) rows.push([displayPath(file), keys.size, 0]);
    for (const [file, keys] of jsCounts) rows.push([displayPath(file), 0, keys.size]);
    if (rows.length === 0) {
        console.log("[i18n] no catalog keys in the UI yet");
        return;
    }
    rows.sort((left, right) => left[0].localeCompare(right[0]));
    const width = rows.reduce((max, [file]) => Math.max(max, file.length), "file".length);
    console.log(`${"file".padEnd(width)}  html    js`);
    for (const [file, html, js] of rows) {
        console.log(`${file.padEnd(width)}  ${String(html).padStart(4)}  ${String(js).padStart(4)}`);
    }
}

function main() {
    configure();
    const locales = readJson(resolve(CATALOG_DIR, "locales.json"));
    if (!Array.isArray(locales) || locales.length === 0) fail("static2/i18n/locales.json: expected a non-empty array");
    loadCatalogs(locales);
    const seeds = readSeeds();

    const htmlFiles = walk(STATIC_DIR).filter((file) => file.endsWith(".html"));
    const jsFiles = walk(SOURCE_DIR).filter((file) => file.endsWith(".js") || file.endsWith(".mjs"));
    // Test files assert on keys; they do not render them.
    const uiFiles = jsFiles.filter((file) => !TEST_FILE.test(file));
    // The Rust emitters: `template.rs` writes markup the browser sees but no
    // static2/js_build file mentions.
    const rustFiles = walk(ROOT).filter((file) => file.endsWith(".rs"));
    const htmlCounts = scanFiles(htmlFiles, HTML_KEY);
    const jsCounts = scanFiles(uiFiles, JS_KEY, 2);
    const rustCounts = scanFiles(rustFiles, RUST_KEY);

    const used = new Set();
    const usedIn = new Map();
    for (const counts of [htmlCounts, jsCounts, rustCounts]) {
        for (const [file, keys] of counts) {
            for (const key of keys) {
                used.add(key);
                if (!usedIn.has(key)) usedIn.set(key, file);
            }
        }
    }

    console.log(
        `[i18n] ${htmlFiles.length} html + ${uiFiles.length} js/mjs + ${rustFiles.length} rust files scanned` +
        ` (${jsFiles.length - uiFiles.length} test file(s) excluded); ${used.size} distinct keys used`,
    );

    const inventory = buildInventory(htmlCounts, jsCounts);
    for (const finding of inventory.findings) findings.push(finding);

    if (INVENTORY || WRITE_BOARD) {
        if (INVENTORY) {
            printInventory(inventory);
            // The numbers this mode prints are only meaningful against the
            // recorded ceiling, so an inventory run checks the board too — that
            // is what makes `--inventory --check` a gate instead of a report.
            checkMigration(htmlCounts, jsCounts, inventory);
        }
        if (WRITE_BOARD) writeBoard(inventory);
        report();
        return;
    }

    printTable(htmlCounts, jsCounts);
    for (const row of checkMigration(htmlCounts, jsCounts, inventory)) console.log(row);
    printSeeds(seeds);

    checkUsage(used, usedIn, seeds);
    checkPlaceholders();
    printCallParameters(checkCallParameters(uiFiles));
    checkRenderContexts(uiFiles);
    checkPluralCategories();
    report();
}

/**
 * Prints the findings, and under `--check` fails on them. Without it the same
 * lines stay a report, so a mid-migration tree is readable while the gate is
 * only as strong as the command that runs it. Every mode ends here: an
 * `--inventory` run used to return before this point, so the command could not
 * fail even when it found an untranslated literal or the extractor crashed.
 */
function report() {
    if (findings.length === 0) {
        const catalogKeys = Object.keys(englishMessages()).length;
        console.log(`[i18n] check: OK — ${catalogKeys} catalog keys in ${catalogs.size} locale(s)`);
        return;
    }
    for (const finding of findings) console.log(`[i18n] ${CHECK ? "FAIL" : "note"}: ${finding}`);
    if (CHECK) process.exit(1);
    console.log(`[i18n] ${findings.length} finding(s) reported; rerun with --check to fail`);
}

main();
