import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import {test} from "node:test";
import {fileURLToPath} from "node:url";
import {
    applyWorldMapFeatureUpdate,
    worldMapEncodingDescription,
    worldMapStatusText,
} from "./world_map_dashlet_state.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../../static2/i18n");
const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

/** The minimum `index.js` and `locale.mjs` need to resolve a catalog. */
function installBrowser(catalog, lang) {
    globalThis.window = {LQOS_I18N_EN: EN};
    if (lang !== "en") globalThis.window.LQOS_I18N = catalog;
    globalThis.localStorage = {getItem: () => null, setItem: () => {}};
    globalThis.document = {documentElement: {lang}, cookie: ""};
}

function makeDashlet() {
    const graphUpdates = [];
    const traceEvents = [];
    return {
        emptyStates: [],
        statusMessages: [],
        graph: {
            updateFeatures: (features) => graphUpdates.push(features),
        },
        graphUpdates,
        traceEvents,
        _showEmpty(show) {
            this.emptyStates.push(show);
        },
        _setStatus(message) {
            this.statusMessages.push(message);
        },
        traceRender(stage, details) {
            traceEvents.push({ stage, details });
        },
    };
}

test("worldMapStatusText reports the feature count in both locales", () => {
    installBrowser(EN, "en");
    const english = worldMapEncodingDescription();
    assert.equal(worldMapStatusText(3), `Showing 3 recent endpoint locations. ${english}`);
    assert.equal(worldMapStatusText(1), `Showing 1 recent endpoint location. ${english}`);

    installBrowser(ES, "es");
    const spanish = worldMapEncodingDescription();
    assert.equal(worldMapStatusText(3), `Se muestran 3 ubicaciones recientes de extremos. ${spanish}`);
    assert.equal(worldMapStatusText(1), `Se muestra 1 ubicación reciente de extremos. ${spanish}`);
    // The encoding sentence is its own message; a status line that dropped it
    // would still read as a count, and the screen reader would lose the meaning.
    assert.match(spanish, /^Los puntos más grandes indican más tráfico/);
});

test("applyWorldMapFeatureUpdate renders features and announces the latest count", () => {
    installBrowser(EN, "en");
    const dashlet = makeDashlet();
    const features = [
        { type: "Feature", properties: { id: 1 } },
        { type: "Feature", properties: { id: 2 } },
    ];

    applyWorldMapFeatureUpdate(dashlet, features, "EndpointLatLon");

    assert.deepEqual(dashlet.emptyStates, [false]);
    assert.deepEqual(dashlet.graphUpdates, [features]);
    assert.deepEqual(dashlet.statusMessages, [worldMapStatusText(2)]);
    assert.deepEqual(dashlet.traceEvents.map((event) => event.stage), ["onMessage", "update-ok"]);
});

test("applyWorldMapFeatureUpdate clears rendered features on no data", () => {
    const dashlet = makeDashlet();

    applyWorldMapFeatureUpdate(dashlet, [], "EndpointLatLon");

    assert.deepEqual(dashlet.emptyStates, [true]);
    assert.deepEqual(dashlet.graphUpdates, [[]]);
    assert.deepEqual(dashlet.statusMessages, []);
    assert.deepEqual(dashlet.traceEvents, []);
});

test("applyWorldMapFeatureUpdate traces and rethrows graph update errors", () => {
    const dashlet = makeDashlet();
    dashlet.graph.updateFeatures = () => {
        throw new Error("map update failed");
    };

    assert.throws(
        () => applyWorldMapFeatureUpdate(dashlet, [{ type: "Feature" }], "EndpointLatLon"),
        /map update failed/,
    );
    assert.deepEqual(dashlet.traceEvents.map((event) => event.stage), ["onMessage", "update-error"]);
});
