import assert from "node:assert/strict";
import {test} from "node:test";
import {
    applyWorldMapFeatureUpdate,
    WORLD_MAP_ENCODING_DESCRIPTION,
    worldMapStatusText,
} from "./world_map_dashlet_state.mjs";

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

test("worldMapStatusText reports feature count and encodings", () => {
    assert.equal(
        worldMapStatusText(3),
        `Showing 3 recent endpoint locations. ${WORLD_MAP_ENCODING_DESCRIPTION}`,
    );
    assert.equal(
        worldMapStatusText(1),
        `Showing 1 recent endpoint location. ${WORLD_MAP_ENCODING_DESCRIPTION}`,
    );
});

test("applyWorldMapFeatureUpdate renders features and announces the latest count", () => {
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
