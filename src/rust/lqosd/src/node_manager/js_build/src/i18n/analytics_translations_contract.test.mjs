// Pinning tests for the analytics surfaces of section S4b: the ASN pages and
// the shared graph modules.
//
// Two kinds of text reach the operator without passing through markup: an
// ECharts axis name and a tooltip the chart builds at hover time. Both are
// composed in code, so the extractor can only see that the key exists; these
// tests render the message in both locales and check the call site that feeds
// it, which is what a translation that silently stopped rendering would break.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t} from "./index.js";
import {formatNumber} from "./format.js";
import {cakeLegendFormatter, cakeSeriesDisplay, cakeTinLabel} from "../graphs/cake_tin_labels.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const NODE_MANAGER = resolve(HERE, "../../..");
const CATALOG_DIR = resolve(NODE_MANAGER, "static2/i18n");
const SOURCE_DIR = resolve(NODE_MANAGER, "js_build/src");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));
const ASN_ANALYSIS = readFileSync(resolve(SOURCE_DIR, "asn_analysis.js"), "utf8");

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

test("the ASN bubble chart names its traffic axis from the catalog", () => {
    installBrowser(EN, "en");
    assert.equal(t("asn_analysis.axis.traffic_15m"), "Traffic (15m total)");
    installBrowser(ES, "es");
    assert.equal(t("asn_analysis.axis.traffic_15m"), `Tráfico (total de 15${NBSP}min)`);

    // The axis name is the message: a literal would leave the Spanish chart English.
    assert.match(
        source("asn_analysis.js"),
        /name: t\("asn_analysis\.axis\.traffic_15m"\)/,
        "the bubble chart x axis must be named through the catalog",
    );
});

test("the ASN measurements take their decimal mark from the active locale", () => {
    // §4.1: a value built with `toFixed()` keeps its English decimal mark inside a
    // Spanish sentence. The ASN formatters ask the locale for the digits instead,
    // at the precision the axis and the tooltips already used.
    const oneDecimal = {minimumFractionDigits: 1, maximumFractionDigits: 1};
    const twoPercent = {style: "percent", minimumFractionDigits: 2, maximumFractionDigits: 2};
    installBrowser(EN, "en");
    assert.equal(formatNumber(25.5, oneDecimal), "25.5");
    assert.equal(formatNumber(25.5 / 100, twoPercent), "25.50%");
    installBrowser(ES, "es");
    assert.equal(formatNumber(25.5, oneDecimal), "25,5");
    assert.equal(formatNumber(25.5 / 100, twoPercent), `25,50${NBSP}%`);

    // The call sites have to route through that formatter: a `toFixed()` that
    // comes back renders "25.5" in the Spanish tooltip again.
    assert.doesNotMatch(ASN_ANALYSIS, /\.toFixed\(/, "the ASN page may not build a decimal string by hand");
    assert.match(ASN_ANALYSIS, /rate: formatNumber\(mbps, \{minimumFractionDigits: digits, maximumFractionDigits: digits\}\)/);
    assert.match(ASN_ANALYSIS, /formatNumber\(rttMs, \{minimumFractionDigits: digits, maximumFractionDigits: digits\}\)/);
    assert.match(ASN_ANALYSIS, /formatNumber\(value \/ 100, RETRANS_PERCENT\)/);
    assert.match(ASN_ANALYSIS, /formatNumber\(toNumber\(value, 0\) \/ 100, RETRANS_AXIS_PERCENT\)/);
});

test("an ASN number stays an identifier: Intl grouping never reaches it", () => {
    // `{name} (ASN {asn})` is an identifier frame, not a count. `t()` formats
    // every number parameter through Intl, so a bare `row.asn` renders
    // "ASN 64.512" in Spanish and "ASN 64,512" in English: the call sites pass
    // the number as a string instead, and only the frame is translated.
    const ASN = 64512;
    installBrowser(EN, "en");
    assert.equal(t("asn_analysis.label_with_asn", {name: "Example", asn: String(ASN)}), "Example (ASN 64512)");
    installBrowser(ES, "es");
    assert.equal(t("asn_analysis.label_with_asn", {name: "Ejemplo", asn: String(ASN)}), "Ejemplo (ASN 64512)");
    // The shape the label must not ship: this is what a raw number renders.
    assert.equal(t("asn_analysis.label_with_asn", {name: "Ejemplo", asn: ASN}), "Ejemplo (ASN 64.512)");
    // Every call site of the frame passes the identifier as a string. `asn` is a
    // number on the chart payloads too, so the guard reads the label calls and
    // not the data objects beside them.
    const labelCalls = [...ASN_ANALYSIS.matchAll(/t\("asn_analysis\.label_with_asn", \{[^}]*\}\)/g)].map((match) => match[0]);
    assert.equal(labelCalls.length, 5, "the page renders the ASN label at five call sites");
    for (const call of labelCalls) {
        assert.match(call, /\basn: String\(/, `the ASN label must pass the identifier as a string: ${call}`);
    }
});

test("the ASN bubble tooltip renders both of its measurements", () => {
    const params = {volume: "1,2 GB", rate: "25,5 Mbps de media"};
    installBrowser(EN, "en");
    assert.equal(t("asn_analysis.tooltip.traffic", {volume: "1.2 GB", rate: "25.5 Mbps avg"}),
        "Traffic: 1.2 GB (25.5 Mbps avg)");
    installBrowser(ES, "es");
    assert.equal(t("asn_analysis.tooltip.traffic", params), "Tráfico: 1,2 GB (25,5 Mbps de media)");

    // Both placeholders have to be supplied at the call site: one missing name
    // renders the literal `{rate}` inside a hover box nobody re-reads.
    assert.match(
        source("asn_analysis.js"),
        /t\("asn_analysis\.tooltip\.traffic", \{volume: formatVolume\([^)]*\), rate: formatTrafficRateMbps\([^)]*\)\}\)/,
        "the bubble tooltip must pass the volume and the rate",
    );
});

test("the graph axis labels and the RTT histogram tooltip come from the catalog", () => {
    installBrowser(EN, "en");
    assert.equal(t("graph.axis.throughput_bps"), "Throughput (bps)");
    assert.equal(t("graph.rtt_histo.tooltip_rtt", {start: 5, end: 10}), "RTT: 5-10 ms");
    installBrowser(ES, "es");
    assert.equal(t("graph.axis.throughput_bps"), "Tasa de transferencia (bps)");
    assert.equal(t("graph.rtt_histo.tooltip_rtt", {start: 5, end: 10}), `RTT de 5 a 10${NBSP}ms`);

    assert.match(
        source("graphs/throughput_ring_graph.js"),
        /\.withScaledAbsYAxis\(t\("graph\.axis\.throughput_bps"\), 40\)/,
        "the throughput axis name must be the translated message",
    );
});

test("CAKE series identifiers keep their identity and render in the active language", () => {
    // The chart series are named after CAKE's tins because `CAKE_TIN_LEGEND_PAIRS`
    // and `cakeTinIndexFor` match on those names; only the legend and the tooltip
    // translate them.
    installBrowser(EN, "en");
    assert.equal(cakeTinLabel("Best Effort"), "Best Effort");
    assert.equal(cakeLegendFormatter("RT Voice Up"), "RT Voice Up");
    installBrowser(ES, "es");
    assert.equal(cakeTinLabel("Best Effort"), "Mejor esfuerzo");
    assert.equal(cakeLegendFormatter("Bulk"), "Masivo");
    assert.equal(cakeLegendFormatter("Bulk Up"), "Masivo (sub.)");
    assert.equal(cakeLegendFormatter("RT Voice Up"), "Voz en tiempo real (sub.)");
    // §4.3: the tooltip strips the `" Up"` suffix and looks the tin up again, so
    // the stripped identifier needs its own mapping; without one the tooltip fell
    // back to the English `RT Voice`.
    assert.equal(cakeSeriesDisplay("RT Voice Up").label, "Voz");
    assert.equal(cakeSeriesDisplay("Voice").label, "Voz");
    assert.equal(cakeSeriesDisplay("RT Video Up").label, "Vídeo en tiempo real");
    // A name the map does not know passes through, so an unmapped tin is visible
    // rather than silently blank in the legend.
    assert.equal(cakeTinLabel("Unmapped Tin"), "Unmapped Tin");
});
