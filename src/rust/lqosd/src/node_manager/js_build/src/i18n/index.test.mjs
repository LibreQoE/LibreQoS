// Contract tests for the i18n runtime. Node built-ins only: the fake browser
// below is the minimum index.js needs (window catalog, localStorage, document).

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {applyDomTranslations, t, tAttr, tHtml, tn} from "./index.js";
import {formatDate, formatDateTime, formatNumber, formatTime, relativeTime} from "./format.js";
import {intlTag} from "./intl.js";
import {locale, servedCatalog, setLocale} from "./locale.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

// A key no catalog defines. Built at runtime so the extractor does not read it
// as a usage site that is missing from en.json.
const ABSENT_KEY = ["common", "nav", "absent"].join(".");
// The same absence, carrying markup: only an escaped fallback can render this
// as `&lt;absent&gt;`, so the assertion can tell escaping from a raw return.
const ABSENT_MARKUP_KEY = ["common", "nav", "<absent>"].join(".");
// Shaped like a language tag, so it reaches Intl, which rejects it.
const REJECTED_TAG = "qzx-QZ";
// Local noon, so the date assertions do not move with the host time zone.
const DATE = new Date(2026, 0, 31, 12, 0);

class FakeText {
    constructor(data) {
        this.nodeType = 3;
        this.data = data;
    }
}

class FakeElement {
    constructor(attributes, textContent = null) {
        this.attributes = {...attributes};
        this.textContent = textContent;
        this.childNodes = [];
        this.children = [];
    }

    getAttribute(name) {
        return Object.prototype.hasOwnProperty.call(this.attributes, name) ? this.attributes[name] : null;
    }

    setAttribute(name, value) {
        this.attributes[name] = value;
    }
}

/** An element that owns markup: `children` holds its element nodes, as in the DOM. */
function elementWithChildren(attributes, nodes) {
    const element = new FakeElement(attributes);
    element.childNodes = nodes;
    element.children = nodes.filter((node) => node instanceof FakeElement);
    return element;
}

function fakeRoot(elements) {
    return {
        querySelectorAll(selector) {
            const attribute = selector.slice(1, -1);
            return elements.filter((element) => element.getAttribute(attribute) !== null);
        },
    };
}

function installBrowser({catalog = null, fallback = null, stored = null, lang = ""} = {}) {
    const storage = new Map();
    if (stored) storage.set("uiLanguage", stored);
    // Only the globals the page actually loaded are set: `en.js` publishes
    // `LQOS_I18N_EN` alone, and a page served another locale adds `LQOS_I18N`.
    globalThis.window = {};
    if (catalog) globalThis.window.LQOS_I18N = catalog;
    if (fallback) globalThis.window.LQOS_I18N_EN = fallback;
    globalThis.localStorage = {
        getItem: (key) => (storage.has(key) ? storage.get(key) : null),
        setItem: (key, value) => storage.set(key, value),
    };
    globalThis.document = {documentElement: {lang}, cookie: ""};
    return storage;
}

test.afterEach(() => {
    delete globalThis.window;
    delete globalThis.localStorage;
    delete globalThis.document;
});

test("the seeded catalogs agree on keys and translate the shell vocabulary", () => {
    assert.deepEqual(Object.keys(ES.messages).sort(), Object.keys(EN.messages).sort());
    installBrowser({catalog: ES});
    assert.equal(t("common.nav.dashboard"), "Panel");
    assert.equal(t("common.nav.tree"), "Árbol");
    assert.equal(t("common.nav.topology"), "Topología");
    assert.equal(t("common.action.save"), "Guardar");
    assert.equal(t("common.action.cancel"), "Cancelar");
    assert.equal(t("common.action.close"), "Cerrar");
    assert.equal(t("common.search.placeholder"), "Buscar…");
    assert.equal(t("common.language.label"), "Idioma");
    assert.equal(t("common.state.loading"), "Cargando…");
    assert.equal(t("common.state.empty"), "No hay datos para mostrar");
    assert.equal(
        t("server.import_failing", {integration: "UISP"}),
        "Shaping activo con la última topología válida · falla la importación de UISP",
    );
});

test("t() fills named placeholders and leaves unknown ones visible", () => {
    installBrowser({catalog: ES});
    assert.equal(t("common.confirm.delete", {name: "Cliente 7"}), "¿Eliminar Cliente 7?");
    assert.equal(t("common.confirm.delete"), "¿Eliminar {name}?");
});

test("a null or undefined parameter leaves the placeholder visible in t() and tHtml()", () => {
    installBrowser({catalog: ES});
    assert.equal(t("common.confirm.delete", {name: undefined}), "¿Eliminar {name}?");
    assert.equal(t("common.confirm.delete", {name: null}), "¿Eliminar {name}?");
    assert.equal(
        tHtml("common.help.docs_link", {url: undefined}),
        'Consulte la <a href="{url}">documentación</a> para obtener más detalles.',
    );
});

test("t() returns the key for an unknown message and warns only in debug mode", () => {
    installBrowser({catalog: ES});
    const warnings = [];
    const originalWarn = console.warn;
    console.warn = (...args) => warnings.push(args.join(" "));
    try {
        assert.equal(t(ABSENT_KEY), ABSENT_KEY);
        assert.deepEqual(warnings, []);
        globalThis.window.LQOS_I18N_DEBUG = true;
        assert.equal(t(ABSENT_KEY), ABSENT_KEY);
        assert.equal(warnings.length, 1);
    } finally {
        console.warn = originalWarn;
    }
});

test("a key the active locale lacks falls back to the English catalog", () => {
    installBrowser({
        catalog: {locale: "es", dir: "ltr", messages: {"common.action.save": "Guardar"}},
        fallback: EN,
    });
    assert.equal(t("common.action.save"), "Guardar");
    assert.equal(t("common.action.cancel"), "Cancel");
    assert.equal(t(ABSENT_KEY), ABSENT_KEY);
});

test("a page that loads only LQOS_I18N_EN resolves as English", () => {
    // The build puts `LQOS_I18N_EN` in en.js and `LQOS_I18N` only in a served
    // catalog, so this is the global shape every English page has. The runtime
    // must take its locale, messages and direction from the fallback alone.
    installBrowser({fallback: EN});
    assert.equal(globalThis.window.LQOS_I18N, undefined);
    // The arm itself: without it `servedCatalog()` is null, and the assertions
    // below would still hold because English is also the last-resort locale.
    assert.equal(servedCatalog(), EN);
    assert.equal(locale(), "en");
    assert.equal(t("common.nav.tree"), "Tree");
});

test("tn() selects the English plural category and formats the count", () => {
    installBrowser({catalog: EN});
    assert.equal(tn("tree.limit.excess_devices", 1), "1 device not shown");
    assert.equal(tn("tree.limit.excess_devices", 0), "0 devices not shown");
    assert.equal(tn("tree.limit.excess_devices", 5), "5 devices not shown");
    assert.equal(tn("tree.limit.excess_devices", 1234), "1,234 devices not shown");
});

test("tn() follows the Spanish plural rule for one, many and other", () => {
    installBrowser({catalog: ES});
    const rules = new Intl.PluralRules("es");
    assert.equal(rules.select(1), "one");
    assert.equal(rules.select(11), "other");
    assert.equal(rules.select(1_000_000), "many");
    assert.equal(tn("tree.limit.excess_devices", 1), "Dispositivo no mostrado: 1");
    assert.equal(tn("tree.limit.excess_devices", 11), "Dispositivos no mostrados: 11");
    // `many` is the exact million multiples only: 1.000.001 stays in `other`.
    assert.equal(tn("tree.limit.excess_devices", 1_000_000), "Dispositivos no mostrados: 1.000.000");
    assert.equal(tn("tree.limit.excess_devices", 1_000_001), "Dispositivos no mostrados: 1.000.001");
});

test("tn() falls back to \"other\" for a category the catalog does not define", () => {
    // A catalog that predates the policy's explicit `many` row still renders:
    // Spanish selects "many" in the millions, so `other` must stand in for it.
    installBrowser({
        catalog: {
            locale: "es",
            dir: "ltr",
            messages: {"tree.limit.excess_devices": {one: "{count} dispositivo", other: "{count} dispositivos"}},
        },
    });
    assert.equal(new Intl.PluralRules("es").select(1_000_000), "many");
    assert.equal(tn("tree.limit.excess_devices", 1_000_000), "1.000.000 dispositivos");
});

test("the scheduler's consecutive-failure count pluralizes in both locales", () => {
    const key = "scheduler.integration.consecutive_failures";
    installBrowser({catalog: EN});
    assert.equal(tn(key, 1), "1 consecutive failure");
    assert.equal(tn(key, 2), "2 consecutive failures");
    installBrowser({catalog: ES});
    assert.equal(tn(key, 1), "Fallo consecutivo: 1");
    assert.equal(tn(key, 2), "Fallos consecutivos: 2");
    // The status line is read in the millions too: `many` must not fall back
    // to an English-shaped count of failures.
    assert.equal(new Intl.PluralRules("es").select(1_000_000), "many");
    assert.equal(tn(key, 1_000_000), "Fallos consecutivos: 1.000.000");
});

test("the tree and circuit counts pluralize in both locales, including many", () => {
    // S2b: the tree's branch-size pill and the circuit's ancestor count. Both
    // are counts of entities, so both go through tn(); Spanish selects `many`
    // for exact millions, and the label form keeps the number readable there.
    const keys = ["tree.branch.circuits", "circuit.queue_tree.upstream_count"];
    installBrowser({catalog: EN});
    assert.equal(tn(keys[0], 1), "1 circuit");
    assert.equal(tn(keys[0], 2), "2 circuits");
    assert.equal(tn(keys[1], 1), "1 upstream node");
    assert.equal(tn(keys[1], 2), "2 upstream nodes");

    installBrowser({catalog: ES});
    assert.equal(tn(keys[0], 1), "Circuito: 1");
    assert.equal(tn(keys[0], 2), "Circuitos: 2");
    assert.equal(tn(keys[1], 1), "Nodo ascendente: 1");
    assert.equal(tn(keys[1], 2), "Nodos ascendentes: 2");
    assert.equal(new Intl.PluralRules("es").select(1_000_000), "many");
    assert.equal(tn(keys[0], 1_000_000), "Circuitos: 1.000.000");
    assert.equal(tn(keys[1], 1_000_000), "Nodos ascendentes: 1.000.000");
    // The million-multiple rule is exact: one more is `other`.
    assert.equal(tn(keys[0], 1_000_001), "Circuitos: 1.000.001");
});

test("a long scheduler sentence keeps both placeholders in both locales", () => {
    const key = "scheduler.transport.cached";
    installBrowser({catalog: EN});
    assert.equal(
        t(key, {message: "System Health response was empty.", updated: "Updated 5 sec. ago"}),
        "System Health response was empty. Showing cached scheduler data. Updated 5 sec. ago.",
    );
    installBrowser({catalog: ES});
    assert.equal(
        t(key, {message: "La respuesta del estado del sistema llegó vacía.", updated: "Última actualización: hace 5 s"}),
        "La respuesta del estado del sistema llegó vacía. Se muestran datos del planificador en caché. Última actualización: hace 5 s.",
    );
});

test("tn() returns the key rather than throwing when the message is not plural", () => {
    installBrowser({catalog: ES});
    assert.equal(tn("common.nav.tree", 2), "common.nav.tree");
});

test("tHtml() keeps allowlisted markup and escapes parameters", () => {
    installBrowser({catalog: EN});
    assert.equal(
        tHtml("common.help.docs_link", {url: "https://example.test/a?b=1&c=2"}),
        'See the <a href="https://example.test/a?b=1&amp;c=2">documentation</a> for details.',
    );
    assert.match(tHtml("common.help.docs_link", {url: 'x" onmouseover="alert(1)'}), /href="x&quot; onmouseover=&quot;alert\(1\)"/);
    // The single escaper encodes `'` as `&apos;`; a second implementation with
    // `&#39;` would show up here.
    assert.match(tHtml("common.help.docs_link", {url: "https://example.test/it's"}), /href="https:\/\/example\.test\/it&apos;s"/);
});

test("tHtml() escapes everything that is not on the allowlist", () => {
    installBrowser({
        catalog: {
            locale: "en",
            dir: "ltr",
            messages: {
                "common.state.empty": "<strong>bold</strong> <script>alert(1)</script>",
                "common.state.loading": "Open <b>{name}</b>",
            },
        },
    });
    assert.equal(
        tHtml("common.state.empty"),
        "<strong>bold</strong> &lt;script&gt;alert(1)&lt;/script&gt;",
    );
    assert.equal(tHtml("common.state.loading", {name: "<i>x</i>"}), "Open &lt;b&gt;&lt;i&gt;x&lt;/i&gt;&lt;/b&gt;");
});

test("applyDomTranslations() writes text and attributes, and keeps English when a key is missing", () => {
    installBrowser({catalog: ES});
    const text = new FakeElement({"data-i18n": "common.nav.dashboard"}, "Dashboard");
    const title = new FakeElement({"data-i18n-title": "common.action.close", title: "Close"});
    const placeholder = new FakeElement({"data-i18n-placeholder": "common.search.placeholder", placeholder: "Search..."});
    const ariaLabel = new FakeElement({"data-i18n-aria-label": "common.action.save", "aria-label": "Save"});
    const alt = new FakeElement({"data-i18n-alt": "common.nav.tree", alt: "Tree"});
    const missing = new FakeElement({"data-i18n": ABSENT_KEY}, "English stays");
    const plural = new FakeElement({"data-i18n": "tree.limit.excess_devices"}, "3 devices not shown");
    const root = fakeRoot([text, title, placeholder, ariaLabel, alt, missing, plural]);

    assert.equal(applyDomTranslations(root), 5);
    assert.equal(text.textContent, "Panel");
    assert.equal(title.getAttribute("title"), "Cerrar");
    assert.equal(placeholder.getAttribute("placeholder"), "Buscar…");
    assert.equal(ariaLabel.getAttribute("aria-label"), "Guardar");
    assert.equal(alt.getAttribute("alt"), "Árbol");
    assert.equal(missing.textContent, "English stays");
    assert.equal(plural.textContent, "3 devices not shown");
});

test("applyDomTranslations() keeps element children and translates only the element's own text", () => {
    installBrowser({catalog: ES});
    const icon = new FakeElement({class: "fa fa-save"});
    const button = elementWithChildren({"data-i18n": "common.action.save"}, [icon, new FakeText(" Save")]);
    const labelled = elementWithChildren({"data-i18n": "common.action.cancel"}, [
        new FakeElement({class: "fa fa-times"}),
        new FakeText(" Cancel "),
        new FakeElement({class: "badge"}),
    ]);
    const iconOnly = elementWithChildren({"data-i18n": "common.action.close"}, [new FakeElement({class: "fa fa-x"})]);
    const root = fakeRoot([button, labelled, iconOnly]);

    // The icon-only element has no text of its own, so it is left alone.
    assert.equal(applyDomTranslations(root), 2);
    // The subtree is never replaced: the icon stays and only the text node moves.
    assert.equal(button.textContent, null);
    assert.equal(button.children[0], icon);
    assert.equal(button.childNodes[1].data, " Guardar");
    assert.equal(labelled.children[0].getAttribute("class"), "fa fa-times");
    assert.equal(labelled.children[1].getAttribute("class"), "badge");
    assert.equal(labelled.childNodes[1].data, " Cancelar ");
    assert.equal(iconOnly.textContent, null);
    assert.equal(iconOnly.childNodes.length, 1);
});

test("the replaced text keeps the whitespace that spaces an icon from its label", () => {
    installBrowser({catalog: ES});
    const loading = elementWithChildren({"data-i18n": "common.state.loading"}, [
        new FakeElement({class: "fa fa-spinner fa-spin"}),
        new FakeText(" Loading..."),
    ]);
    const padded = elementWithChildren({"data-i18n": "common.action.save"}, [
        new FakeElement({class: "fa fa-save"}),
        new FakeText("\n            Save\n        "),
    ]);

    assert.equal(applyDomTranslations(fakeRoot([loading, padded])), 2);
    // `<i></i> Loading...` must not render the spinner flush against the word.
    assert.equal(loading.childNodes[1].data, " Cargando…");
    // The indentation the author wrote around the label survives too.
    assert.equal(padded.childNodes[1].data, "\n            Guardar\n        ");
});

test("applyDomTranslations() without a root translates nothing", () => {
    assert.equal(applyDomTranslations(), 0);
    assert.equal(applyDomTranslations(null), 0);
});

test("tAttr() escapes a message for a quoted attribute and keeps placeholders filled", () => {
    installBrowser({
        catalog: {locale: "en", dir: "ltr", messages: {"common.nav.tree": 'Say "hi" & <bye>'}},
    });
    assert.equal(tAttr("common.nav.tree"), "Say &quot;hi&quot; &amp; &lt;bye&gt;");
    // A parameter that carries a quote cannot close the attribute it sits in.
    installBrowser({catalog: EN});
    assert.match(tAttr("common.help.docs_link", {url: 'x" onmouseover="alert(1)'}), /&quot;/);
    assert.doesNotMatch(tAttr("common.help.docs_link", {url: 'x" onmouseover="alert(1)'}), /"/);
    // Numbers still come from Intl: the escape runs after the message renders.
    installBrowser({catalog: ES});
    assert.equal(tAttr("circuit.queue_tree.ancestor_position", {position: 12345, total: 2}), "Ancestro 12.345 / 2");
});

test("locale() prefers the served catalog, then storage, then <html lang>, then English", () => {
    installBrowser({catalog: ES, stored: "en", lang: "fr"});
    assert.equal(locale(), "es");
    installBrowser({stored: "es", lang: "fr"});
    assert.equal(locale(), "es");
    installBrowser({lang: "es-MX"});
    assert.equal(locale(), "es-MX");
    installBrowser();
    assert.equal(locale(), "en");
});

test("setLocale() stores the choice in localStorage and in the lqos_lang cookie", () => {
    const storage = installBrowser({catalog: EN});
    setLocale("es");
    assert.equal(storage.get("uiLanguage"), "es");
    assert.match(globalThis.document.cookie, /^lqos_lang=es;path=\/;max-age=\d+;SameSite=Lax$/);
});

test("setLocale() ignores an empty tag instead of clearing the choice", () => {
    const storage = installBrowser({catalog: EN});
    setLocale("");
    setLocale("   ");
    assert.equal(storage.size, 0);
    assert.equal(globalThis.document.cookie, "");
});

test("tHtml() falls back to the escaped key, not to undefined or raw markup", () => {
    installBrowser({catalog: EN});
    assert.equal(tHtml(ABSENT_KEY), ABSENT_KEY);
    // `<absent>` must arrive escaped: a raw return would put a tag into innerHTML.
    assert.equal(tHtml(ABSENT_MARKUP_KEY), "common.nav.&lt;absent&gt;");
});

test("format helpers use the active locale", () => {
    installBrowser({catalog: EN});
    assert.equal(relativeTime(-5, "minute"), "5 minutes ago");
    assert.equal(formatNumber(3.5), "3.5");

    installBrowser({catalog: ES});
    assert.match(relativeTime(-3, "day"), /^hace 3 d/);
    assert.equal(formatNumber(3.5), "3,5");
});

test("formatDate() and formatDateTime() follow the active locale", () => {
    installBrowser({catalog: EN});
    const englishDate = formatDate(DATE);
    const englishDateTime = formatDateTime(DATE);
    assert.equal(englishDate, "Jan 31, 2026");
    assert.match(englishDateTime, /^Jan 31, 2026, 12:00/);

    installBrowser({catalog: ES});
    assert.equal(formatDate(DATE), "31 ene 2026");
    assert.equal(formatDateTime(DATE), "31 ene 2026, 12:00");
    assert.notEqual(formatDate(DATE), englishDate);
    assert.notEqual(formatDateTime(DATE), englishDateTime);
});

test("formatTime() renders a locale-appropriate clock, and no date", () => {
    installBrowser({catalog: EN});
    const englishTime = formatTime(DATE);
    assert.match(englishTime, /^12:00/);
    // A Date and its epoch milliseconds are the same instant.
    assert.equal(formatTime(DATE.getTime()), englishTime);

    installBrowser({catalog: ES});
    // Spanish is 24-hour: no AM/PM and no date part.
    assert.equal(formatTime(DATE), "12:00");
    assert.notEqual(formatTime(DATE), englishTime);
});

test("the Intl formatters survive every locale value the browser can hand them", () => {
    const formatEverything = () => {
        relativeTime(-5, "minute");
        formatNumber(1234.5);
        formatDate(DATE);
        formatDateTime(DATE);
        formatTime(DATE);
        tn("tree.limit.excess_devices", 5);
    };

    for (const stored of ["not a locale", "zz-ZZ", ""]) {
        installBrowser({stored});
        assert.doesNotThrow(formatEverything, `stored locale ${JSON.stringify(stored)} must not throw`);
    }
    // A value Intl rejects renders English instead of throwing inside a render.
    installBrowser({stored: "not a locale"});
    assert.equal(relativeTime(-5, "minute"), "5 minutes ago");
    assert.equal(formatDate(DATE), "Jan 31, 2026");

    installBrowser({stored: "es"});
    assert.equal(formatDate(DATE), "31 ene 2026");
    installBrowser({stored: "es-MX"});
    assert.doesNotThrow(formatEverything);
});

// The S2a pages count devices, circuits, failed probes and sharded workers.
// Spanish selects `many` only for exact positive millions, so the count text
// has to come from the category the number picks, not from `count === 1`.
test("the page counts pluralize in Spanish, millions included", () => {
    installBrowser({catalog: ES});
    assert.equal(tn("shaped_devices.device_count", 1), "Dispositivo: 1");
    assert.equal(tn("shaped_devices.device_count", 2), "Dispositivos: 2");
    assert.equal(tn("shaped_devices.device_count", 1_000_000), "Dispositivos: 1.000.000");
    assert.equal(tn("bridge.page.failed_probes", 1), "Sonda fallida consecutiva: 1");
    assert.equal(tn("bridge.page.failed_probes", 1_000_000), "Sondas fallidas consecutivas: 1.000.000");
    // A label keeps its plural wording at any count, including one.
    assert.equal(tn("cpu_workers.sharded_badge", 3), "con sharding · workers: 3");
    assert.equal(tn("cpu_workers.sharded_badge", 1), "con sharding · workers: 1");
});

test("a count inside a page sentence is grouped, not abbreviated", () => {
    installBrowser({catalog: ES});
    assert.equal(t("cpu_workers.owned_summary", {count: 12_345, sites: 2}), "Circuitos: 12.345 · sitios/AP: 2");
    assert.equal(t("shaped_devices.showing", {start: 1, end: 24, total: 12_345}), "Mostrando 1–24 de 12.345");
});

test("intlTag re-probes a rejected tag instead of memoizing the rejection", () => {
    const OriginalDateTimeFormat = Intl.DateTimeFormat;
    let probes = 0;
    Intl.DateTimeFormat = function (tag) {
        probes += 1;
        if (tag === REJECTED_TAG) throw new RangeError("invalid language tag");
        return new OriginalDateTimeFormat(tag);
    };
    try {
        assert.equal(intlTag(REJECTED_TAG), "en");
        assert.equal(intlTag(REJECTED_TAG), "en");
        // The tag behind `locale()` can change, so a rejection is never cached.
        assert.equal(probes, 2, "a rejected candidate must be probed again");
    } finally {
        Intl.DateTimeFormat = OriginalDateTimeFormat;
    }
});
