import assert from "node:assert/strict";
import {test} from "node:test";
import {
    colorAlpha,
    geometryPolygons,
    normalizePolygonRings,
    signedRingArea,
    subdivideTriangleToMaxEdge,
    triangleMaxEdgeDeg,
    unwrapRing,
} from "./flow_globe_countries.mjs";

// `lqos-74t.50.7` Stage 2: the merged country caps need normalized ring winding
// (outer CCW / holes CW), antimeridian unwrapping, and the theme rgba alpha.

test("signedRingArea: CCW is positive, CW is negative", () => {
    const ccw = [[0, 0], [1, 0], [1, 1], [0, 1]];
    const cw = [[0, 0], [0, 1], [1, 1], [1, 0]];
    assert.ok(signedRingArea(ccw) > 0);
    assert.ok(signedRingArea(cw) < 0);
    assert.equal(signedRingArea([[0, 0], [1, 0]]), 0, "a ring needs >= 3 points");
});

test("normalizePolygonRings: outer CCW, holes CW, input not mutated", () => {
    const outerCw = [[0, 0], [0, 1], [1, 1], [1, 0]];
    const holeCcw = [[0.2, 0.2], [0.8, 0.2], [0.8, 0.8], [0.2, 0.8]];
    const polygon = [outerCw, holeCcw];
    const normalized = normalizePolygonRings(polygon);
    assert.ok(signedRingArea(normalized[0]) > 0, "outer is CCW");
    assert.ok(signedRingArea(normalized[1]) < 0, "hole is CW");
    assert.ok(signedRingArea(polygon[0]) < 0, "the input ring is untouched");
    assert.ok(signedRingArea(polygon[1]) > 0, "the input hole is untouched");
});

test("unwrapRing: contiguous across the antimeridian; rejects a >180 span", () => {
    const seam = unwrapRing([[179, 0], [-179, 0], [-179, 1], [179, 1]]);
    assert.ok(seam, "a seam ring unwraps");
    const xs = seam.map(([x]) => x);
    assert.ok(Math.max(...xs) - Math.min(...xs) <= 180, "unwrapped span <= 180");
    assert.equal(
        unwrapRing([[-170, 0], [-10, 0], [10, 0], [170, 0]]),
        null,
        "a genuine >180 span is rejected",
    );
});

test("colorAlpha parses the theme rgba alpha; geometryPolygons splits types", () => {
    assert.equal(colorAlpha("rgba(164, 196, 255, 0.74)"), 0.74);
    assert.equal(colorAlpha("rgba(111, 157, 230, 0.18)"), 0.18);
    assert.equal(colorAlpha("#aabbcc"), 1, "no alpha => 1");
    assert.equal(geometryPolygons({type: "Polygon", coordinates: [[[0, 0]]]}).length, 1);
    assert.equal(
        geometryPolygons({type: "MultiPolygon", coordinates: [[[[0, 0]]], [[[1, 1]]]]}).length,
        2,
    );
    assert.deepEqual(geometryPolygons({type: "Point", coordinates: [0, 0]}), []);
});

// `lqos-74t.50.7`: a coarse earcut cap triangle sags through the globe; bounding
// the triangle's edge bounds the chord sag `R(1 - cos(theta/2))` below the cap's
// altitude (R = 1).
test("adaptive cap subdivision bounds the chord sag below the country altitude", () => {
    const COUNTRY_ALTITUDE = 0.004;
    const coarse = [[-20, -20], [20, -20], [0, 20]];
    assert.ok(triangleMaxEdgeDeg(coarse) > 20, "the raw earcut triangle is coarse");
    const subs = subdivideTriangleToMaxEdge(coarse, 3);
    assert.ok(subs.length > 1, "a coarse triangle is subdivided");
    const maxEdge = Math.max(...subs.map(triangleMaxEdgeDeg));
    assert.ok(maxEdge <= 3 + 1e-9, `all edges <= 3 deg (got ${maxEdge})`);
    const sag = 1 - Math.cos((maxEdge / 2) * (Math.PI / 180));
    assert.ok(sag < COUNTRY_ALTITUDE, `sag ${sag} < ${COUNTRY_ALTITUDE}`);
});

test("subdivideTriangleToMaxEdge leaves an already-fine triangle alone", () => {
    const fine = [[0, 0], [1, 0], [0, 1]];
    assert.equal(subdivideTriangleToMaxEdge(fine, 3).length, 1);
});
