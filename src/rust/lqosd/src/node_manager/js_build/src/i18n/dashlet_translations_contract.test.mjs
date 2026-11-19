// Pinning tests for the analytics dashlets of section S4d: the StormGuard
// widgets, the throughput widgets, and the RTT-exclusion wizard.
//
// Two things reach the operator without passing through markup. A badge label is
// composed in code from a metric, and the wizard's help text is assembled at
// click time. The extractor can only prove a key exists, so these tests render
// the messages in both locales and check the call site that feeds them.
//
// The StormGuard badges carry a second risk: the attention ranking used to
// compare on the English label, which stops working the moment the label is
// translated. The ranking now compares a stable `code`, and the source checks
// below fail if a label comparison comes back.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {formatNumber} from "./format.js";
import {t, tn} from "./index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const NODE_MANAGER = resolve(HERE, "../../..");
const CATALOG_DIR = resolve(NODE_MANAGER, "static2/i18n");
const SOURCE_DIR = resolve(NODE_MANAGER, "js_build/src");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

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

const STORMGUARD_SHARED = source("dashlets/stormguard_shared.js");
const THROUGHPUT_BPS = source("dashlets/throughput_bps_dash.js");
const THROUGHPUT_RING = source("dashlets/throughput_ring_dash.js");
const RTT_WIZARD = source("lq_js_common/helpers/flow_rtt_exclude_wizard.js");

test("the StormGuard badges render the ruled Spanish state terms", () => {
    installBrowser(ES, "es");
    const ruled = new Map([
        ["dashlet.stormguard.state.no_data", "Sin datos"],
        ["dashlet.stormguard.state.cooling", "En enfriamiento"],
        ["dashlet.stormguard.state.reducing", "Reduciendo"],
        ["dashlet.stormguard.state.recovering", "Recuperando"],
        ["dashlet.stormguard.state.at_floor", "En el límite mínimo"],
        ["dashlet.stormguard.state.at_ceiling", "En el límite máximo"],
        ["dashlet.stormguard.state.congestion", "Señal de congestión"],
        ["dashlet.stormguard.state.loss", "Señal de retransmisiones"],
        ["dashlet.stormguard.state.warmup", "Recopilando la línea base"],
        ["dashlet.stormguard.state.holding", "Manteniendo la tasa"],
    ]);
    for (const [key, spanish] of ruled) {
        assert.equal(t(key), spanish, `${key} must render the ruled term`);
    }
    installBrowser(EN, "en");
    assert.equal(t("dashlet.stormguard.state.at_floor"), "At Floor");
    assert.equal(t("dashlet.stormguard.state.congestion"), "Congestion Signal");
    assert.equal(t("dashlet.stormguard.badge.dry_run"), "Dry Run");
    assert.equal(t("dashlet.stormguard.badge.live"), "Live");
});

test("the StormGuard attention ranking compares codes, never a translated label", () => {
    // A label is locale text: comparing on it ranks nothing once the page is
    // served in Spanish, and the failure is silent (every site scores 0).
    assert.doesNotMatch(STORMGUARD_SHARED, /\.label\s*[!=]==/, "no summary label may be compared");
    assert.doesNotMatch(STORMGUARD_SHARED, /label:\s*"/, "every badge label comes from the catalog");
    for (const [code, weight] of [["congestion", 60], ["reducing", 50], ["recovering", 40], ["at_floor", 20], ["at_ceiling", 10]]) {
        assert.match(
            STORMGUARD_SHARED,
            new RegExp(`\\["${code}",\\s*${weight}\\]`),
            `the ranking must weight the "${code}" code`,
        );
    }
    assert.match(STORMGUARD_SHARED, /codes\.has\(code\)/, "the ranking reads the code set");
    // Both directions contribute, so the summary carries the code on each branch.
    assert.match(STORMGUARD_SHARED, /downloadSummary\?\.code/);
    assert.match(STORMGUARD_SHARED, /uploadSummary\?\.code/);
    // The state value the backend sends is a token, not text: only its display
    // string is translated.
    assert.match(STORMGUARD_SHARED, /state === "warmup"/);
});

test("the StormGuard reasons keep the measurement inside the message", () => {
    installBrowser(EN, "en");
    assert.equal(
        t("dashlet.stormguard.reason.congestion", {ms: "12.50"}),
        "Standing delay is 12.50 ms above baseline",
    );
    installBrowser(ES, "es");
    assert.equal(
        t("dashlet.stormguard.reason.congestion", {ms: "12,50"}),
        `El retardo sostenido supera la línea base en 12,50${NBSP}ms`,
    );
    assert.equal(
        t("dashlet.stormguard.detail.cooldown", {seconds: "4,5"}),
        `Enfriamiento activo: 4,5${NBSP}s.`,
    );
    assert.equal(t("tree.stormguard.cooldown_remaining", {seconds: "4.5"}), `Enfriamiento: 4.5${NBSP}s`);
    assert.equal(t("dashlet.stormguard.detail.retrans", {percent: `1,25${NBSP}%`}), `Las retransmisiones son del 1,25${NBSP}%.`);
    // Both placeholders have to be supplied at the call site.
    assert.match(STORMGUARD_SHARED, /t\("dashlet\.stormguard\.reason\.congestion", \{ms: formatNumber\(delay, TWO_DECIMALS\)\}\)/);
    assert.match(STORMGUARD_SHARED, /t\("dashlet\.stormguard\.detail\.cooldown", \{seconds: formatNumber\(cooldown, ONE_DECIMAL\)\}\)/);
});

test("the StormGuard measurements take their decimal mark from the active locale", () => {
    // §4.1: `toFixed()` and a hand-appended "%" put an English decimal mark and a
    // glued unit inside a Spanish sentence. The formatters ask the locale for
    // both. The module is bundle-only, so the wiring is read from its source and
    // the numbers themselves are checked against the shared formatter.
    const oneDecimal = {minimumFractionDigits: 1, maximumFractionDigits: 1};
    const twoDecimals = {minimumFractionDigits: 2, maximumFractionDigits: 2};
    const percent = {style: "percent", minimumFractionDigits: 2, maximumFractionDigits: 2};
    installBrowser(EN, "en");
    assert.equal(formatNumber(4.5, oneDecimal), "4.5");
    assert.equal(formatNumber(12.5, twoDecimals), "12.50");
    assert.equal(formatNumber(0.0125, percent), "1.25%");
    installBrowser(ES, "es");
    assert.equal(formatNumber(4.5, oneDecimal), "4,5");
    assert.equal(formatNumber(12.5, twoDecimals), "12,50");
    // The percent mark's spacing is the locale's, not a literal glued to the digits.
    assert.equal(formatNumber(0.0125, percent), `1,25${NBSP}%`);

    assert.doesNotMatch(STORMGUARD_SHARED, /\.toFixed\(/, "no StormGuard value may be built with toFixed()");
    assert.doesNotMatch(STORMGUARD_SHARED, /toLocaleString\(undefined/, "the formatter may not fall back to the runtime locale");
    assert.match(STORMGUARD_SHARED, /formatNumber\(n, STORMGUARD_MBPS\)/);
    assert.match(STORMGUARD_SHARED, /formatNumber\(n, STORMGUARD_PERCENT\)/);
    assert.match(STORMGUARD_SHARED, /\{seconds: formatNumber\(cooldown, ONE_DECIMAL\)\}/);
    assert.match(STORMGUARD_SHARED, /\{ms: formatNumber\(delay, TWO_DECIMALS\)\}/);
});

test("the throughput dashlets name their period from the catalog and keep the period token literal", () => {
    installBrowser(EN, "en");
    assert.equal(t("dashlet.name.throughput_bits_per_second"), "Throughput Bits/Second");
    assert.equal(t("dashlet.throughput.ring.period.6h"), "Last 6 Hours");
    assert.equal(t("dashlet.throughput.ring.title", {period: t("dashlet.throughput.ring.period.6h")}), "Last 6 Hours Traffic");
    assert.equal(tn("dashlet.throughput.ring.period.minutes", 5), "Last 5 Minutes");
    assert.equal(tn("dashlet.throughput.ring.period.minutes", 1), "Last 1 Minute");

    installBrowser(ES, "es");
    assert.equal(t("dashlet.name.throughput_bits_per_second"), "Tasa de transferencia en bits/s");
    assert.equal(t("dashlet.throughput.ring.period.6h"), "Últimas 6 horas");
    assert.equal(t("dashlet.throughput.ring.title", {period: t("dashlet.throughput.ring.period.6h")}), "Últimas 6 horas: tráfico");
    assert.equal(tn("dashlet.throughput.ring.period.minutes", 5), "Últimos 5 minutos");
    assert.equal(tn("dashlet.throughput.ring.period.minutes", 1), "Último 1 minuto");
    assert.equal(
        t("dashlet.throughput.ring.tooltip", {window: t("dashlet.throughput.ring.describe.6h")}),
        "Tráfico asignado (controlado y limitado por AQM) y sin asignar (no encontrado en su archivo ShapedDevices.csv) durante las últimas 6 horas.",
    );
    // The graph legend reuses the shipped mapped/unmapped wording.
    assert.equal(t("graph.pie.mapped"), "Asignado");
    assert.equal(t("graph.pie.unmapped"), "Sin asignar");

    // `Live` and the hour tokens are values the page compares against, so the
    // switch must key on them and translate only what each branch returns.
    assert.match(THROUGHPUT_RING, /activePeriod === "Live"/);
    for (const token of ["1h", "6h", "12h", "24h", "7d"]) {
        assert.match(
            THROUGHPUT_RING,
            new RegExp(`case "${token}": return t\\("dashlet\\.throughput\\.ring\\.period\\.${token}"\\)`),
            `the "${token}" branch must return a catalog message`,
        );
    }
    assert.doesNotMatch(THROUGHPUT_RING, /return "Last /, "no period label may be an English literal");
});

test("the bits-per-second widget keeps the raw rate formatter and translates only its labels", () => {
    // The values keep the unit-less scaled number the graph axis expects; the
    // labels around them come from the catalog.
    assert.match(THROUGHPUT_BPS, /scaleNumber\(dlMedian, 1\)/);
    assert.match(THROUGHPUT_BPS, /scaleNumber\(upMedian, 1\)/);
    assert.match(THROUGHPUT_BPS, /t\("config\.network\.node\.download"\)/);
    assert.match(THROUGHPUT_BPS, /t\("config\.network\.node\.upload"\)/);
    assert.doesNotMatch(THROUGHPUT_BPS, /textContent = "Download:"/);

    // §4.4: the tooltip names the traffic's direction from the subscriber's side
    // in both locales; the ISP-centric parenthesis contradicted the glossary, and
    // the counters themselves are not swapped to fit a sentence.
    assert.equal(
        EN.messages["dashlet.throughput.bps.tooltip"],
        "Shows the current throughput in bits per second, split into download (to the subscriber) and upload (from the subscriber).",
    );
    assert.doesNotMatch(EN.messages["dashlet.throughput.bps.tooltip"], /ISP/);
    assert.match(THROUGHPUT_BPS, /msg\.data\.bps\.down/);
    assert.match(THROUGHPUT_BPS, /msg\.data\.bps\.up/);

    installBrowser(ES, "es");
    assert.equal(t("dashlet.throughput.bps.current"), "Actual:");
    assert.equal(t("dashlet.throughput.bps.yesterday"), "Ayer:");
    assert.equal(t("dashlet.throughput.bps.last_week"), "La semana pasada:");
    // The entitlement notice is a three-line stack; each line is its own message.
    assert.equal(
        [t("dashlet.throughput.bps.history"), t("dashlet.throughput.bps.requires"), t("dashlet.throughput.bps.entitlement")].join(" "),
        "Historial requiere derechos de uso",
    );
});

test("the RTT-exclusion wizard names its consequence and takes every label from the catalog", () => {
    installBrowser(ES, "es");
    assert.equal(t("flow_rtt_exclude.title"), "Excluir el RTT del extremo remoto");
    assert.equal(t("flow_rtt_exclude.scope.label"), "Elija el alcance de la exclusión:");
    assert.equal(t("flow_rtt_exclude.scope.host"), "Solo esta dirección IP (recomendado):");
    assert.equal(t("flow_rtt_exclude.copy_cidr"), "Copiar el CIDR");
    assert.equal(t("flow_rtt_exclude.copied"), "Copiado");
    assert.equal(t("flow_rtt_exclude.copy_failed"), "No se pudo copiar");
    // Suppressing a measurement has a consequence, and the help text states it
    // instead of describing the click path only.
    assert.match(t("flow_rtt_exclude.explain"), /Seguimiento de flujos → Subredes excluidas del seguimiento/);
    assert.match(t("flow_rtt_exclude.explain"), /se ignoran al calcular la puntuación y los mapas de calor/);
    assert.match(t("flow_rtt_exclude.explain"), /no se guarda nada automáticamente/);

    installBrowser(EN, "en");
    assert.match(t("flow_rtt_exclude.explain"), /ignored for scoring and heatmaps/);
    assert.match(t("flow_rtt_exclude.explain"), /nothing is saved automatically/);

    // The modal is built in code: a literal here would leave the Spanish wizard
    // half English, and the extractor cannot see a label that was never a key.
    assert.match(RTT_WIZARD, /title\.textContent = t\("flow_rtt_exclude\.title"\)/);
    assert.match(RTT_WIZARD, /explain\.textContent = t\("flow_rtt_exclude\.explain"\)/);
    assert.match(RTT_WIZARD, /t\("common\.action\.close"\)/);
    assert.doesNotMatch(RTT_WIZARD, /textContent = "Copy CIDR"/);
    assert.doesNotMatch(RTT_WIZARD, /aria-label", "Close"/);
});

test("the shared modal and toast helpers close through the catalog", () => {
    for (const relative of ["lq_js_common/helpers/alert_modal.js", "lq_js_common/helpers/toasts.js"]) {
        const text = source(relative);
        assert.match(text, /t\("common\.action\.close"\)/, `${relative} must translate its close control`);
        assert.doesNotMatch(text, /"Close"/, `${relative} must not carry the English close label`);
    }
});
