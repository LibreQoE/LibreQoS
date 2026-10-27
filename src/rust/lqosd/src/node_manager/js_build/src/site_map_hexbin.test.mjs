import assert from "node:assert/strict";
import {test} from "node:test";
import {haloRadiusKm, hexRing, planarHaloHexbin} from "./site_map_hexbin.mjs";

test("empty input yields no hexagons", () => {
    const out = planarHaloHexbin([], {});
    assert.deepEqual(out.features, []);
    assert.equal(out.cells, 0);
});

test("haloRadiusKm: sqrt(throughput), clamped to [min,max]", () => {
    assert.equal(haloRadiusKm(0, 100, 20, 3, 60), 3, "min clamp");
    assert.equal(haloRadiusKm(100, 100, 20, 3, 60), 20, "ref -> base");
    assert.equal(haloRadiusKm(10000, 100, 20, 3, 60), 60, "max clamp");
    assert.ok(haloRadiusKm(25, 100, 20, 3, 60) < haloRadiusKm(100, 100, 20, 3, 60), "monotonic");
});

test("halo cells have a radial alpha gradient (full at the site, faint at the edge)", () => {
    const out = planarHaloHexbin([
        { lng: 0, lat: 0, value: 30, throughput: 100 },
        { lng: 2, lat: 0, value: 30, throughput: 100 },
    ], {});
    assert.ok(out.features.length > 0, "the sites have halos");
    const alphas = out.features.map((f) => f.properties.alpha);
    assert.ok(Math.max(...alphas) > 0.9, "a centre cell is fully covered");
    assert.ok(Math.min(...alphas) < 0.5, "the halo edge fades");
    assert.ok(out.features.every((f) => f.properties.rtt === 30), "every halo cell takes the site RTT");
    assert.ok(out.features.every((f) => f.properties.opacity > 0 && f.properties.opacity <= 1));
});

test("a busy site's halo is LARGER than a quiet one's (shared tpRef)", () => {
    // Busy (value 10, tp 10000) + quiet (value 90, tp 100) => tpRef = 10000.
    const out = planarHaloHexbin([
        { lng: -1, lat: 0, value: 10, throughput: 10000 },
        { lng: 1, lat: 0, value: 90, throughput: 100 },
    ], {});
    const busy = out.features.filter((f) => f.properties.rtt === 10).length;
    const quiet = out.features.filter((f) => f.properties.rtt === 90).length;
    assert.ok(busy > 0 && quiet > 0, "both halos exist");
    assert.ok(busy > quiet, `the busy halo covers more cells (${busy} > ${quiet})`);
});

test("cells outside every halo are EMPTY (no feature)", () => {
    // A tiny halo (maxKm small) leaves most of a wide bbox empty.
    const out = planarHaloHexbin([
        { lng: 0, lat: 0, value: 10, throughput: 1 },
        { lng: 2, lat: 0, value: 90, throughput: 1 },
    ], { minKm: 1, maxKm: 2, baseKm: 2 });
    assert.ok(out.features.length > 0, "halos exist");
    // The two halos are ~1-2 km; the 222 km bbox has vast empty space => few cells.
    assert.ok(out.features.length < 200, `mostly empty (got ${out.features.length})`);
});

test("the fill colour comes from the caller's colorFor(value) (the config scale)", () => {
    const out = planarHaloHexbin([
        { lng: 0, lat: 0, value: 30, throughput: 100 },
        { lng: 2, lat: 0, value: 30, throughput: 100 },
    ], { colorFor: (v) => `cfg${v}` });
    assert.ok(out.features.length > 0);
    assert.ok(out.features.every((f) => f.properties.metricColor === "cfg30"));
    assert.ok(!("domain" in out), "no data-range domain is returned (`.67.15`)");
});

test("cell size is UNIFORM (no per-cell scaling)", () => {
    const out = planarHaloHexbin([
        { lng: -92.2, lat: 39.1, value: 28, throughput: 10000 },
        { lng: -92.0, lat: 39.2, value: 30, throughput: 1 },
    ], {});
    const span = (f) => {
        const xs = f.geometry.coordinates[0].map((p) => p[0]);
        const ys = f.geometry.coordinates[0].map((p) => p[1]);
        return [Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)];
    };
    const [w0, h0] = span(out.features[0]);
    for (const f of out.features) {
        const [w, h] = span(f);
        assert.ok(Math.abs(w - w0) < 1e-9 && Math.abs(h - h0) < 1e-9, "all hexagons are the same size");
    }
});

test("hexRing is pointy-top: a vertex at the top, 6 sides + close", () => {
    const ring = hexRing(0, 0, 0.1, 0.1, 1);
    assert.equal(ring.length, 7);
    assert.ok(Math.abs(ring[0][0] - 0) < 1e-9);
    assert.ok(Math.abs(ring[0][1] - 0.1) < 1e-9);
    assert.deepEqual(ring[0], ring[6]);
});

function sharedVertices(a, b) {
    let shared = 0;
    for (const p of a) {
        for (const q of b) {
            if (Math.hypot(p[0] - q[0], p[1] - q[1]) < 1e-9) {
                shared += 1;
                break;
            }
        }
    }
    return shared;
}

test("adjacent hexagons share their edge (no WG2.2 gap)", () => {
    const rLat = 0.05;
    const rLng = 0.05;
    const colStep = Math.sqrt(3) * rLng;
    const rowStep = 1.5 * rLat; // = 0.75 * latStep (the WG2.2 pointy-top row step)
    const center = hexRing(0, 0, rLat, rLng, 1);
    assert.ok(
        sharedVertices(center, hexRing(0, colStep, rLat, rLng, 1)) >= 2,
        "same-row neighbours must share 2 vertices",
    );
    assert.ok(
        sharedVertices(center, hexRing(rowStep, colStep / 2, rLat, rLng, 1)) >= 2,
        "inter-row neighbours must share 2 vertices (rowStep regression guard)",
    );
});
