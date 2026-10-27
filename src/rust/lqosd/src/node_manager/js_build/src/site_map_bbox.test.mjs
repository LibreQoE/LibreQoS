import assert from "node:assert/strict";
import {test} from "node:test";
import {localBboxCenter} from "./site_map_bbox.mjs";

test("centers on the sites' bounding box", () => {
    const center = localBboxCenter([[10, 20], [12, 24]]);
    assert.equal(center.lat, 11);
    assert.equal(center.lon, 22);
    assert.equal(center.zoom, 6);
});

test("a single site centers on itself with a close zoom", () => {
    const center = localBboxCenter([[40.1, -3.7]]);
    assert.equal(center.lat, 40.1);
    assert.equal(center.lon, -3.7);
    assert.ok(center.zoom >= 10);
});

test("empty, null, and malformed input yield a world view", () => {
    assert.deepEqual(localBboxCenter([]), {lat: 0, lon: 0, zoom: 2});
    assert.deepEqual(localBboxCenter(null), {lat: 0, lon: 0, zoom: 2});
    assert.deepEqual(localBboxCenter([[NaN, 1], "x", [2]]), {lat: 0, lon: 0, zoom: 2});
});

test("a wide span zooms out", () => {
    const center = localBboxCenter([[-30, -120], [30, 120]]);
    assert.equal(center.lat, 0);
    assert.equal(center.lon, 0);
    assert.equal(center.zoom, 2);
});

test("ignores non-finite coordinates in a mixed list", () => {
    const center = localBboxCenter([[Infinity, 5], [10, 20], [10, 24]]);
    assert.equal(center.lat, 10);
    assert.equal(center.lon, 22);
});
