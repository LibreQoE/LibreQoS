// The topology manager's node kinds must come from the catalog.
//
// A label derived from a `switch` on the node id would render English under
// every locale and still pass the gates: `extract.mjs` only knows a key has a
// reader when a call site names one, and a hardcoded branch names nothing. These
// tests render the shipped catalogs — then a catalog whose Spanish value is
// deliberately unreachable from any English branch — so the label can only come
// from whatever catalog the page loaded.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {nodeKindLabel, nodeKindPriority, ROOT_NODE_ID} from "./topology_labels.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../static2/i18n");
const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

function installCatalogs(served = null) {
    globalThis.window = served ? {LQOS_I18N_EN: EN, LQOS_I18N: served} : {LQOS_I18N_EN: EN};
}

test.afterEach(() => {
    delete globalThis.window;
});

test("the node kinds render in Spanish when the served catalog is Spanish", () => {
    installCatalogs(ES);
    assert.equal(nodeKindLabel(null), "Nodo");
    assert.equal(nodeKindLabel(ROOT_NODE_ID), "Raíz");
    assert.equal(nodeKindLabel("uisp:site:abc"), "Sitio");
    assert.equal(nodeKindLabel("uisp:device:abc"), "Dispositivo");
    assert.equal(nodeKindLabel("libreqos:generated:1"), "Generado");
    assert.equal(nodeKindLabel("some-other-id"), "Nodo");
});

test("the kind label follows the loaded catalog, not a hardcoded switch", () => {
    const custom = {
        ...ES,
        messages: {...ES.messages, "topology.node_kind.site": "Sitio de prueba"},
    };
    installCatalogs(custom);
    assert.equal(nodeKindLabel("uisp:site:abc"), "Sitio de prueba");
    // The other kinds still resolve from the same catalog.
    assert.equal(nodeKindLabel(ROOT_NODE_ID), "Raíz");
});

test("a page that loads only the fallback catalog renders English kinds", () => {
    installCatalogs();
    assert.equal(nodeKindLabel("uisp:site:abc"), "Site");
    assert.equal(nodeKindLabel(ROOT_NODE_ID), "Root");
});

test("the ranking still separates sites, generated nodes and devices", () => {
    // Moved here with the kind vocabulary; the search ordering depends on it.
    assert.equal(nodeKindPriority("uisp:site:s"), 3);
    assert.equal(nodeKindPriority("libreqos:generated:1"), 2);
    assert.equal(nodeKindPriority("uisp:device:d"), 1);
    assert.equal(nodeKindPriority("some-other-id"), 0);
    assert.equal(nodeKindPriority(null), 0);
});
