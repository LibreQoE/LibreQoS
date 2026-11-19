// Behaviour tests for the alerts nav label: which catalog key a count selects,
// and that Spanish really inflects it. The catalog contracts compare whole
// messages and placeholder sets, so collapsing `es.json` to a single plural
// variant would pass them; only a rendered label catches it.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {alertsAriaLabel} from "./alerts_aria_label.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");

const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

/** The minimum `i18n/index.js` reads to resolve the active catalog. */
function useCatalog(catalog) {
    globalThis.window = {LQOS_I18N: catalog};
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang: ""}, cookie: ""};
}

test.afterEach(() => {
    delete globalThis.window;
    delete globalThis.localStorage;
    delete globalThis.document;
});

test("English counts the active alerts and names the empty case", () => {
    useCatalog(EN);
    assert.equal(alertsAriaLabel(0), "Alerts: none active");
    assert.equal(alertsAriaLabel(1), "Alerts: 1 active");
    assert.equal(alertsAriaLabel(4), "Alerts: 4 active");
});

test("Spanish inflects the count and names the empty case", () => {
    useCatalog(ES);
    assert.equal(alertsAriaLabel(0), "Alertas: ninguna activa");
    assert.equal(alertsAriaLabel(1), "Alerta activa: 1");
    assert.equal(alertsAriaLabel(4), "Alertas activas: 4");
    // Millions select `many`, which the policy fills with the plural header.
    assert.equal(alertsAriaLabel(1_000_000), "Alertas activas: 1.000.000");
});

test("the Spanish plural message keeps distinct one and many texts", () => {
    const active = ES.messages["common.alerts.aria_active"];
    assert.equal(typeof active, "object", "es.json must carry the plural object");
    assert.ok(active.one && active.many && active.other, "es.json needs one, many and other");
    assert.notEqual(active.one, active.many, "es.json must inflect the alert count");
});
