// Pinning tests for the last translated surfaces (section S5b): Help and its
// support-ticket modals, the runtime setup wizard, the API overview and its
// Swagger frame, the StormGuard debug page, Ethernet caps and the whole-tree
// Sankey.
//
// These pages are the ones an operator reads when something is wrong, so the
// Spanish is pinned sentence by sentence: a harmless-looking rewording is what
// turns a licence requirement into a promise, or a diagnostic into advice. The
// page contract already proves each key reaches its file; this file fixes the
// wording that reaches the screen.
//
// Four kinds of string stay out of the catalog on purpose, and each is checked
// from the source instead of the catalog: the two server reasons `help.js`
// compares to reveal the licence gate, the target-kind token `ethernet_caps.js`
// compares, the CSS values the Sankey overlay applies, and the server's own
// onboarding label, which `setup_runtime.js` shows as data.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {t, tn} from "./index.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const NODE_MANAGER = resolve(HERE, "../../..");
const CATALOG_DIR = resolve(NODE_MANAGER, "static2/i18n");
const STATIC_DIR = resolve(NODE_MANAGER, "static2");
const SOURCE_DIR = resolve(NODE_MANAGER, "js_build/src");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

const HELP_MARKUP = readFileSync(resolve(STATIC_DIR, "help.html"), "utf8");
const SETUP_MARKUP = readFileSync(resolve(STATIC_DIR, "setup_runtime.html"), "utf8");
const API_MARKUP = readFileSync(resolve(STATIC_DIR, "api.html"), "utf8");
const API_DOCS_MARKUP = readFileSync(resolve(STATIC_DIR, "api_docs.html"), "utf8");
const STORMGUARD_MARKUP = readFileSync(resolve(STATIC_DIR, "stormguard_debug.html"), "utf8");
const ETHERNET_MARKUP = readFileSync(resolve(STATIC_DIR, "ethernet_caps.html"), "utf8");
const SANKEY_MARKUP = readFileSync(resolve(STATIC_DIR, "all_tree_sankey.html"), "utf8");

const HELP_BUNDLE = readFileSync(resolve(SOURCE_DIR, "help.js"), "utf8");
const SETUP_BUNDLE = readFileSync(resolve(SOURCE_DIR, "setup_runtime.js"), "utf8");
const STORMGUARD_BUNDLE = readFileSync(resolve(SOURCE_DIR, "stormguard_debug.js"), "utf8");
const ETHERNET_BUNDLE = readFileSync(resolve(SOURCE_DIR, "ethernet_caps.js"), "utf8");
const SANKEY_BUNDLE = readFileSync(resolve(SOURCE_DIR, "all_tree_sankey.js"), "utf8");

/** The minimum `index.js` and `locale.mjs` need: the catalogs and a document. */
function installBrowser(catalog, lang) {
    globalThis.window = {LQOS_I18N_EN: EN};
    if (lang !== "en") globalThis.window.LQOS_I18N = catalog;
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang}, cookie: ""};
}

/** Asserts each `key → Spanish` pair renders exactly that sentence. */
function assertSpanish(pairs) {
    installBrowser(ES, "es");
    for (const [key, spanish] of pairs) {
        assert.equal(t(key), spanish, `${key} must render the ruled Spanish`);
    }
}

test("the help page and its ticket modals keep their ruled Spanish", () => {
    assertSpanish([
        [
            "help.tickets.subtitle",
            "El soporte prioritario está disponible en los niveles de licencia con derechos de uso.",
        ],
        [
            "help.tickets.license_required",
            "Para enviar un ticket de soporte necesita una licencia con derechos de uso.",
        ],
        ["help.ticket.new_title", "Nuevo ticket de soporte"],
        ["help.ticket.subject", "Asunto"],
        [
            "help.ticket.details_placeholder",
            "Describa el problema, lo que esperaba y lo que ocurrió.",
        ],
        ["help.ticket.priority_lowest", "0 - La más baja"],
        ["help.ticket.priority_highest", "5 - La más alta"],
        ["help.ticket.submit", "Enviar ticket"],
        ["help.tickets.empty", "Todavía no hay tickets."],
        ["help.comments.empty_error", "El comentario no puede estar vacío."],
        ["help.docs.action", "Leer la documentación de LibreQoS"],
        ["help.chat.link", "Sistema de chat Zulip"],
        ["help.about.memorial", "En memoria de Dave Täht"],
        ["help.social.fosstodon", "Fosstodon (Mastodon libre y de código abierto)"],
    ]);

    // The two reasons the server sends are matched, never re-rendered from the
    // literal: translating either one would silently disable the licence gate.
    assert.match(HELP_BUNDLE, /message\.includes\("Support tickets require an entitled license\."\)/);
    assert.match(HELP_BUNDLE, /message\.includes\("license valid, control service unavailable"\)/);
    // The ticket list prints catalogued empty and loading states, not English.
    assert.match(HELP_BUNDLE, /t\("help\.tickets\.empty"\)/);
    assert.match(HELP_BUNDLE, /t\("help\.tickets\.loading"\)/);
    // Brand names stay literal on the community card.
    assert.match(HELP_MARKUP, /translatable: false — "Twitter" "Facebook"/);
});

test("the runtime setup wizard keeps its ruled Spanish and shows the server state as data", () => {
    assertSpanish([
        ["setup_runtime.title", "Completar la configuración"],
        [
            "setup_runtime.subtitle",
            "LibreQoS está en ejecución, pero aún necesita una fuente de topología antes de que el planificador pueda crear las colas de los suscriptores.",
        ],
        [
            "setup_runtime.integration.note",
            "Guarde allí la configuración de su proveedor y vuelva aquí si el planificador sigue indicando que falta configuración.",
        ],
        [
            "setup_runtime.custom.step_integration",
            "Defina el comportamiento compartido de la topología en Fuentes de datos.",
        ],
        ["setup_runtime.manual.title", "Editar manualmente la lista de suscriptores"],
        ["setup_runtime.state.title", "Estado detectado en ejecución"],
        ["setup_runtime.status.setup_required", "Configuración pendiente"],
        [
            "setup_runtime.summary.choose_source",
            "Elija una fuente de topología antes de esperar actividad del planificador.",
        ],
        ["setup_runtime.present", "Presente"],
        ["setup_runtime.missing", "No presente"],
        ["setup_runtime.none_configured", "Ninguna configurada"],
    ]);

    // `runtime_onboarding.rs` owns the banner text; the page must keep sending it
    // through unchanged, with the catalogued fallback only when it is absent.
    assert.match(SETUP_BUNDLE, /state\?\.status_label \|\| t\("setup_runtime\.status\.setup_required"\)/);
    assert.match(SETUP_BUNDLE, /state\?\.summary \|\| t\("setup_runtime\.summary\.choose_source"\)/);
    // The three runtime file readings are catalogued, not English literals.
    assert.match(SETUP_BUNDLE, /t\("setup_runtime\.present"\) : t\("setup_runtime\.missing"\)/);
});

test("the API overview translates the prose and leaves every technical token literal", () => {
    assertSpanish([
        ["api.title", "API de LibreQoS y Libby"],
        ["api.subtitle", "— respuestas más rápidas, mejor visibilidad"],
        ["api.what.title", "Qué puede hacer"],
        [
            "api.what.libby",
            "Pregunte a Libby lo que necesite: primero consulta las herramientas y después explica los resultados en lenguaje sencillo.",
        ],
        ["api.start.title", "Primeros pasos"],
        [
            "api.start.part3",
            "en el mismo puerto de escucha que esta página, así que no hay nada más que instalar ni habilitar. Todas las solicitudes necesitan una clave de API local en la cabecera",
        ],
        ["api.start.keys_link", "Licencia y servicios → Claves de API locales"],
        [
            "api.start.part5",
            "(la clave se muestra una sola vez y solo se guarda su resumen).",
        ],
        ["api_docs.title", "Documentación de la API local"],
        ["api_docs.open", "Abrir en una pestaña nueva"],
        ["api_docs.frame_title", "Documentación de la API de LibreQoS"],
    ]);

    // The endpoint, the header name and the schema URL are literals in the
    // markup: an operator copies them, so they never enter a message.
    assert.match(API_MARKUP, /<code>lqosd<\/code>/);
    assert.match(API_MARKUP, /<code>\/api\/v3<\/code>/);
    assert.match(API_MARKUP, /<code>x-bearer<\/code>/);
    assert.match(API_MARKUP, /<code>\/api\/v3\/openapi\.json<\/code>/);
    assert.match(API_DOCS_MARKUP, /src="\/api\/v3\/docs\/"/);
    // The one path the prose names survives translation unchanged.
    for (const catalog of [EN, ES]) {
        assert.match(catalog.messages["api_docs.subtitle"], /\/api\/v3\/docs$/);
    }
    assert.match(API_MARKUP, /<code>\/api\/v3<\/code>/);
    assert.match(API_MARKUP, /<code>x-bearer<\/code>/);
    assert.match(API_DOCS_MARKUP, /src="\/api\/v3\/docs\/"/);
});

test("the StormGuard debug page keeps its ruled Spanish and leaves server tokens alone", () => {
    assertSpanish([
        ["stormguard_debug.title", "Depuración de StormGuard"],
        [
            "stormguard_debug.lead",
            "Límites HTB en vivo y las métricas que evalúa StormGuard (tasa de transferencia, pérdida, RTT, saturación, estado). Las tasas llegan de",
        ],
        ["stormguard_debug.lead_inputs", "y las entradas de"],
        ["stormguard_debug.current_limits", "Límites actuales"],
        ["stormguard_debug.evaluation", "Datos de evaluación"],
        ["stormguard_debug.column.dir", "Sentido"],
        ["stormguard_debug.column.queue_range", "Cola (mín./máx.)"],
        ["stormguard_debug.column.saturation_range", "Sat. (act./máx.)"],
        ["stormguard_debug.dir.down", "Desc."],
        ["stormguard_debug.dir.up", "Sub."],
        ["stormguard_debug.baseline", "Línea base"],
        ["stormguard_debug.status.connecting", "Conectando…"],
        ["stormguard_debug.status.live", "En vivo"],
        ["stormguard_debug.status.disconnected", "Desconectado"],
        ["stormguard_debug.empty_status", "No hay datos de StormGuard disponibles."],
        ["stormguard_debug.empty_debug", "No hay datos de depuración disponibles."],
    ]);

    // The state and action columns print the server's own tokens; the S4 ruling
    // keeps state identifiers untranslated, so the bundle must not map them.
    assert.match(STORMGUARD_BUNDLE, /escapeHtml\(d\.state \|\| "\?"\)/);
    assert.match(STORMGUARD_BUNDLE, /escapeHtml\(d\.last_action\)/);
    // Values arrive from the server, so each one crosses the HTML boundary escaped.
    assert.match(STORMGUARD_BUNDLE, /import \{escapeHtml\} from "\.\/i18n\/escape\.mjs"/);
    for (const field of ["entry.site", "d.saturation_current", "d.strategy"]) {
        assert.match(
            STORMGUARD_BUNDLE,
            new RegExp(`escapeHtml\\(${field.replace(".", "\\.")}`),
            `${field} must be escaped before it reaches innerHTML`,
        );
    }
    // The header names the protocol tokens literally.
    assert.match(STORMGUARD_MARKUP, /<th scope="col">RTT<\/th>/);
});

test("the Ethernet caps page keeps its ruled Spanish and the transport vocabulary literal", () => {
    assertSpanish([
        ["ethernet_caps.title", "Límites de Ethernet"],
        [
            "ethernet_caps.subtitle",
            "Circuitos y nodos de topología cuya tasa se redujo automáticamente porque la velocidad de Ethernet detectada era inferior a la tasa solicitada.",
        ],
        ["ethernet_caps.column.target", "Destino"],
        ["ethernet_caps.column.requested", "Solicitada"],
        ["ethernet_caps.column.applied", "Aplicada"],
        ["ethernet_caps.column.limiting_device", "Dispositivo limitante"],
        ["ethernet_caps.none_active", "No hay límites de Ethernet activos"],
        [
            "ethernet_caps.load_failed",
            "No se pudieron cargar los datos de límites de Ethernet. Compruebe la conexión con el administrador de nodos y vuelva a intentarlo.",
        ],
        [
            "ethernet_caps.showing",
            "Mostrando {start}–{end} de {total} • Página {page} de {pages}",
        ],
    ]);

    // The count line is a whole message with its number named, not a numeral
    // glued to an English suffix: one, other and the million category all agree.
    installBrowser(ES, "es");
    assert.equal(tn("ethernet_caps.targets_count", 1), "Destino limitado por Ethernet: 1");
    assert.equal(tn("ethernet_caps.targets_count", 2), "Destinos limitados por Ethernet: 2");
    assert.equal(tn("ethernet_caps.targets_count", 1_000_000), "Destinos limitados por Ethernet: 1.000.000");

    // The tier filter options are port speeds, not prose.
    for (const tier of ["10M", "100M", "1G+"]) {
        assert.match(ETHERNET_MARKUP, new RegExp(`<option[^>]*>${tier.replace("+", "\\+")}</option>`));
    }
    // The target kind is a server token compared here and labelled from the catalog.
    assert.match(ETHERNET_BUNDLE, /row\?\.target_kind === "Node"/);
    assert.match(ETHERNET_BUNDLE, /t\("tree\.table\.node"\) : t\("common\.table\.circuit"\)/);
});

test("the Sankey keeps its ruled chart vocabulary and its waiting states", () => {
    assertSpanish([
        ["sankey.top_node", "Nodo superior:"],
        ["sankey.max_depth", "Profundidad máxima:"],
        ["sankey.overlay.limited_title", "Tasa de transferencia limitada"],
        ["sankey.overlay.nothing", "Aún no hay nada que representar."],
        ["sankey.overlay.requesting", "Solicitando el árbol de red…"],
        [
            "sankey.overlay.no_response",
            "Aún no se ha recibido ninguna respuesta del websocket NetworkTreeLite.",
        ],
        ["graph.sankey.root", "Raíz"],
    ]);

    // The overlay's panel is styled by CSS values, and the server's synthetic
    // node suffix and the redaction font stay literal.
    assert.match(SANKEY_BUNDLE, /translatable: false — "var\(--lqos-surface\)"/);
    assert.match(SANKEY_BUNDLE, /params\.name\.replace\("\(Generated Site\) ", ""\)/);
    assert.match(SANKEY_BUNDLE, /label\.fontFamily = "Illegible"/);
    // Pause and resume keep the dashboard's own action words.
    assert.match(SANKEY_BUNDLE, /t\("dashboard\.resume"\)/);
    assert.match(SANKEY_BUNDLE, /t\("dashboard\.pause"\)/);
    assert.match(SANKEY_MARKUP, /data-i18n="dashboard\.pause"/);
});
