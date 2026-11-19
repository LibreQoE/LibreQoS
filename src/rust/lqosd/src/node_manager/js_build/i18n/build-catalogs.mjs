#!/usr/bin/env node
// Compiles static2/i18n/*.json into classic scripts under js_build/out/i18n/.
// `js_build/esbuild.sh` runs this after the bundle loop, so a page can load
// `i18n/<tag>.js` as a plain <script> before its own bundle.
//
// The fallback locale's script publishes `window.LQOS_I18N_EN` and every other
// catalog publishes `window.LQOS_I18N`, so the runtime can name the fallback
// instead of inferring it from whichever script the page emitted last.
//
// Also emits the en-XA pseudo-locale from en.json: accented vowels, ~40% longer
// text in brackets. en-XA is only an overflow-check tool — it is never listed in
// locales.json and never served to operators.
//
// `out/i18n/locales.js` carries the same list for the language selector the
// sidebar and the standalone pages share, which must not hardcode the tags a
// third time.
//
// `--catalogs <dir>` and `--out <dir>` override the source and output trees; the
// defaults are the ones the build uses, and the tests point them at fixtures.

import {mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {fileURLToPath} from "node:url";
import {FALLBACK_LOCALE} from "../src/i18n/locale.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const JS_BUILD_DIR = resolve(HERE, "..");
const CATALOG_DIR = resolve(JS_BUILD_DIR, "../static2/i18n");
const OUTPUT_DIR = resolve(JS_BUILD_DIR, "out/i18n");

const PSEUDO_LOCALE = "en-XA";
const PSEUDO_EXPANSION = 0.4;
const PSEUDO_PADDING = "·";
const LOCALE_TAG = /^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$/;
const MESSAGE_KEY = /^[a-z][A-Za-z0-9_]*(\.[A-Za-z0-9_]+)+$/;
const PLACEHOLDER = /\{([^}]*)\}/g;
const PLACEHOLDER_NAME = /^[A-Za-z][A-Za-z0-9_]*$/;
const PLURAL_CATEGORIES = new Set(["zero", "one", "two", "few", "many", "other"]);
const ACCENTED_VOWELS = {a: "á", e: "é", i: "í", o: "ó", u: "ú", A: "Á", E: "É", I: "Í", O: "Ó", U: "Ú"};
// Placeholders and markup stay literal in the pseudo-locale, otherwise the
// overflow check would exercise a broken render path instead of the real one.
const PSEUDO_LITERAL = /\{[^}]*\}|<[^>]*>/g;

function fail(message) {
    throw new Error(message);
}

/** The value of a `--flag <path>` argument, resolved against the working directory. */
function flagPath(name, fallback) {
    const index = process.argv.indexOf(`--${name}`);
    if (index === -1) return fallback;
    const value = process.argv[index + 1];
    if (!value || value.startsWith("--")) fail(`--${name} needs a directory`);
    return resolve(value);
}

function readJson(path) {
    let text;
    try {
        text = readFileSync(path, "utf8");
    } catch (error) {
        fail(`cannot read ${path}: ${error.message}`);
    }
    try {
        return JSON.parse(text);
    } catch (error) {
        fail(`invalid JSON in ${path}: ${error.message}`);
    }
}

function readLocales(catalogDir) {
    const path = resolve(catalogDir, "locales.json");
    const locales = readJson(path);
    if (!Array.isArray(locales) || locales.length === 0) {
        fail(`${path}: expected a non-empty array of {tag, name, dir}`);
    }
    const seen = new Set();
    for (const locale of locales) {
        if (!locale || typeof locale !== "object" || Array.isArray(locale)) {
            fail(`${path}: every entry must be an object`);
        }
        if (Object.keys(locale).sort().join(",") !== "dir,name,tag") {
            fail(`${path}: every entry needs exactly "tag", "name" and "dir"`);
        }
        if (!LOCALE_TAG.test(locale.tag)) {
            fail(`${path}: invalid locale tag "${locale.tag}"`);
        }
        if (locale.tag === PSEUDO_LOCALE) {
            fail(`${path}: ${PSEUDO_LOCALE} is a generated pseudo-locale for overflow checks, never a listed locale`);
        }
        if (typeof locale.name !== "string" || !locale.name.trim()) {
            fail(`${path}: "${locale.tag}" needs its native name`);
        }
        if (locale.dir !== "ltr" && locale.dir !== "rtl") {
            fail(`${path}: "${locale.tag}" dir must be "ltr" or "rtl"`);
        }
        if (seen.has(locale.tag)) {
            fail(`${path}: duplicate locale tag "${locale.tag}"`);
        }
        seen.add(locale.tag);
    }
    if (!seen.has(FALLBACK_LOCALE)) {
        fail(`${path}: the fallback locale "${FALLBACK_LOCALE}" must be listed`);
    }
    return locales;
}

function validatePlaceholders(path, label, text) {
    for (const match of text.matchAll(PLACEHOLDER)) {
        if (!PLACEHOLDER_NAME.test(match[1])) {
            fail(`${path}: "${label}" has invalid placeholder "${match[0]}" (use {lowerCamel})`);
        }
    }
}

function validateMessage(path, key, message) {
    if (!MESSAGE_KEY.test(key)) {
        fail(`${path}: key "${key}" must be dotted segments such as common.action.save`);
    }
    if (typeof message === "string") {
        validatePlaceholders(path, key, message);
        return;
    }
    if (!message || typeof message !== "object" || Array.isArray(message)) {
        fail(`${path}: "${key}" must be a string or a plural object`);
    }
    if (!Object.prototype.hasOwnProperty.call(message, "other")) {
        fail(`${path}: "${key}" plural object needs an "other" variant`);
    }
    for (const [category, text] of Object.entries(message)) {
        if (!PLURAL_CATEGORIES.has(category)) {
            fail(`${path}: "${key}" has unknown plural category "${category}"`);
        }
        if (typeof text !== "string") {
            fail(`${path}: "${key}.${category}" must be a string`);
        }
        validatePlaceholders(path, `${key}.${category}`, text);
    }
}

function readCatalog(tag, dir, catalogDir) {
    const path = resolve(catalogDir, `${tag}.json`);
    const catalog = readJson(path);
    if (!catalog || typeof catalog !== "object" || Array.isArray(catalog)) {
        fail(`${path}: expected a catalog object with "locale", "dir" and "messages"`);
    }
    if (catalog.locale !== tag) {
        fail(`${path}: "locale" must be "${tag}", found ${JSON.stringify(catalog.locale)}`);
    }
    if (catalog.dir !== dir) {
        fail(`${path}: "dir" must be "${dir}" (from locales.json), found ${JSON.stringify(catalog.dir)}`);
    }
    const messages = catalog.messages;
    if (!messages || typeof messages !== "object" || Array.isArray(messages)) {
        fail(`${path}: "messages" must be an object`);
    }
    if (Object.keys(messages).length === 0) {
        fail(`${path}: "messages" is empty`);
    }
    for (const [key, message] of Object.entries(messages)) {
        validateMessage(path, key, message);
    }
    return catalog;
}

function accent(text) {
    return text.replace(/[aeiouAEIOU]/g, (vowel) => ACCENTED_VOWELS[vowel]);
}

function pseudoText(text) {
    let accented = "";
    let cursor = 0;
    for (const match of text.matchAll(PSEUDO_LITERAL)) {
        accented += accent(text.slice(cursor, match.index)) + match[0];
        cursor = match.index + match[0].length;
    }
    accented += accent(text.slice(cursor));
    return `[${accented}${PSEUDO_PADDING.repeat(Math.ceil(accented.length * PSEUDO_EXPANSION))}]`;
}

function pseudoCatalog(english) {
    const messages = {};
    for (const [key, message] of Object.entries(english.messages)) {
        messages[key] = typeof message === "string"
            ? pseudoText(message)
            : Object.fromEntries(Object.entries(message).map(([category, text]) => [category, pseudoText(text)]));
    }
    return {locale: PSEUDO_LOCALE, dir: english.dir, messages};
}

function catalogScript(catalog, sourceName) {
    // The fallback gets its own global, so a page that emitted it after the
    // served catalog would leave the served catalog in `LQOS_I18N` instead of
    // renaming it. The runtime reads the active catalog as
    // `LQOS_I18N ?? LQOS_I18N_EN`.
    const isFallback = catalog.locale === FALLBACK_LOCALE;
    const lines = [
        `// Generated by js_build/i18n/build-catalogs.mjs from ${sourceName}.`,
        "// Edit the JSON, not this file.",
        isFallback
            ? "// The fallback catalog: every page loads this one."
            : "// The served catalog: the page loads the fallback catalog first.",
        `window.${isFallback ? "LQOS_I18N_EN" : "LQOS_I18N"} = ${JSON.stringify(catalog)};`,
    ];
    return `${lines.join("\n")}\n`;
}

function ensureEveryCatalogIsListed(locales, catalogDir) {
    const listed = new Set(locales.map((locale) => locale.tag));
    for (const name of readdirSync(catalogDir)) {
        if (!name.endsWith(".json") || name === "locales.json") continue;
        const tag = name.slice(0, -".json".length);
        if (!listed.has(tag)) {
            fail(`${resolve(catalogDir, name)}: locale "${tag}" is not listed in locales.json`);
        }
    }
}

function build() {
    const catalogDir = flagPath("catalogs", CATALOG_DIR);
    const outputDir = flagPath("out", OUTPUT_DIR);
    const locales = readLocales(catalogDir);
    ensureEveryCatalogIsListed(locales, catalogDir);
    const catalogs = locales.map((locale) => [locale, readCatalog(locale.tag, locale.dir, catalogDir)]);
    const english = catalogs.find(([locale]) => locale.tag === FALLBACK_LOCALE)[1];

    mkdirSync(outputDir, {recursive: true});
    for (const name of readdirSync(outputDir)) {
        if (name.endsWith(".js")) rmSync(resolve(outputDir, name));
    }

    for (const [locale, catalog] of catalogs) {
        const source = `static2/i18n/${locale.tag}.json`;
        writeFileSync(resolve(outputDir, `${locale.tag}.js`), catalogScript(catalog, source), "utf8");
        console.log(`[i18n] ${locale.tag}: ${Object.keys(catalog.messages).length} messages`);
    }

    writeFileSync(
        resolve(outputDir, `${PSEUDO_LOCALE}.js`),
        catalogScript(pseudoCatalog(english), "static2/i18n/en.json"),
        "utf8",
    );
    console.log(`[i18n] ${PSEUDO_LOCALE}: pseudo-locale for overflow checks`);

    // The switcher's only source of locale tags and native names.
    const switcherLocales = locales.map(({tag, name, dir}) => ({tag, name, dir}));
    writeFileSync(
        resolve(outputDir, "locales.js"),
        [
            "// Generated by js_build/i18n/build-catalogs.mjs from static2/i18n/locales.json.",
            "// Edit the JSON, not this file.",
            `window.LQOS_I18N_LOCALES = ${JSON.stringify(switcherLocales)};`,
            "",
        ].join("\n"),
        "utf8",
    );
    console.log(`[i18n] locales: ${switcherLocales.length} locale(s) for the switcher`);
}

try {
    build();
} catch (error) {
    console.error(`[i18n] build-catalogs: ${error.message}`);
    process.exit(1);
}
