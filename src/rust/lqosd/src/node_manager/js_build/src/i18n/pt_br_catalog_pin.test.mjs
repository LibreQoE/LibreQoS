// Catalog pin for the shipped Brazilian Portuguese catalog.
//
// The pt-BR campaign translated the node_manager UI one page family at a time:
// unit 2 took the shared shell, unit 3 the `config.*` pages, and unit 4 the
// remaining families, which completes the catalog. The extractor and the build
// contract already force key-set, message kind and placeholder parity, which
// leaves the property only a catalog can carry: a value must actually be
// Portuguese. This file pins that twice — no message may hold the English text
// unless it is on the closed IDENTICAL_BY_DESIGN list, and every namespace in
// the catalog must be declared translated, so a page family added later fails
// here until it ships a catalog too.
//
// The namespace table is exhaustive on purpose: a new namespace lands outside
// the declared set and fails the table test, and its English keys would fail the
// identity test as well.
//
// IDENTICAL_BY_DESIGN is closed: each row is a value that is legitimately the
// same text in both catalogs (an acronym, a unit, a product or screen name, a
// loanword, or a frame made only of those). Translating one of these and leaving
// the row behind fails, which keeps the list honest. A row may only be added
// when a recorded policy ruling keeps that value literal — the direction labels
// `config.network.node.download/upload` joined this way under the review ruling
// that fixed the download/upload loanword policy for standalone labels.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {formatNumber} from "./format.js";
import {tn} from "./index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const PT = JSON.parse(readFileSync(resolve(CATALOG_DIR, "pt-BR.json"), "utf8"));
const LOCALES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "locales.json"), "utf8"));

/** Every catalog namespace the campaign translated. Config pages are covered by prefix. */
const TRANSLATED_NAMESPACES = new Set([
    "api",
    "api_docs",
    "asn_analysis",
    "asn_explorer",
    "bakery",
    "bridge",
    "chatbot",
    "circuit",
    "common",
    "cpu_workers",
    "dashboard",
    "dashlet",
    "ethernet_caps",
    "executive",
    "first_run",
    "flow_map",
    "flow_rtt_exclude",
    "graph",
    "help",
    "login",
    "queue_mode",
    "sankey",
    "scheduler",
    "server",
    "setup_runtime",
    "shaped_devices",
    "site_map",
    "sponsor",
    "stormguard_debug",
    "topology",
    "topology_manager",
    "topology_probes",
    "tree",
    "trial",
    "unknown_ips",
    "version",
]);

const CONFIG_PREFIX = "config.";

/** `config.general.x` groups by its page family, everything else by top segment. */
function namespaceOf(key) {
    const parts = key.split(".");
    return key.startsWith(CONFIG_PREFIX) ? parts.slice(0, 2).join(".") : parts[0];
}

/** `[variant, text]` pairs: one entry for a string, one per plural category. */
function variants(message) {
    return typeof message === "string" ? [["", message]] : Object.entries(message);
}

const PLACEHOLDER = /\{([A-Za-z0-9_]+)\}/g;

function placeholders(text) {
    return [...new Set(String(text).match(PLACEHOLDER) ?? [])].sort();
}

function categoriesOf(message) {
    return variants(message).map(([category]) => category);
}

/** The text the catalog would show: the runtime's own substitution, minus escaping. */
function rendered(template, count, params) {
    const values = {...params, count};
    return template.replace(PLACEHOLDER, (match, name) => {
        if (!(name in values)) return match;
        const value = values[name];
        return typeof value === "number" ? formatNumber(value) : String(value);
    });
}

/** The pt-BR CLDR categories, as the runtime's `pluralRules()` sees them. */
const SELECTABLE = new Set(new Intl.PluralRules(PT.locale).resolvedOptions().pluralCategories);

/** Values that are the same text in both catalogs on purpose, each with its reason. */
const IDENTICAL_BY_DESIGN = new Map([
    ["asn_analysis.label_with_asn", "`ASN` is the acronym the campaign keeps literal and the parentheses are punctuation, so the name-then-number frame reads the same"],
    ["asn_analysis.tooltip.qoo", "`QoO` is the fixed score name and the rest is the number frame around it plus the middle dot the source uses"],
    ["asn_explorer.col.bytes", "`Bytes` is the unit name the column heads, so the cell reads the same in both locales"],
    ["asn_explorer.heading.asn", "`ASN` is the acronym the campaign keeps literal and the rest is the number frame around it"],
    ["asn_explorer.name", "`ASN Explorer` is the screen name the navigation shows, so it is not translated"],
    ["bakery.elapsed.day_hour", "a unit frame: the letters are the day and hour units and the digits are rendered by the caller"],
    ["bakery.elapsed.seconds", "a unit frame: the letters are the seconds unit and the digits are rendered by the caller"],
    ["bridge.health.status", "`status` is the same word in Portuguese and the line inserts the server's own token"],
    ["bridge.health.workers", "a count frame around the `worker` concept the campaign keeps literal"],
    ["circuit.devices.rtt_p50_p95", "`RTT` and the percentile names stay literal"],
    ["circuit.rtt.internet", "`Internet` is the same word in Portuguese and names the measured segment"],
    ["circuit.traffic.qoo_columns", "`QoO` is the fixed score name plus the `d/u` direction abbreviations"],
    ["circuit.traffic.rtt", "`RTT` and `d/u` are a kept acronym and direction abbreviations"],
    ["common.display.klingon", "`Klingon` is the proper name the display-language menu shows too"],
    ["common.footer.copyright", "a proper name and the year prefix; the year itself is written at load time"],
    ["common.nav.bridge", "`Bridge` is the component name the campaign keeps literal; the nav row names the same shell"],
    ["common.table.dl", "`DL` is the direction abbreviation the campaign keeps literal"],
    ["common.table.dl_abbr", "an arrow and the `DL` abbreviation, both kept literal"],
    ["common.table.down", "`Down` is the direction token the campaign keeps literal"],
    ["common.table.download", "`Download` is the loanword Brazilian operators use, and the table head names the direction"],
    ["common.table.rtt_dl", "`RTT` is a kept acronym and `DL` a kept direction abbreviation"],
    ["common.table.rtt_dl_arrow", "an arrow and the `RTT` acronym, both kept literal"],
    ["common.table.rtt_ms", "an acronym plus its unit: `RTT` and `ms` stay literal"],
    ["common.table.rtt_ul", "`RTT` is a kept acronym and `UL` a kept direction abbreviation"],
    ["common.table.rtt_ul_arrow", "an arrow and the `RTT` acronym, both kept literal"],
    ["common.table.sites", "the campaign term for `site` is `Site`, so the plural head is the same word in both locales"],
    ["common.table.total", "`Total` is also the Portuguese word, and it heads the column that sums the two directions"],
    ["common.table.ul", "`UL` is the direction abbreviation the campaign keeps literal"],
    ["common.table.ul_abbr", "an arrow and the `UL` abbreviation, both kept literal"],
    ["common.table.up", "`Up` is the direction token the campaign keeps literal"],
    ["common.table.upload", "`Upload` is the loanword Brazilian operators use, and the table head names the direction"],
    ["common.worker", "the campaign keeps `worker` literal for the runtime concept, so badge and fact label share the word"],
    ["config.devices.sqm_down", "`SQM` names the kernel facility and `Down` is a kept direction token, so only a space separates the two locales"],
    ["config.devices.sqm_up", "`SQM` names the kernel facility and `Up` is a kept direction token, so only a space separates the two locales"],
    ["config.dynamic_circuits.ranges.down_aria", "`Down` is a kept direction token plus the `Mbps` unit: an attribute label, not prose"],
    ["config.dynamic_circuits.ranges.up_aria", "`Up` is a kept direction token plus the `Mbps` unit: an attribute label, not prose"],
    ["config.flows.notrack.label", "`CIDRs` is the acronym the row configures, so the label is the same word in both locales"],
    ["config.general.qoo.table.id", "`ID` is the same abbreviation in Portuguese; it heads the column of profile ids the server emits"],
    ["config.integration.ap_polling.state.ok", "`OK` is the status token the polling job reports, not prose"],
    ["config.integration.ap_polling.status.state", "`Status` is also the Portuguese word; the cell shows the server's own token"],
    ["config.integration.status.ingest", "the line names the watched `libreqos_network` directory and inserts the server's own state token, so only the punctuation around them is ours"],
    ["config.network.node.download", "`Download` is the direction loanword the campaign keeps in standalone labels; only the source's colon follows it"],
    ["config.network.node.upload", "`Upload` is the direction loanword the campaign keeps in standalone labels; only the source's colon follows it"],
    ["cpu_workers.cpu_unknown", "an acronym and the unknown-value question mark"],
    ["cpu_workers.local_api_unavailable", "an on-disk identifier and the server's own reason token, not prose"],
    ["cpu_workers.percent_value", "a bare number-and-percent frame"],
    ["cpu_workers.phase.tx", "`TX` is a direction token the campaign keeps literal"],
    ["cpu_workers.segment_title", "a label and percentage frame; both values are rendered by the caller"],
    ["cpu_workers.shaping", "`shaping` is the LibreQoS concept the campaign keeps literal"],
    ["cpu_workers.site_ap", "the campaign terms `Site` and `AP`, joined by the source's separator"],
    ["cpu_workers.snapshot_time", "`Snapshot` is the loanword the campaign keeps and the time is rendered by the caller"],
    ["cpu_workers.topology.shaping", "`shaping` is a kept concept and the count is rendered by the caller"],
    ["dashlet.category.cake", "`CAKE` is the qdisc name the campaign keeps literal"],
    ["dashlet.category.insight", "`Insight` is the product name"],
    ["dashlet.category.rtt", "`RTT` is a kept acronym"],
    ["dashlet.category.shaper", "`shaper` is the LibreQoS concept the campaign keeps literal"],
    ["dashlet.category.top_10", "`Top 10` is the ranking frame the page shows as-is"],
    ["dashlet.pulse.delta_24h", "a delta symbol and the hour window around a comparison the page names"],
    ["dashlet.pulse.downstream", "`Downstream` is the loanword Brazilian operators use for the direction"],
    ["dashlet.pulse.upstream", "`Upstream` is the loanword Brazilian operators use for the direction"],
    ["dashlet.stormguard.table.rtt", "`RTT` is a kept acronym heading the column"],
    ["dashlet.zoom", "`Zoom` is the same word in Portuguese and names the control"],
    ["ethernet_caps.column.ethernet", "`Ethernet` is the technology name the campaign keeps literal"],
    ["ethernet_caps.column.interface", "`Interface` is also the Portuguese word, and it heads the port column"],
    ["executive.col.asn", "`ASN` is the kept acronym heading the column"],
    ["executive.col.oversub_down", "the abbreviated `oversubscription` loanword plus the kept direction token, as a column head"],
    ["executive.col.oversub_up", "the abbreviated `oversubscription` loanword plus the kept direction token, as a column head"],
    ["executive.heat.direction.download", "`download` is the direction loanword the campaign keeps literal"],
    ["executive.heat.direction.upload", "`upload` is the direction loanword the campaign keeps literal"],
    ["executive.heat.kind.asn", "`ASN` is the kept acronym in its plural form"],
    ["executive.heat.kind.sites", "the campaign term for `site` is `Site`, so the lower-case kind is the same word"],
    ["executive.heat.tooltip.upload_download", "the two kept direction loanwords around values the caller renders"],
    ["executive.leaderboard.asn_number", "`ASN` plus the number the caller renders"],
    ["executive.panel.rtt_label", "`RTT` and the percentile names stay literal"],
    ["flow_map.cell.latitude", "`Latitude` is the same word in Portuguese and names the cell field"],
    ["flow_map.cell.longitude", "`Longitude` is the same word in Portuguese and names the cell field"],
    ["flow_map.hud.endpoints", "a count frame around the `endpoint` loanword the campaign keeps"],
    ["graph.axis.bytes", "`Bytes` is the unit name heading the axis"],
    ["graph.rtt_histo.tooltip_rtt", "`RTT` plus a start-end range and the `ms` unit, all rendered by the caller"],
    ["graph.series.download", "`Download` is the loanword Brazilian operators use, naming the series"],
    ["graph.series.upload", "`Upload` is the loanword Brazilian operators use, naming the series"],
    ["graph.stormguard.axis_rtt_ms", "`RTT` and the `ms` unit on an axis label"],
    ["help.chat.title", "`Chat` is the loanword Portuguese uses for the chat card"],
    ["help.ticket.view_title", "`Ticket` is the loanword Portuguese support pages use for the card title"],
    ["scheduler.degraded.detail.combined", "two values the caller renders, joined by the source's middle dot"],
    ["scheduler.panel.shaping", "`SHAPING` is the kept concept as an uppercase section label"],
    ["scheduler.progress.without_step", "a label and percentage frame; both values are rendered by the caller"],
    ["scheduler.runs.ok", "`ok` is the status token the scheduler reports"],
    ["site_map.popup.qoo", "`QoO` plus a value the caller renders"],
    ["site_map.popup.rtt", "`RTT` plus a value the caller renders"],
    ["site_map.unmapped.aps", "`APs` is the kept acronym in its plural form"],
    ["site_map.unmapped.sites", "the campaign term for `site` is `Site`, so the group name is the same word"],
    ["stormguard_debug.dir.down", "`Down` is the direction token the campaign keeps literal"],
    ["stormguard_debug.dir.up", "`Up` is the direction token the campaign keeps literal"],
    ["topology.node_kind.site", "the campaign term for `site` is `Site`, so the badge reads the same in both locales"],
    ["topology_manager.attachment.down_up", "a unit frame: the direction tokens `down`/`up` and `Mbps` are all kept literal"],
    ["topology_manager.details.via", "`via` is the same word in Portuguese between two values the caller renders"],
    ["topology_manager.rate.download", "`Download` is the loanword Brazilian operators use, followed by the unit"],
    ["topology_manager.rate.upload", "`Upload` is the loanword Brazilian operators use, followed by the unit"],
    ["topology_manager.role.manual", "`Manual` is the same word in Portuguese and names the attachment mode"],
    ["topology_manager.role.ptmp_uplink", "`PtMP` and `Uplink` are the network terms the campaign keeps literal"],
    ["topology_manager.role.ptp_backhaul", "`PtP` and `Backhaul` are the network terms the campaign keeps literal"],
    ["topology_probes.status", "`Status` is also the Portuguese word; the badge shows the probe state"],
    ["tree.branch.sites", "the campaign term for `site` is `Site`, so both plural forms are the same word"],
    ["tree.limit.kind.base", "`Base` is the same word in Portuguese and names the limit kind"],
    ["tree.stormguard.bounds", "a bounds frame: `D`/`U` direction tokens, digits and the `Mbps` unit"],
    ["tree.stormguard.candidate", "an action and a rate, both rendered by the caller, around an arrow"],
    ["trial.carousel.slide", "`Slide` is the loanword the signup carousel uses and the number is rendered by the caller"],
]);

/** Whether every category of a message still carries the English text. */
function isEnglishStub(key) {
    const portuguese = new Map(variants(PT.messages[key]));
    return variants(EN.messages[key]).every(([category, text]) => portuguese.get(category) === text);
}

/** The minimum `index.js` and `locale.mjs` need to resolve a catalog. */
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

test("locales.json advertises pt-BR as Portuguese (Brasil), left to right", () => {
    const entry = LOCALES.find((locale) => locale.tag === "pt-BR");
    assert.ok(entry, "locales.json must list pt-BR");
    assert.equal(entry.name, "Português (Brasil)");
    assert.equal(entry.dir, "ltr");
    assert.equal(PT.locale, "pt-BR", "the catalog must declare the tag the page serves");
    assert.equal(PT.dir, "ltr", "pt-BR is written left to right");
});

test("every catalog namespace is declared translated", () => {
    const namespaces = new Set(
        Object.keys(EN.messages)
            .filter((key) => !key.startsWith(CONFIG_PREFIX))
            .map(namespaceOf),
    );
    assert.deepEqual(
        [...TRANSLATED_NAMESPACES].sort(),
        [...namespaces].sort(),
        "TRANSLATED_NAMESPACES must name every catalog namespace; a new page family needs its translation and an entry here",
    );
});

test("no pt-BR variant carries the English text outside the closed list", () => {
    for (const [key, source] of Object.entries(EN.messages)) {
        if (IDENTICAL_BY_DESIGN.has(key)) continue;
        const portuguese = new Map(variants(PT.messages[key]));
        for (const [category, text] of variants(source)) {
            const label = category === "" ? key : `${key}.${category}`;
            assert.notEqual(portuguese.get(category), text, `pt-BR.json "${label}" still carries the English text`);
        }
    }
});

test("the English-verbatim values are exactly the closed list", () => {
    const total = Object.keys(EN.messages).length;
    const verbatim = Object.keys(EN.messages).filter(isEnglishStub).sort();
    assert.deepEqual(
        verbatim,
        [...IDENTICAL_BY_DESIGN.keys()].sort(),
        `pt-BR translates ${total - verbatim.length} of ${total} keys; only the closed list may stay English, and a row whose value was translated must be deleted`,
    );
});

test("the translated plurals define one, many and other, with many spelling other", () => {
    assert.deepEqual([...SELECTABLE].sort(), ["many", "one", "other"], "pt-BR CLDR must select one, many and other");
    const plurals = Object.keys(EN.messages).filter((key) => typeof EN.messages[key] !== "string");
    assert.ok(plurals.length > 0, "the catalog defines plural messages");
    for (const key of plurals) {
        const message = PT.messages[key];
        for (const category of SELECTABLE) {
            assert.equal(typeof message[category], "string", `pt-BR.json "${key}" is missing "${category}"`);
        }
        // The decision recorded for every catalog plural: `many` is a category
        // spelling the same text as `other`, not a second translation.
        assert.equal(message.many, message.other, `pt-BR.json "${key}" must spell "many" with the "other" text`);
        assert.notEqual(message.one, message.other, `pt-BR.json "${key}" must distinguish the singular from the plural`);
    }
});

test("tn() selects one for zero and many for a million", () => {
    installBrowser(PT, "pt-BR");
    const params = {total: "T", hint: "H", encoding: "E", volume: "V", endpoints: "P", state: "S"};
    const plurals = Object.keys(EN.messages).filter((key) => typeof EN.messages[key] !== "string");
    for (const key of plurals) {
        const message = PT.messages[key];
        assert.equal(
            tn(key, 0, params),
            rendered(message.one, 0, params),
            `pt-BR.json "${key}" must select "one" for a count of zero`,
        );
        assert.equal(
            tn(key, 1_000_000, params),
            rendered(message.many, 1_000_000, params),
            `pt-BR.json "${key}" must select "many" for a count of one million`,
        );
    }
});

test("pt-BR keeps the English key set, message kinds and placeholders", () => {
    assert.deepEqual(
        Object.keys(PT.messages).sort(),
        Object.keys(EN.messages).sort(),
        "pt-BR.json must define exactly the en.json key set",
    );
    for (const [key, source] of Object.entries(EN.messages)) {
        const translated = PT.messages[key];
        assert.equal(typeof translated, typeof source, `pt-BR.json "${key}" must keep the en.json message kind`);
        const sourceCategories = categoriesOf(source);
        const translatedCategories = categoriesOf(translated);
        // A category only the source has is a dropped shape; a category only pt-BR
        // has is allowed when pt-BR CLDR can select it (the added `many`).
        for (const category of sourceCategories) {
            assert.ok(translatedCategories.includes(category), `pt-BR.json "${key}" lost the "${category}" variant`);
        }
        for (const category of translatedCategories) {
            assert.ok(
                sourceCategories.includes(category) || SELECTABLE.has(category),
                `pt-BR.json "${key}" has a "${category}" variant neither en.json nor pt-BR CLDR defines`,
            );
        }
        const sourcePlural = new Set(variants(source).flatMap(([, text]) => placeholders(text)));
        const translatedPlural = new Set(variants(translated).flatMap(([, text]) => placeholders(text)));
        assert.deepEqual(
            [...translatedPlural].sort(),
            [...sourcePlural].sort(),
            `pt-BR.json "${key}" changed the placeholder set of the message`,
        );
        for (const category of sourceCategories.filter((name) => name !== "")) {
            assert.deepEqual(
                placeholders(PT.messages[key][category]),
                placeholders(source[category]),
                `pt-BR.json "${key}.${category}" changed the placeholder set`,
            );
        }
    }
});
