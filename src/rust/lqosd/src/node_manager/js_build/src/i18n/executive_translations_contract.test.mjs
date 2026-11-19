// Pinning tests for the shared executive page templates and heatmap dashlets
// (section S4c). Nine executive pages render through two page templates, four
// heatmap widgets and one snapshot widget, so a message dropped from one of them
// silently de-translates several screens at once.
//
// Two shapes need more than `extract.mjs`: a message whose metric or kind
// arrives as a placeholder (the page description, the row aria-label, the panel
// caption), and a message the markup keys but nobody sees until the placeholder
// data arrives. These tests render those messages in both locales and pin the
// call site that fills them.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t} from "./index.js";
import {heatRowAriaLabel, heatRowsCaption} from "../executive_heat_labels.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const NODE_MANAGER = resolve(HERE, "../../..");
const CATALOG_DIR = resolve(NODE_MANAGER, "static2/i18n");
const SOURCE_DIR = resolve(NODE_MANAGER, "js_build/src");
const STATIC_DIR = resolve(NODE_MANAGER, "static2");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));
const MESSAGES = {en: EN.messages, es: ES.messages};

/** The non-breaking space GLOSSARY §4 puts between a value and its unit. */
const NBSP = "\u00a0";

/**
 * The minimum `index.js` and `locale.mjs` need: the catalog globals, the
 * document's language and a cookie jar.
 * @param {object} catalog The catalog the page loaded.
 * @param {string} lang The served locale tag.
 */
function installBrowser(catalog, lang) {
    globalThis.window = {LQOS_I18N_EN: EN};
    if (lang !== "en") globalThis.window.LQOS_I18N = catalog;
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang}, cookie: ""};
}

function source(relative) {
    return readFileSync(resolve(SOURCE_DIR, relative), "utf8");
}

const PAGE_TEMPLATE = "executive_heatmap_page_template.js";
const LEADERBOARD_TEMPLATE = "executive_leaderboard_page_template.js";

test("the ranked heatmap description renders its metric in both locales", () => {
    installBrowser(EN, "en");
    assert.equal(
        t("executive.heatmap.description.ranked", {metric: t("executive.heatmap.metric.retransmit_pct")}),
        "Paged list ranked server-side by latest retransmit percentage.",
    );
    installBrowser(ES, "es");
    assert.equal(
        t("executive.heatmap.description.ranked", {metric: t("executive.heatmap.metric.retransmit_pct")}),
        "Lista paginada ordenada en el servidor por el valor más reciente de porcentaje de retransmisiones.",
    );

    // Three panels read the same sentence with a different metric: the metric is
    // a placeholder, so a fourth panel cannot grow a fourth page-specific title.
    const template = source(PAGE_TEMPLATE);
    for (const metric of ["retransmit_pct", "download_utilization", "upload_utilization"]) {
        assert.match(
            template,
            new RegExp(`t\\("executive\\.heatmap\\.description\\.ranked", \\{\\s*metric: t\\("executive\\.heatmap\\.metric\\.${metric}"\\),?\\s*\\}\\)`),
            `the ranked description must name the ${metric} metric through the catalog`,
        );
    }
});

test("the executive page shells key their title, lede and waiting state", () => {
    const SHELLS = new Map([
        ["executive_worst_sites.html", ["executive.page.title.worst_sites", "executive.page.lede.worst_sites"]],
        ["executive_oversubscribed_sites.html", ["executive.page.title.oversubscribed", "executive.page.lede.oversubscribed"]],
        ["executive_sites_due_upgrade.html", ["executive.page.title.sites_due_upgrade", "executive.page.lede.sites_due_upgrade"]],
        ["executive_circuits_due_upgrade.html", ["executive.page.title.circuits_due_upgrade", "executive.page.lede.circuits_due_upgrade"]],
        ["executive_top_asns.html", ["executive.page.title.top_asns", "executive.page.lede.top_asns"]],
        ["executive_heatmap_rtt.html", ["dashlet.name.median_rtt_heatmap", "executive.heatmap.listing.rtt"]],
        ["executive_heatmap_retransmit.html", ["dashlet.name.tcp_retransmits_heatmap", "executive.heatmap.listing.retransmit"]],
        ["executive_heatmap_download.html", ["executive.heatmap.title.download", "executive.heatmap.listing.download"]],
        ["executive_heatmap_upload.html", ["executive.heatmap.title.upload", "executive.heatmap.listing.upload"]],
    ]);
    for (const [file, keys] of SHELLS) {
        const html = readFileSync(resolve(STATIC_DIR, file), "utf8");
        for (const key of [...keys, "executive.page.waiting_data"]) {
            assert.match(html, new RegExp(`data-i18n="${key.replaceAll(".", "\\.")}"`),
                `${file} must key "${key}" so the shell's translation pass reaches it`);
            for (const locale of ["en", "es"]) {
                assert.ok(key in MESSAGES[locale], `"${key}" must be defined in ${locale}.json`);
            }
        }
        // The English text stays inline as the fallback the applier overwrites;
        // an empty element would render nothing until the catalog loads.
        for (const match of html.matchAll(/<([a-z0-9]+)[^>]*\bdata-i18n="[^"]+"[^>]*>([\s\S]*?)<\/\1>/gi)) {
            assert.notEqual(match[2].replace(/<[^>]*>/g, "").trim(), "",
                `${file}: a keyed element carries no inline English`);
        }
    }
});

test("the heat row aria frame renders the kind and the metric in both locales", () => {
    installBrowser(EN, "en");
    assert.equal(heatRowAriaLabel("Circuit 1", "Circuit", "5 %"), "Circuit 1 Circuit latest 5 %");
    assert.equal(heatRowAriaLabel("Site 2", "Site", "12 ms", "RTT"), "Site 2 Site latest RTT 12 ms");
    // ASN keeps its acronym in both locales: the badge word is the literal.
    assert.equal(heatRowAriaLabel("AS 64500", "ASN", "3 %"), "AS 64500 ASN latest 3 %");

    installBrowser(ES, "es");
    assert.equal(heatRowAriaLabel("Circuito 1", "Circuit", "5 %"), "Circuito 1 (Circuito). Valor más reciente: 5 %");
    assert.equal(heatRowAriaLabel("Sitio 2", "Site", "12 ms", "RTT"), "Sitio 2 (Sitio). Valor más reciente: RTT 12 ms");
});

test("the ASN identifier renders ungrouped in the leaderboard and the heatmap", () => {
    // Both surfaces frame a number the operator reads as an identifier, not a
    // count: `t()` would group 64512 into "64.512" in Spanish and "64,512" in
    // English, so the call sites pass a string and only the frame varies.
    installBrowser(ES, "es");
    assert.equal(t("executive.leaderboard.asn_number", {asn: String(64512)}), "ASN 64512");
    assert.equal(t("executive.leaderboard.asn_number", {asn: 64512}), "ASN 64.512");
    for (const file of [LEADERBOARD_TEMPLATE, "dashlets/executive_heatmap_shared.js"]) {
        const text = source(file);
        assert.match(text, /asn: String\((?:row\.asn|asn\.asn)\)/, `${file} must pass the ASN as a string`);
    }
});

test("the panel caption enumerates the rendered kinds in the active locale", () => {
    const rows = (...entity_kind) => entity_kind.map((kind) => ({entity_kind: kind}));

    installBrowser(EN, "en");
    assert.equal(heatRowsCaption(rows("Site", "Circuit")), "sites & circuits");
    assert.equal(heatRowsCaption(rows("Site", "Circuit", "Asn")), "sites, circuits & ASNs");
    assert.equal(heatRowsCaption(rows("Asn")), "ASNs");

    installBrowser(ES, "es");
    assert.equal(heatRowsCaption(rows("Site", "Circuit")), "sitios y circuitos");
    assert.equal(heatRowsCaption(rows("Site", "Circuit", "Asn")), "sitios, circuitos y ASN");
    assert.equal(heatRowsCaption(rows("Asn")), "ASN");
});

test("the pagination frame and the block tooltip are whole messages", () => {
    installBrowser(EN, "en");
    assert.equal(t("common.page.position", {page: 2, pages: 5}), "Page 2 / 5");
    assert.equal(t("common.state.updated", {time: "14:05"}), "Updated 14:05");
    assert.equal(t("executive.heat.block", {index: 3}), "15-minute block 3");

    installBrowser(ES, "es");
    assert.equal(t("common.page.position", {page: 2, pages: 5}), "Página 2 de 5");
    assert.equal(t("common.state.updated", {time: "14:05"}), "Última actualización: 14:05");
    assert.equal(t("executive.heat.block", {index: 3}), `Bloque 3 de 15${NBSP}min`);

    // The templates render the clock through the shared Intl formatter, so the
    // timestamp inside the sentence follows the served locale too.
    for (const template of [PAGE_TEMPLATE, LEADERBOARD_TEMPLATE]) {
        assert.match(source(template), /t\("common\.state\.updated", \{time: formatTime\(generatedAt\)\}\)/,
            `${template} must render the updated time through the catalog and the shared formatter`);
        assert.equal(/\.toLocale(?:String|TimeString|DateString)\(/.test(source(template)), false,
            `${template} must not format a clock by hand`);
    }
});

test("the dashlet titles and captions come from the catalog", () => {
    installBrowser(ES, "es");
    assert.equal(t("dashlet.name.global_heatmap"), "Mapa de calor global");
    assert.equal(t("executive.dashlet.title.circuit_utilization"), "Utilización de circuitos");
    assert.equal(t("executive.dashlet.title.site_ap_utilization"), "Utilización de sitios y AP");
    assert.equal(t("executive.caption.sites_aps"), "sitios y AP");
    assert.equal(t("executive.heat.kind.circuits"), "circuitos");

    const panels = source("dashlets/executive_heatmap_panels.js");
    for (const key of ["dashlet.name.global_heatmap", "executive.dashlet.title.circuit_utilization",
        "executive.dashlet.title.site_ap_utilization", "executive.caption.sites_aps"]) {
        assert.ok(panels.includes(`t("${key}")`), `the heatmap panels must render "${key}"`);
    }
});
