import assert from "node:assert/strict";
import {test} from "node:test";
import {emptyFeatureCollection, rowsToEndpointFeatures} from "./world_map_model.mjs";

test("emptyFeatureCollection returns an empty GeoJSON collection", () => {
    assert.deepEqual(emptyFeatureCollection(), {
        type: "FeatureCollection",
        features: [],
    });
});

test("rowsToEndpointFeatures converts endpoint rows to weighted point features", () => {
    const colors = [];
    const features = rowsToEndpointFeatures([
        [10, 20, "a", 100, 25],
        [15, 30, "b", 400, 75],
    ], (rtt) => {
        colors.push(rtt);
        return `rtt-${rtt}`;
    });

    assert.deepEqual(colors, [25, 75]);
    assert.equal(features.length, 2);
    assert.deepEqual(features[0].geometry.coordinates, [20, 10]);
    assert.deepEqual(features[1].geometry.coordinates, [30, 15]);
    assert.equal(features[0].properties.color, "rtt-25");
    assert.equal(features[1].properties.color, "rtt-75");
    assert.equal(features[0].properties.weight, 0.5);
    assert.equal(features[1].properties.weight, 1);
    assert.equal(features[0].properties.radius, 5);
    assert.equal(features[1].properties.radius, 8);
});

test("rowsToEndpointFeatures filters invalid coordinates", () => {
    const features = rowsToEndpointFeatures([
        ["not-lat", 20, "a", 999999, 1],
        [91, 20, "b", 999999, 1],
        [10, -181, "c", 999999, 1],
        [-90, 180, "d", 100, 1],
    ], () => "#fff");

    assert.equal(features.length, 1);
    assert.deepEqual(features[0].geometry.coordinates, [180, -90]);
    assert.equal(features[0].properties.weight, 1);
    assert.equal(features[0].properties.radius, 8);
});

test("rowsToEndpointFeatures handles empty or malformed payloads", () => {
    assert.deepEqual(rowsToEndpointFeatures(null, () => "#fff"), []);
    assert.deepEqual(rowsToEndpointFeatures({}, () => "#fff"), []);

    const rtts = [];
    const features = rowsToEndpointFeatures([
        [1, 2, "zero", 0, 15],
        [3, 4, "missing"],
        [5, 6, "nan", "not-bytes", "not-rtt"],
    ], (rtt) => {
        rtts.push(rtt);
        return "#fff";
    });

    assert.deepEqual(rtts, [15, 0, 0]);
    assert.equal(features.length, 3);
    assert.equal(features[0].properties.weight, 0);
    assert.equal(features[0].properties.radius, 1);
    assert.equal(features[1].properties.weight, 0);
    assert.equal(features[1].properties.radius, 1);
    assert.equal(features[2].properties.weight, 0);
    assert.equal(features[2].properties.radius, 1);
});
