// Catalog contract for the shipped locales: `es.json` must actually carry
// Spanish.
//
// The extractor closes key-set and placeholder parity, and the contracts for the
// shell and the standalone pages check "differs from English" for the keys their
// screens render. That leaves every other message: a copy-paste that lands an
// English string in `es.json` is invisible to all of them. This file checks the
// whole catalog, variant by variant, so a new key cannot ship untranslated, and
// bans a retired page label in all three shipped catalogs and in the inline
// fallbacks that mirror them.

import assert from "node:assert/strict";
import {readFileSync, readdirSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t} from "./index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");
const STATIC_DIR = resolve(HERE, "../../../static2");
const STATIC_PAGES = readdirSync(STATIC_DIR).filter((name) => name.endsWith(".html")).sort();

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));
const PT = JSON.parse(readFileSync(resolve(CATALOG_DIR, "pt-BR.json"), "utf8"));

/**
 * The minimum `index.js` and `locale.mjs` need: the catalog globals, the
 * document's language and a cookie jar. The interpolated sentences below are
 * asserted through the shipped runtime, not through a local substitution: a
 * mirror cannot see Intl grouping, a plural rule or an escaping change.
 */
function installBrowser(catalog, lang) {
    globalThis.window = {LQOS_I18N_EN: EN};
    if (lang !== "en") globalThis.window.LQOS_I18N = catalog;
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang}, cookie: ""};
}

test.afterEach(() => {
    delete globalThis.window;
    delete globalThis.localStorage;
    delete globalThis.document;
});
// The categories this locale's CLDR rules can select. `i18n/GLOSSARY.md` asks
// `es` for an explicit `many` on top of the fallback's one/other; a category
// outside this set is dead text and fails below.
const SELECTABLE = new Set(new Intl.PluralRules(ES.locale).resolvedOptions().pluralCategories);

// Values that are identical in both locales on purpose, each with the reason.
// The list can only shrink: a row that stops matching fails, so translating one
// of these and leaving the row behind costs an edit.
const IDENTICAL_BY_DESIGN = new Map([
    ["common.display.klingon", '"Klingon" is the proper name the English menu shows too'],
    ["common.footer.copyright", "a proper name and the year prefix; the year itself is written at load time"],
    ["common.worker", "GLOSSARY §2 keeps `worker` untranslated: the badge and the CPU-worker fact label are the same word in Spanish"],
    ["common.table.rtt_ms", "an acronym plus its unit: GLOSSARY §6 keeps `RTT` and `ms` literal, so the header reads the same in both locales"],
    ["circuit.rtt.internet", "`Internet` is the same proper noun in Spanish; the three RTT legs are the operator's own names"],
    ["circuit.devices.rtt_p50_p95", "two RTT percentile names and the separator between them; the digits and `RTT` are §6 literals"],
    ["topology_manager.role.manual", "`Manual` is the same word in Spanish, and it is the attachment-role token the topology source emits"],
    ["topology_probes.load_state.error", "`Error` is the same word in Spanish; the status badge names the failed load, not a translated concept"],
    ["tree.limit.kind.override", "GLOSSARY §2 keeps `override` untranslated: it labels the rate that prevails over the base data"],
    ["cpu_workers.cpu_unknown", "an unknown CPU id has no name to translate; the placeholder reads the same in both locales"],
    ["cpu_workers.value.no", "`no` is the same word in Spanish; the cell is a boolean reading, not prose"],
    ["cpu_workers.phase.tx", "`TX` is a transport abbreviation GLOSSARY §6 keeps literal; it names a bridge phase"],
    ["scheduler.degraded.detail.combined", "GLOSSARY §3 keeps the middle dot the source uses between the two axes; both sentences either side are translated"],
    ["config.general.qoo.table.id", "`ID` is the same abbreviation in Spanish; it heads the column of profile ids the server emits, which stay literal"],
    ["config.common.menu.general", "`General` is the same word in Spanish; the config menu names the general settings section, and no shorter synonym fits the row"],
    ["config.ssl.value.no", "`No` is the same word in Spanish; the row reports whether Caddy is installed, not prose"],
    ["config.integration.status.ingest", "the line names the watched `libreqos_network` directory and inserts the server's own state token, so only the punctuation around them is ours"],
    ["site_map.popup.rtt", "`RTT` is the technical name GLOSSARY §2 keeps literal; the marker popup adds only the measurement and its colon"],
    ["asn_analysis.label_with_asn", "`ASN` is the acronym GLOSSARY §2 keeps literal and the parenthesis is punctuation, so the name-then-number frame reads the same in both locales"],
    ["executive.col.asn", "`ASN` is the acronym GLOSSARY §2 keeps literal, so the leaderboard's column header reads the same in both locales"],
    ["executive.leaderboard.asn_number", "the acronym, the server's ASN number and the parentheses around them are all GLOSSARY §2/§6 literals, so the frame reads the same"],
    ["executive.panel.rtt_label", "`RTT` is the acronym GLOSSARY §2 keeps literal and p50/p90 are the percentile names the chart prints, so the Global Heatmap row label reads the same"],
    ["dashlet.stormguard.table.rtt", "`RTT` is the acronym GLOSSARY §2 keeps literal, so the site-detail table row label reads the same in both locales"],
    ["trial.error.title", "`Error` is the same word in Spanish; the trial page modal title names the failed signup or licence step"],
    ["common.severity.error", "`Error` is the same word in Spanish; the severity badge names the server's error level, not a translated concept"],
    ["common.table.total", "`Total` is the same word in Spanish, and it heads the top-flows column that sums the two directions"],
    ["common.table.rtt_dl_arrow", "an arrow and the RTT acronym: GLOSSARY §2 keeps `RTT` literal, so the download column header reads the same in both locales"],
    ["common.table.rtt_ul_arrow", "an arrow and the RTT acronym: GLOSSARY §2 keeps `RTT` literal, so the upload column header reads the same in both locales"],
    ["help.chat.title", "`Chat` is the loanword Spanish uses for the chat card, so the card title reads the same in both locales"],
    ["ethernet_caps.column.ethernet", "Ethernet is the technology name GLOSSARY §6 keeps literal; the tier filter and the table column name the same port speed in both locales"],
    ["stormguard_debug.status.error", "`Error` is the same word in Spanish; the badge names the websocket failure, not a translated concept"],
    ["asn_explorer.col.bytes", "S4 Oracle: `Bytes` is the unit name the column heads in both locales, so the Spanish header is the English word with its capital"],
    ["graph.axis.bytes", "S4 Oracle: `Bytes` names the unit at the start of an axis label in both locales; the axis measures the same quantity"],
]);

/** `[variant, text]` pairs: one entry for a string, one per plural category. */
function variants(message) {
    return typeof message === "string" ? [["", message]] : Object.entries(message);
}

test("every Spanish message differs from the English source", () => {
    const pending = new Map(IDENTICAL_BY_DESIGN);
    const englishKeys = Object.keys(EN.messages);
    assert.ok(englishKeys.length > 0, "en.json must define messages");

    for (const [key, source] of Object.entries(EN.messages)) {
        const translated = ES.messages[key];
        assert.ok(translated !== undefined, `es.json is missing "${key}"`);
        assert.equal(typeof translated, typeof source, `es.json "${key}" must keep the en.json message shape`);
        const spanish = new Map(variants(translated));
        for (const [category, text] of variants(source)) {
            const label = category === "" ? key : `${key}.${category}`;
            const candidate = spanish.get(category);
            assert.equal(typeof candidate, "string", `es.json is missing "${label}"`);
            if (candidate !== text) continue;
            assert.ok(pending.has(key), `es.json "${label}" still carries the English text`);
            pending.delete(key);
        }
        // An extra variant is allowed only when this locale can select it: the
        // policy adds `many` to `es` while `en` merges it into "other". A
        // category Spanish CLDR never selects stays a finding.
        for (const category of spanish.keys()) {
            const fellBack = variants(source).some(([name]) => name === category);
            assert.ok(
                fellBack || SELECTABLE.has(category),
                `es.json "${key}" has a "${category}" variant neither en.json nor ${ES.locale} CLDR defines`,
            );
        }
    }

    // A row that no longer matches is stale: the value was translated and the
    // row should have been deleted with it.
    assert.deepEqual(
        [...pending.keys()],
        [],
        "these IDENTICAL_BY_DESIGN rows no longer match and must be deleted",
    );
});

test("every Spanish plural message defines the categories Spanish selects", () => {
    // `i18n/GLOSSARY.md` §5: a plural message carries one, many and other, so the
    // policy stays visible instead of leaning on the runtime's `other` fallback.
    const plurals = Object.entries(ES.messages).filter(([, message]) => typeof message !== "string");
    assert.ok(plurals.length > 0, "es.json must define at least one plural message");
    for (const [key, message] of plurals) {
        for (const category of SELECTABLE) {
            assert.equal(typeof message[category], "string", `es.json "${key}" is missing "${category}"`);
        }
    }
});

// Interpolated sentences are the catalog's riskiest text: the fixed words are
// translated once, but a count or a field label arrives at render time. These
// guards pin the two properties the S3 review established, so an edit that
// restores a bare `{field}` or puts a count in front of a translated plural noun
// fails here rather than in an alert nobody reads. The sentences are rendered
// by the shipped `t()`, so the digits are Intl's: a local `String(params[name])`
// mirror (what this file used to keep) could not observe grouping or a plural.
const COUNT_MESSAGES = [
    "config.devices.showing_matching",
    "config.devices.showing_all",
];
const FIELD_MESSAGES = [
    "config.stormguard.validation.must_be",
    "config.stormguard.validation.between_1_100",
    "config.stormguard.validation.between_0_100",
    "config.stormguard.validation.between_0_and_1",
    "config.stormguard.validation.not_empty",
    "config.lts.validation.period_invalid",
];

test("count summaries read for zero, one and several", () => {
    const cases = [
        {start: 0, end: 0, total: 0, circuits: 0},
        {start: 1, end: 1, total: 1, circuits: 1},
        {start: 11, end: 20, total: 57, circuits: 3},
        // Thousands exercise Intl grouping: the digit separator is the locale's,
        // and no hand-built string can stand in for it.
        {start: 1, end: 20, total: 12_345, circuits: 3},
    ];
    // Both translated catalogs take the same guard, so a count that leads a
    // plural noun fails in either language.
    for (const [lang, catalog] of [["es", ES], ["pt-BR", PT]]) {
        installBrowser(catalog, lang);
        for (const key of COUNT_MESSAGES) {
            for (const params of cases) {
                const text = t(key, params);
                assert.doesNotMatch(text, /\{[a-z_]+\}/, `${lang}.json "${key}" left a placeholder unfilled`);
                // Labelling the count first is what keeps `1` away from a plural
                // noun ("1 dispositivos"): the number must not lead one.
                assert.doesNotMatch(
                    text,
                    /\b1\s+(?:dispositivos|circuitos)\b/,
                    `${lang}.json "${key}" puts a bare singular count in front of a plural noun`,
                );
            }
            const summary = t(key, {start: 1, end: 20, total: 12_345, circuits: 3});
            assert.match(summary, /12\.345/, `${lang}.json "${key}" must group the count by the active locale`);
            // The branch really rendered its own catalog: each locale has its
            // own verb for these sentences, so a branch that silently kept the
            // other catalog fails here instead of passing on the shared words.
            assert.match(
                summary,
                lang === "pt-BR" ? /Exibindo/ : /Mostrando/,
                `${lang}.json "${key}" must render its own sentence`,
            );
        }
    }
});

test("validation sentences keep the field label inside a quoted frame", () => {
    // The two genders the frame has to absorb, taken from labels the StormGuard
    // and LTS alerts actually interpolate: "Tasa" is feminine, "Objetivo" is
    // masculine. A sentence that starts with `{field}` cannot carry both.
    const labels = [
        ES.messages["config.stormguard.delay.min_throughput"],
        ES.messages["config.stormguard.active_ping.target"],
    ];
    installBrowser(ES, "es");
    for (const key of FIELD_MESSAGES) {
        for (const label of labels) {
            const text = t(key, {field: label, relation: "mayor que 0"});
            assert.doesNotMatch(text, /\{[a-z_]+\}/, `es.json "${key}" left a placeholder unfilled`);
            assert.match(
                text,
                /^(?:El valor de|El campo) «[^»]+» (?:no )?debe/,
                `es.json "${key}" must frame the label with a noun so either gender reads`,
            );
            assert.equal(
                text.split(`«${label}»`).length - 1,
                1,
                `es.json "${key}" must quote "${label}"`,
            );
            assert.equal(
                text.split(label).length - 1,
                1,
                `es.json "${key}" repeats "${label}" outside its quotes`,
            );
        }
    }
});

// Commit 8942830e2 renamed the page to "Data Sources". The catalog is what the
// page renders, and the static2 pages that carry an inline fallback for these
// keys mirror `en.json` word for word, so the retired label is banned in the
// catalogs and in every static2 page, and the fallbacks are compared: a
// fallback that drifted back would otherwise stay invisible until the catalog
// failed to load.
const RETIRED_PAGE_LABEL_DASHED = /integra(?:tion|ci[oó]n|[cç][aã]o)\s*[-–—]\s*com(?:mon|[uú]n|um)/i;
const RETIRED_PAGE_LABEL_SPACED = /integra(?:tion|ci[oó]n|[cç][aã]o)\s+(com(?:mon|un|ún|um))/i;
// The only lexemes the space form may carry: the label is a name, so it is
// capitalised, while ordinary prose ("uma integração comum a todos") is not.
const RETIRED_LEXEMES = new Set(["Common", "Comun", "Común", "Comum"]);

/**
 * Whether `text` carries the retired page label in either shape it shipped in:
 * the dash form in any case ("Integration - Common", "Integração-Comum",
 * "Integración – común"), or the space form with the label's own capitalised
 * lexeme. "Integration (Common)" would slip through; no catalog or page ever
 * wrote that shape, and the dash form covers every variant the rename produced.
 */
function retiredPageName(text) {
    if (RETIRED_PAGE_LABEL_DASHED.test(text)) return true;
    const spaced = text.match(RETIRED_PAGE_LABEL_SPACED);
    return Boolean(spaced) && RETIRED_LEXEMES.has(spaced[1]);
}

// [page, key, tag] for every inline fallback that names the page. These four
// are the complete set for this class: every static2 element whose fallback
// text carries the page name. The two compile_mode_help paragraphs are empty in
// their pages (their bundles render them from the catalog), so they have no
// fallback to compare.
const PAGE_NAME_FALLBACKS = [
    ["config_uisp.html", "config.uisp.subtitle", "div"],
    ["config_splynx.html", "config.splynx.subtitle", "div"],
    ["setup_runtime.html", "setup_runtime.custom.step_integration", "li"],
    ["setup_runtime.html", "config.common.menu.integration", "a"],
];

test("no shipped catalog names the retired page, and its inline fallbacks mirror en.json", () => {
    for (const [tag, catalog] of [["en", EN], ["es", ES], ["pt-BR", PT]]) {
        for (const [key, message] of Object.entries(catalog.messages)) {
            for (const [category, text] of variants(message)) {
                const label = category === "" ? key : `${key}.${category}`;
                assert.ok(
                    !retiredPageName(text),
                    `${tag}.json "${label}" still names the retired page`,
                );
            }
        }
    }
    for (const page of STATIC_PAGES) {
        assert.ok(
            !retiredPageName(readFileSync(resolve(STATIC_DIR, page), "utf8")),
            `static2/${page} still names the retired page`,
        );
    }
    for (const [page, key, tag] of PAGE_NAME_FALLBACKS) {
        const html = readFileSync(resolve(STATIC_DIR, page), "utf8");
        const element = new RegExp(
            `<${tag}\\b[^>]*data-i18n="${key.replaceAll(".", "\\.")}"[^>]*>([\\s\\S]*?)</${tag}>`,
        );
        const found = html.match(element);
        assert.ok(found, `static2/${page} must carry a <${tag}> fallback for ${key}`);
        const text = found[1].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim();
        assert.ok(text, `static2/${page} "${key}" must carry the inline fallback text`);
        assert.equal(text, EN.messages[key], `static2/${page} "${key}" must mirror en.json`);
    }
});
