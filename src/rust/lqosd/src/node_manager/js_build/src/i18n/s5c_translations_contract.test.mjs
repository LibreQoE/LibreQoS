// Pinning tests for section S5c: the long tail of dashlets, the dashboard
// editor, the Libby chat page, and the Insight promotions.
//
// The extractor proves a key exists and has a reader. It cannot see whether the
// *rendered* surface still names the thing it is supposed to name: a dashlet
// that loses its `t()` call, or a catalog that ships the English value under
// `es`, passes the gate. Each case below pins the load-bearing key to the file
// that renders it, so a deletion fails here instead of silently dropping a
// label.
//
// The bundles build DOM at import time, so nothing here imports them: the
// assertions read the files as text and render messages from the catalogs.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t, tn} from "./index.js";
import {ONE_DECIMAL, formatNumber} from "./format.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const NODE_MANAGER = resolve(HERE, "../../..");
const CATALOG_DIR = resolve(NODE_MANAGER, "static2/i18n");
const SOURCE_DIR = resolve(NODE_MANAGER, "js_build/src");
const STATIC_DIR = resolve(NODE_MANAGER, "static2");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

/** The minimum `index.js` and `locale.mjs` need: the catalog globals and a document. */
function installBrowser(catalog, lang) {
    globalThis.window = {LQOS_I18N_EN: EN};
    if (lang !== "en") globalThis.window.LQOS_I18N = catalog;
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang}, cookie: ""};
}

function source(relative) {
    return readFileSync(resolve(SOURCE_DIR, relative), "utf8");
}

function markup(relative) {
    return readFileSync(resolve(STATIC_DIR, relative), "utf8");
}

test("the dashlets translate titles, tooltips, and column headers", () => {
    const CASES = [
        ["dashlets/cpu_dash.js", "dashlet.name.cpu_utilization", "dashlet.cpu.tooltip"],
        ["dashlets/ram_dash.js", "dashlet.name.ram_utilization", "dashlet.ram.tooltip"],
        ["dashlets/flow_durations_dash.js", "dashlet.name.flow_duration", "dashlet.flow_duration.tooltip"],
        ["dashlets/rtt_histo3d_dash.js", "dashlet.name.round_trip_time_histogram_3d", "dashlet.rtt_histo3d.tooltip"],
        ["dashlets/worst10_downloaders.js", "dashlet.name.worst_10_round_trip_time", "dashlet.worst10.rtt.tooltip"],
        ["dashlets/worst10_retransmits.js", "dashlet.name.worst_10_retransmits", "dashlet.worst10.retransmits.tooltip"],
        ["dashlets/stormguard_status.js", "dashlet.name.stormguard_bandwidth_adjustments", "dashlet.stormguard_bandwidth.tooltip"],
        ["dashlets/ltsShaperStatus.js", "dashlet.name.insight_shaper_status", "dashlet.lts_shaper_status.tooltip"],
    ];
    for (const [file, nameKey, tooltipKey] of CASES) {
        const text = source(file);
        assert.match(text, new RegExp(`t\\("${nameKey.replaceAll(".", "\\.")}"\\)`), `${file} must translate its title`);
        assert.match(text, new RegExp(`t\\("${tooltipKey.replaceAll(".", "\\.")}"\\)`), `${file} must translate its tooltip`);
        // A tooltip's heading is the dashlet's own title, not a second literal.
        assert.doesNotMatch(text, /<h5>[A-Z][^<${]*<\/h5>/, `${file} must not carry an English tooltip heading`);
    }

    // Column headers are the labels most likely to be dropped: they live in a
    // template literal, where the extractor only sees the key, never the cell.
    const bytesTable = source("dashlets/top10flows_bytes.js");
    for (const key of ["common.table.ip_circuit", "common.table.dl_abbr", "common.table.ul_abbr",
        "common.table.total", "common.table.remote_asn", "common.table.rxmits_dl"]) {
        assert.match(bytesTable, new RegExp(`t\\("${key.replaceAll(".", "\\.")}"\\)`), `the top-flows table must translate ${key}`);
    }
    assert.doesNotMatch(bytesTable, /theading\("(?:DL|UL|Total|Remote ASN)"/, "no column header may stay an English literal");

    installBrowser(ES, "es");
    assert.equal(t("common.table.dl_abbr"), "Desc. \u2b07\ufe0f");
    assert.equal(t("common.table.ul_abbr"), "Sub. \u2b06\ufe0f");
    assert.equal(t("common.table.remote_asn"), "ASN remoto");
    assert.equal(t("dashlet.tree_summary.marks_heading"), "Marcas de CAKE");
    assert.equal(t("dashlet.capacity.circuits.title"), "Circuitos en su capacidad máxima");
});

test("the live pulse keeps its measurements inside whole messages", () => {
    const pulse = source("dashlets/live_traffic_pulse_dash.js");
    assert.doesNotMatch(pulse, /% of total/, "the share of total must come from the catalog");
    assert.doesNotMatch(pulse, /Last 30 seconds/, "the window label must come from the catalog");
    // The percent is formatted by Intl *before* it enters the message: a
    // `toFixed()` string skips `t()`'s number formatting and ships "12.5" in a
    // sentence that reads "(12,5 % del total)" everywhere else.
    assert.match(pulse, /t\("dashlet\.pulse\.pct_of_total", \{percent: formatNumber\(pct, ONE_DECIMAL\)\}\)/);
    assert.doesNotMatch(pulse, /pct\.toFixed\(/, "the percent must not be stringified before t()");
    assert.match(pulse, /t\("dashlet\.pulse\.window", \{seconds: 30\}\)/);

    installBrowser(ES, "es");
    // The rendered path, end to end: formatNumber supplies the digits and the
    // message supplies the Spanish frame.
    assert.equal(t("dashlet.pulse.pct_of_total", {percent: formatNumber(12.5, ONE_DECIMAL)}), "(12,5% del total)");
    assert.equal(t("dashlet.pulse.pct_of_total", {percent: formatNumber(12345.6, ONE_DECIMAL)}), "(12.345,6% del total)");
    // A bare number reaches the same rendering: t() is the formatting authority.
    assert.equal(t("dashlet.pulse.pct_of_total", {percent: 12.5}), "(12,5% del total)");
    assert.equal(t("dashlet.pulse.window", {seconds: 30}), "Últimos 30 segundos");
    // A plural status line: the count selects the category, and the encoding
    // sentence rides inside the same message.
    installBrowser(EN, "en");
    assert.equal(
        tn("dashlet.world_map.status", 1, {count: 1, encoding: "X."}),
        "Showing 1 recent endpoint location. X.",
    );
    installBrowser(ES, "es");
    assert.equal(
        tn("dashlet.world_map.status", 1, {count: 1, encoding: "X."}),
        "Se muestra 1 ubicación reciente de extremos. X.",
    );
    assert.equal(
        tn("dashlet.world_map.status", 4, {count: 4, encoding: "X."}),
        "Se muestran 4 ubicaciones recientes de extremos. X.",
    );
});

test("the dashboard editor takes every label from the catalog and keeps its CSS in the stylesheet", () => {
    const editor = source("lq_js_common/dashboard/dashboard_editor.js");
    for (const key of ["dashboard.editor.title", "dashboard.editor.download", "dashboard.editor.upload",
        "dashboard.editor.clear_tab", "dashboard.editor.restore_defaults", "dashboard.editor.add_tab",
        "dashboard.editor.drag_hint", "dashboard.editor.imported", "dashboard.editor.parse_failed"]) {
        assert.match(editor, new RegExp(`t\\("${key.replaceAll(".", "\\.")}"`), `the editor must translate ${key}`);
    }
    // The modal is assembled from a template literal: an English label left in
    // it is invisible to the extractor and ships untranslated.
    for (const literal of ["Dashboard Editor", "Download Layout", "Clear Current Tab", "Restore Defaults", "Add Tab"]) {
        assert.equal(editor.includes(`>${literal}<`), false, `the modal must not carry "${literal}" as text`);
    }
    // The editor used to inject its own stylesheet; that block was a single
    // untranslatable literal, and CSS belongs to the stylesheet.
    assert.doesNotMatch(editor, /<style/, "the editor must not build a stylesheet string");
    const css = readFileSync(resolve(STATIC_DIR, "node_manager.css"), "utf8");
    assert.match(css, /#dashboardEditorModal \.dashboard-grid-container/, "the editor scaffolding must stay styled");
    // A default tab name is a message with the number inside it, not a
    // concatenation the translator cannot reorder.
    const layout = source("lq_js_common/dashboard/layout.js");
    assert.match(layout, /t\("dashboard\.tab_default", \{number: index \+ 1\}\)/);
    assert.doesNotMatch(layout, /`Tab \$\{/, "no tab name may be built from an English literal");

    installBrowser(ES, "es");
    assert.equal(t("dashboard.editor.title"), "Editor del panel");
    assert.equal(t("dashboard.tab_default", {number: 3}), "Pestaña 3");
    assert.equal(t("dashboard.editor.parse_failed", {error: "boom"}), "No se pudo leer el archivo de diseño: boom");
});

test("the Libby chat page translates its notice and composer", () => {
    const page = markup("chatbot.html");
    for (const key of ["chatbot.avatar_alt", "chatbot.notice.entitled_required", "chatbot.notice.link_tail",
        "chatbot.composer.placeholder", "chatbot.composer.input_label", "chatbot.composer.send_to_libby",
        "chatbot.composer.send_title"]) {
        assert.match(page, new RegExp(`data-i18n(?:-[a-z-]+)?="${key.replaceAll(".", "\\.")}"`), `chatbot.html must key ${key}`);
    }
    assert.doesNotMatch(page, /placeholder="Ask something\.\.\."(?! data-i18n)/, "the composer placeholder must be keyed");

    const chat = source("chatbot.js");
    for (const key of ["chatbot.notice.control_unavailable", "chatbot.notice.entitled_required",
        "chatbot.composer.entitled_placeholder", "chatbot.composer.entitled_title",
        "chatbot.status.connected", "chatbot.status.unavailable"]) {
        assert.match(chat, new RegExp(`t\\("${key.replaceAll(".", "\\.")}"\\)`), `chatbot.js must translate ${key}`);
    }
    // The websocket/SSE plumbing and CSS class names stay literal, and the
    // service's own answer stream is never translated.
    assert.match(chat, /trimmed\.startsWith\('data:'\)/);
    assert.match(chat, /row\.className = 'msg me'/);

    installBrowser(ES, "es");
    assert.equal(t("chatbot.composer.placeholder"), "Escriba una pregunta…");
    assert.equal(t("chatbot.notice.entitled_required"), "Libby requiere una licencia con derechos de uso.");
    assert.equal(t("chatbot.status.unavailable"), "Libby requiere una licencia con derechos de uso. Abra «Licencia y servicios» para habilitar el chat.");
});

test("the Insight promotions translate the local copy and leave the service copy alone", () => {
    const teasers = source("lts_teasers_shared.js");
    // The teaser objects are shared by the trial page and the nag modal, so the
    // copy is a getter: a module-load snapshot would freeze the first locale.
    assert.match(teasers, /get title\(\) \{ return t\("trial\.teaser\.site_rankings\.title"\); \}/);
    assert.match(teasers, /get description\(\) \{ return t\("trial\.teaser\.api_only\.body"\); \}/);
    assert.match(teasers, /get title\(\) \{ return t\("dashboard\.tab\.executive_summary"\); \}/);
    assert.doesNotMatch(teasers, /title: "/, "no teaser title may stay an English literal");
    assert.doesNotMatch(teasers, /description:\s*"/, "no teaser body may stay an English literal");

    const sponsor = source("toasts/sponsor_us.js");
    assert.match(sponsor, /\(\) => t\("sponsor\.message\.heatmaps"\)/);
    // The exceeded banner names the excess the cap suppresses, not the configured
    // total, and never a negative count.
    assert.match(
        sponsor,
        /message: tn\("sponsor\.limit\.exceeded", Math\.max\(0, mappedCircuits - 1000\)\)/,
        "the exceeded banner must render the circuits the cap suppresses",
    );
    assert.match(sponsor, /message: t\("sponsor\.limit\.approaching"\)/);
    assert.doesNotMatch(sponsor, /message: "LibreQoS handles shaping/, "no sponsor line may stay an English literal");

    installBrowser(ES, "es");
    assert.equal(t("trial.teaser.site_rankings.title"), "Clasificación de sitios");
    assert.equal(t("trial.teaser.executive.body").slice(0, 10), "Vea de un ");
});

test("the mapped-circuit cap copy names the consequence in every plural form", () => {
    installBrowser(EN, "en");
    assert.equal(
        t("sponsor.limit.approaching"),
        "Unlicensed deployments are capped at 1,000 mapped circuits, and this system is approaching that limit. Circuits beyond the cap are not created.",
    );
    assert.equal(
        tn("sponsor.limit.exceeded", 1),
        "1 mapped circuit is not being created because unlicensed deployments are capped at 1,000. Add a license to lift the cap.",
    );
    assert.equal(
        tn("sponsor.limit.exceeded", 137),
        "137 mapped circuits are not being created because unlicensed deployments are capped at 1,000. Add a license to lift the cap.",
    );

    installBrowser(ES, "es");
    assert.equal(
        t("sponsor.limit.approaching"),
        "Las instalaciones sin licencia tienen un límite de 1000 circuitos asignados, y este sistema se está acercando a ese límite. Los circuitos que lo superan no se crean.",
    );
    assert.equal(
        tn("sponsor.limit.exceeded", 1),
        "1 circuito asignado no se está creando porque las instalaciones sin licencia tienen un límite de 1000. Añada una licencia para eliminar el límite.",
    );
    assert.equal(
        tn("sponsor.limit.exceeded", 137),
        "137 circuitos asignados no se están creando porque las instalaciones sin licencia tienen un límite de 1000. Añada una licencia para eliminar el límite.",
    );
});
