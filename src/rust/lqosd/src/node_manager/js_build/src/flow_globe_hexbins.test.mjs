import assert from "node:assert/strict";
import {test} from "node:test";
import {
    HEX_ALTITUDE,
    R6_FILL_MARGIN,
    buildHexasphere,
    buildHexGeometry,
    buildHexLattice,
    buildLatLngIndex,
    cellMedianInternetRttMs,
    coverageRadiusKm,
    greatCircleKm,
    hexbinBorderAlpha,
    hexbinPointsFromRows,
    latticeCacheKey,
    layerUpdatePlan,
    mapEndpoints,
    mixRgb,
    nearestInIndex,
    packLayerColors,
    parseColorValue,
    r6CellFillRgb,
    rttPercentileDomain,
    shouldRefreshHud,
    mixColors,
    tweenAlpha,
    tweenStep,
    tweenStepTime,
} from "./flow_globe_hexbins.mjs";

test("maps wire rows (internet RTT + total bytes) and drops zero-geo", () => {
    const points = hexbinPointsFromRows([
        [10, 20, "a", 5_000, 4_000_000, 1_000],
        [0, 0, "nowhere", 1_000, 1_000_000, 0],
        [30, 40, "b", 0, 9_000_000, 2_000],
    ]);
    assert.equal(points.length, 2);
    assert.deepEqual(points[0], {lat: 10, lng: 20, label: "a", bytes: 6_000, rttInternetMs: 4});
    assert.equal(points[1].bytes, 2_000);
    assert.equal(points[1].rttInternetMs, 9);
    assert.deepEqual(hexbinPointsFromRows(null), []);
});

test("the Points-mode cell colour metric is the median INTERNET RTT", () => {
    assert.equal(cellMedianInternetRttMs([{rttInternetMs: 10}]), 10);
    assert.equal(
        cellMedianInternetRttMs([{rttInternetMs: 1}, {rttInternetMs: 3}, {rttInternetMs: 2}]),
        2,
    );
    assert.equal(
        cellMedianInternetRttMs([{rttInternetMs: 0}, {rttInternetMs: 5}]),
        5,
        "a direction with no sample must not drag the median to 0",
    );
    assert.equal(cellMedianInternetRttMs([]), 0);
});

test("the altitude is flat (WG1.1) and the border intensity tracks throughput", () => {
    assert.ok(HEX_ALTITUDE >= 0.006 && HEX_ALTITUDE <= 0.012, "flat tile altitude");
    const near = (a, b) => Math.abs(a - b) < 1e-9;
    assert.ok(near(hexbinBorderAlpha(0, 0), 0.3), "0.3 floor");
    assert.ok(near(hexbinBorderAlpha(100, 100), 0.9));
    const mid = hexbinBorderAlpha(50, 100);
    assert.ok(mid > 0.3 && mid < 0.9);
});

test("greatCircleKm is zero at a point and antimeridian-safe", () => {
    assert.equal(greatCircleKm(10, 20, 10, 20), 0);
    const oneDeg = greatCircleKm(0, 0, 1, 0);
    assert.ok(oneDeg > 110 && oneDeg < 112, "~111 km per degree");
    const across = greatCircleKm(0, 179, 0, -179);
    assert.ok(across > 200 && across < 230, "the short way across the antimeridian");
});

test("the bucket index finds the nearest endpoint, wrapping the antimeridian", () => {
    const index = buildLatLngIndex([
        { lat: 0, lng: 0, rttInternetMs: 10 },
        { lat: 40, lng: -100, rttInternetMs: 30 },
    ], 5);
    const near = nearestInIndex(index, 1, 1, 2);
    assert.equal(near.point.rttInternetMs, 10);
    assert.ok(near.distKm < 200);
    assert.equal(nearestInIndex({ buckets: new Map(), bucketDeg: 5 }, 0, 0).point, null);
    // A query just across +-180 finds an endpoint just on the other side.
    const seam = buildLatLngIndex([{ lat: 0, lng: 179.5, rttInternetMs: 7 }], 45);
    assert.equal(nearestInIndex(seam, 0, -179.5, 2).point.rttInternetMs, 7);
});

test("coverageRadiusKm uses the K-th nearest location, clamped", () => {
    // A tight pair: d_K is tiny, so both clamp to the floor.
    const cluster = coverageRadiusKm([
        { lat: 0, lng: 0 },
        { lat: 0, lng: 0.1 },
    ], { k: 1, c: 1.5, minRadiusKm: 300, maxRadiusKm: 2500 });
    assert.equal(cluster.length, 2);
    assert.ok(cluster.every((r) => r >= 300), "cluster halos are at least the floor");
    assert.ok(cluster.every((r) => r <= 2500), "and never above the cap");

    // A single/isolated location has no K-th neighbour => the cap.
    const single = coverageRadiusKm(
        [{ lat: 0, lng: 0, bytes: 5 }],
        { k: 5, c: 1.5, minRadiusKm: 300, maxRadiusKm: 2500 },
    );
    assert.deepEqual(single, [2500], "an isolated location takes the cap");

    // K ordering: a larger K reaches farther (or equal), so R is monotone in K.
    const spread = [
        { lat: 0, lng: 0 },
        { lat: 0, lng: 5 },
        { lat: 0, lng: 10 },
        { lat: 0, lng: 15 },
        { lat: 0, lng: 20 },
    ];
    const r1 = coverageRadiusKm(spread, { k: 1, c: 1.0, minRadiusKm: 0, maxRadiusKm: 2500 });
    const r3 = coverageRadiusKm(spread, { k: 3, c: 1.0, minRadiusKm: 0, maxRadiusKm: 2500 });
    for (let i = 0; i < spread.length; i += 1) {
        assert.ok(r3[i] >= r1[i], `K=3 radius >= K=1 radius at ${i}`);
    }
    assert.ok(r3.every((r) => r <= 2500), "the cap holds");

    // Empty / all-invalid => no radii.
    assert.deepEqual(coverageRadiusKm([], {}), []);
    assert.deepEqual(coverageRadiusKm([{ lat: NaN, lng: 0 }], {}), []);
});

test("coverageRadiusKm's gather finds a latitude-band-edge neighbour", () => {
    // maxSearch = 2500/1.5 = 1666.67 km. A pure-latitude neighbour at 14.9 deg is
    // ~1656.8 km (inside) — the band uses a conservative MIN km/lat-deg so it is
    // gathered, not clamped to the cap.
    const got = coverageRadiusKm(
        [{ lat: 0, lng: 0 }, { lat: 14.9, lng: 0 }],
        { k: 1, c: 1.5, minRadiusKm: 0, maxRadiusKm: 2500, bucketDeg: 5 },
    );
    for (const r of got) {
        assert.ok(r > 0 && r < 2500, `band-edge neighbour gathered (got ${r})`);
    }
    // Beyond cap/c there is no neighbour in range => the cap.
    assert.deepEqual(
        coverageRadiusKm(
            [{ lat: 0, lng: 0 }, { lat: 20, lng: 0 }],
            { k: 1, c: 1.5, minRadiusKm: 0, maxRadiusKm: 2500, bucketDeg: 5 },
        ),
        [2500, 2500],
    );
});

test("coverageRadiusKm caps every halo when there are fewer than K neighbours", () => {
    const two = [{ lat: 0, lng: 0 }, { lat: 0, lng: 1 }];
    assert.deepEqual(
        coverageRadiusKm(two, { k: 5, c: 1.5, minRadiusKm: 300, maxRadiusKm: 2500 }),
        [2500, 2500],
        "N <= K => every point has no K-th neighbour => the cap",
    );
});

// R3: the indexed K-th-neighbour gather must equal a brute-force K-th nearest
// over a corpus that exercises clusters, the antimeridian, and the poles, for
// several K and cap values (the cap sets the search radius = cap/c).
test("coverageRadiusKm's indexed gather equals brute force (R3)", () => {
    const bruteRadii = (points, k, c, min, max) => points.map((p, i) => {
        const dists = [];
        points.forEach((q, j) => {
            if (i === j) return;
            dists.push(greatCircleKm(p.lat, p.lng, q.lat, q.lng));
        });
        dists.sort((a, b) => a - b);
        const dK = dists.length >= k ? dists[k - 1] : Infinity;
        const r = Number.isFinite(dK) ? c * dK : max;
        return Math.max(min, Math.min(max, r));
    });

    const corpus = [
        { lat: 0, lng: 0 },
        { lat: 0, lng: 0.05 }, // a tight cluster
        { lat: 0, lng: 0.1 },
        { lat: 10, lng: 20 },
        { lat: -33.9, lng: 151.2 },
        { lat: 51.5, lng: -0.12 },
        { lat: 0, lng: 179.9 }, // antimeridian pair
        { lat: 0, lng: -179.9 },
        { lat: 84, lng: 10 }, // near-pole pair
        { lat: 84.5, lng: -170 },
        { lat: -45, lng: 0 },
        { lat: 14.9, lng: 3 }, // near the latitude band edge for cap/c
        { lat: -13.5, lng: 5 },
    ];
    for (const k of [1, 3, 5]) {
        for (const cap of [800, 2500, 1e9]) {
            const got = coverageRadiusKm(corpus, { k, c: 1.5, minRadiusKm: 300, maxRadiusKm: cap });
            const want = bruteRadii(corpus, k, 1.5, 300, cap);
            assert.equal(got.length, want.length);
            for (let i = 0; i < got.length; i += 1) {
                assert.ok(
                    Math.abs(got[i] - want[i]) < 1e-6,
                    `k=${k} cap=${cap} i=${i}: gather ${got[i]} vs brute ${want[i]}`,
                );
            }
        }
    }
});

// ---------------------------------------------------------------------------
// WG2.2-B — the R6 grid, geometry packing and adaptive colour domain.

test("buildHexGeometry packs a fan/cell and maps faces via faceCell", () => {
    const cells = [
        {
            lat: 0, lng: 0, covered: true, measured: true, bytes: 100, rttInternetMs: 10,
            ring: [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]],
        },
        {
            lat: 0, lng: 2, covered: false, measured: false, bytes: 0, rttInternetMs: 0,
            ring: [[1, -1], [3, -0.5], [3, 0.5], [2, 1], [1, 0.5], [1, -0.5]],
        },
    ];
    const project = (lat, lng, alt) => ({ x: lng, y: lat, z: alt });
    const geometry = buildHexGeometry(cells, {
        project,
        colorFor: () => [1, 0, 0],
        borderColorFor: () => [0, 1, 0],
    });
    assert.equal(geometry.emittedCells.length, 2);
    assert.equal(geometry.triangleCount, 12);
    // 6 triangles * 3 vertices * 3 position components per cell.
    assert.equal(geometry.positions.length, 2 * 54);
    // Colour is RGBA now (4 components per vertex): 6*3*4 per cell.
    assert.equal(geometry.colors.length, 2 * 72);
    // One triangle per ring edge; faceCell is the triangle -> cell table.
    assert.equal(geometry.faceCell.length, geometry.triangleCount);
    assert.equal(geometry.faceCell[0], 0);
    assert.equal(geometry.faceCell[5], 0);
    assert.equal(geometry.faceCell[6], 1);
    assert.equal(geometry.faceCell[11], 1);
    // The fill stays just above the country land (~0.0035-0.004).
    for (let i = 2; i < geometry.positions.length; i += 3) {
        assert.ok(geometry.positions[i] > 0.004, "fill above the country land");
    }
});

test("buildHexasphere tiles the sphere with 10f^2+2 cells (12 pentagons, edge-matched)", () => {
    const f = 8;
    const { cells, total, pentagons } = buildHexasphere(
        [{ lat: 0, lng: 0, bytes: 1000, rttInternetMs: 10 }],
        { frequency: f },
    );
    assert.equal(total, 10 * f * f + 2, "Goldberg cell count");
    assert.equal(pentagons, 12, "exactly 12 pentagons");
    assert.ok(cells.every((cell) => cell.ring.length === 5 || cell.ring.length === 6));
    // Edge-matched (seamless) proof: the dual of a closed triangulation has one
    // distinct ring vertex per triangle, and each is shared by exactly 3 cells.
    const counts = new Map();
    for (const cell of cells) {
        for (const [lng, lat] of cell.ring) {
            const key = `${lng.toFixed(6)},${lat.toFixed(6)}`;
            counts.set(key, (counts.get(key) || 0) + 1);
        }
    }
    assert.equal(counts.size, 20 * f * f, "one dual vertex per triangle");
    assert.ok([...counts.values()].every((n) => n === 3), "each dual vertex joins exactly 3 cells");
});

test("buildHexGeometry emits each shared border edge once", () => {
    // Two covered cells sharing the [1,-0.5]-[1,0.5] edge: 12 raw edges -> 11.
    const ringA = [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]];
    const ringB = [[1, -1], [3, -0.5], [3, 0.5], [2, 1], [1, 0.5], [1, -0.5]];
    const cells = [
        { lat: 0, lng: 0, covered: true, bytes: 100, ring: ringA },
        { lat: 0, lng: 2, covered: true, bytes: 200, ring: ringB },
    ];
    const project = (lat, lng, alt) => ({ x: lng, y: lat, z: alt });
    const geometry = buildHexGeometry(cells, { project, borderColorFor: () => [0, 1, 0] });
    assert.equal(geometry.borderPositions.length, 11 * 2 * 3, "11 unique edges");
});

test("buildHexGeometry emits per-vertex RGBA alpha and the edge carries cell alpha", () => {
    const ringA = [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]];
    const ringB = [[1, -1], [3, -0.5], [3, 0.5], [2, 1], [1, 0.5], [1, -0.5]];
    const cells = [
        { lat: 0, lng: 0, covered: true, measured: true, bytes: 100, ring: ringA },
        { lat: 0, lng: 2, covered: true, measured: false, bytes: 50, ring: ringB },
    ];
    const edgeAlphas = [];
    const project = (lat, lng, alt) => ({ x: lng, y: lat, z: alt });
    const geometry = buildHexGeometry(cells, {
        project,
        alphaFor: (cell) => (cell.lng === 0 ? 0.9 : 0.2),
        borderColorFor: (edge) => {
            edgeAlphas.push(edge.alpha);
            return [1, 1, 1];
        },
    });
    assert.equal(geometry.colors.length, 2 * 18 * 4, "RGBA per vertex");
    assert.ok(Math.abs(geometry.colors[3] - 0.9) < 1e-6, "cell 0 alpha 0.9");
    assert.ok(edgeAlphas.includes(0.9), "the shared edge takes the max cell alpha");
    assert.ok(edgeAlphas.every((a) => a <= 0.9));
});

test("buildHexGeometry skips cells whose projection is not ready yet", () => {
    const cells = [
        { lat: 0, lng: 0, covered: true, bytes: 1, ring: [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]] },
        { lat: 90, lng: 0, covered: true, bytes: 1, ring: [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]] },
    ];
    const project = (lat) => (lat === 90 ? null : { x: 0, y: 0, z: 1 });
    const geometry = buildHexGeometry(cells, { project });
    assert.equal(geometry.emittedCells.length, 1, "the null-projection cell is dropped");
    assert.equal(geometry.triangleCount, 6);
});

test("rttPercentileDomain is the covered RTT p5-p95 band", () => {
    const cells = Array.from({ length: 100 }, (_, i) => ({ covered: true, rttInternetMs: i + 1 }));
    cells.push({ covered: false, rttInternetMs: 9999 });
    const { p5, p95 } = rttPercentileDomain(cells);
    assert.ok(p5 >= 1 && p5 <= 10, "near the 5th percentile");
    assert.ok(p95 >= 90 && p95 <= 100, "near the 95th percentile");
    assert.deepEqual(rttPercentileDomain([]), { p5: 0, p95: 1 });
});

// ---------------------------------------------------------------------------
// Theme colour values. The node_manager palette mixes "#RRGGBB" (oceanColor)
// with the "r, g, b" triplet form (hexNeutralRgb/hexBorderRgb); parsing a
// triplet as a hex silently produced black, which made the dark NO-DATA
// baseline near-invisible and the throughput border ramp to black.

test("parseColorValue accepts #RRGGBB and the 'r, g, b' theme triplet", () => {
    assert.deepEqual(parseColorValue("#0b1730"), [11, 23, 48]);
    assert.deepEqual(parseColorValue("148, 163, 184"), [148, 163, 184]);
    assert.deepEqual(parseColorValue("100, 116, 139"), [100, 116, 139]);
    assert.deepEqual(parseColorValue("15, 23, 42"), [15, 23, 42]);
    assert.deepEqual(parseColorValue("garbage"), [0, 0, 0]);
});

test("the NO-DATA baseline and throughput border resolve in BOTH themes", () => {
    const themes = {
        dark: { ocean: "#0b1730", neutral: "148, 163, 184", border: "226, 232, 240" },
        light: { ocean: "#dce9f8", neutral: "100, 116, 139", border: "15, 23, 42" },
    };
    for (const [name, theme] of Object.entries(themes)) {
        const backdrop = parseColorValue(theme.ocean);
        const neutral = parseColorValue(theme.neutral);
        const border = parseColorValue(theme.border);
        const triplet = (value) => value.split(",").map((part) => parseInt(part.trim(), 10));
        assert.deepEqual(neutral, triplet(theme.neutral), `${name} neutral parses`);
        assert.deepEqual(border, triplet(theme.border), `${name} border parses`);
        // The faint baseline must be a real tint, not backdrop*0.84 (the bug).
        const noData = mixRgb(backdrop, neutral, 0.16);
        assert.ok(
            noData.some((channel, i) => Math.abs(channel - backdrop[i]) > 5),
            `${name} NO-DATA baseline is visible`,
        );
        // A full-throughput border must reach the border colour, not black.
        const heavy = mixRgb(backdrop, border, 0.9);
        assert.ok(
            heavy.some((channel, i) => Math.abs(channel - border[i]) < 40),
            `${name} throughput border reaches the border colour`,
        );
    }
});

test("r6CellFillRgb: NO-DATA for uncovered + RTT-0, ramp for measured, in BOTH themes", () => {
    const themes = {
        dark: { backdrop: parseColorValue("#0b1730"), neutral: parseColorValue("148, 163, 184") },
        light: { backdrop: parseColorValue("#dce9f8"), neutral: parseColorValue("100, 116, 139") },
    };
    const domain = { p5: 10, p95: 80 };
    for (const [name, theme] of Object.entries(themes)) {
        const noData = r6CellFillRgb({ covered: false, rttInternetMs: 0 }, { ...theme, domain });
        const zero = r6CellFillRgb({ covered: true, measured: true, rttInternetMs: 0 }, { ...theme, domain });
        assert.deepEqual(zero, noData, `${name}: a covered RTT-0 cell is NO-DATA, not the fast end`);
        assert.ok(
            noData.some((channel, i) => Math.abs(channel - theme.backdrop[i]) > 5),
            `${name} NO-DATA is a visible tint`,
        );
        const slow = r6CellFillRgb({ covered: true, measured: true, rttInternetMs: 80 }, { ...theme, domain });
        const interpolated = r6CellFillRgb({ covered: true, measured: false, rttInternetMs: 80 }, { ...theme, domain });
        assert.notDeepEqual(slow, noData, `${name} a measured cell uses the ramp`);
        assert.deepEqual(
            interpolated,
            slow,
            `${name} the hue is identical; per-cell alpha carries the measured/interpolated fade`,
        );
    }
});

// ---------------------------------------------------------------------------
// `.50.3` — the split lattice + value pass, target-colour parity, the tween and
// the pure geometry-cache decision.

test("packLayerColors over emittedCells matches buildHexGeometry when projection skips cells", () => {
    const ring = [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]];
    const cells = [
        { lat: 0, lng: 0, covered: true, bytes: 1, ring },
        { lat: 90, lng: 0, covered: true, bytes: 1, ring }, // projection not ready
        { lat: 0, lng: 2, covered: true, bytes: 1, ring },
    ];
    const project = (lat) => (lat === 90 ? null : { x: 0, y: 0, z: 1 });
    const geometry = buildHexGeometry(cells, { project });
    assert.equal(geometry.emittedCells.length, 2, "the null-projection cell is skipped");
    // The tween target must use the EMITTED cells (not the full input set), or
    // its length diverges from the geometry's colour buffer.
    const packed = packLayerColors(geometry.emittedCells, {});
    assert.equal(
        packed.length,
        geometry.colors.length,
        "target length equals the geometry colour-attribute length",
    );
});

test("buildHexLattice + mapEndpoints is the buildHexasphere differential (incl. empty)", () => {
    const points = [
        { lat: 0, lng: 0, bytes: 1000, rttInternetMs: 10 },
        { lat: 40, lng: -100, bytes: 8000, rttInternetMs: 30 },
        { lat: -20, lng: 179.5, bytes: 400, rttInternetMs: 5 },
    ];
    const options = { frequency: 6, maxEndpoints: 100 };
    const composed = mapEndpoints(buildHexLattice({ frequency: 6 }), points, options);
    assert.deepEqual(
        composed,
        buildHexasphere(points, options),
        "lattice + mapEndpoints is cell-for-cell equal to buildHexasphere",
    );
    // The 0-endpoint short-circuit is preserved (so the oracle holds on empty).
    assert.deepEqual(
        mapEndpoints(buildHexLattice({ frequency: 6 }), [], options),
        buildHexasphere([], options),
    );
    assert.deepEqual(buildHexasphere([], options), {
        cells: [],
        covered: 0,
        measured: 0,
        total: 0,
        pentagons: 0,
    });
});

test("packLayerColors matches buildHexGeometry's RGBA pack order (target parity)", () => {
    const ring = [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]];
    const cells = [
        { lat: 0, lng: 0, covered: true, bytes: 100, ring },
        { lat: 0, lng: 2, covered: true, bytes: 50, ring },
    ];
    const project = (lat, lng, alt) => ({ x: lng, y: lat, z: alt });
    const rgbFor = (cell) => (cell.lng === 0 ? [1, 0, 0] : [0, 1, 0]);
    const alphaFor = (cell) => (cell.lng === 0 ? 0.9 : 0.2);
    const geometry = buildHexGeometry(cells, { project, colorFor: rgbFor, alphaFor });
    const packed = packLayerColors(geometry.emittedCells, { rgbFor, alphaFor });
    assert.deepEqual(
        Array.from(packed),
        Array.from(geometry.colors),
        "the tween target equals the current settled colouring",
    );
});

test("tweenStep converges monotonically with no overshoot", () => {
    const current = Float32Array.from([0, 10, -5, 1]);
    const target = Float32Array.from([1, 0, 5, 1]);
    const previous = Array.from(current, (value, i) => Math.abs(target[i] - value));
    let converged = false;
    for (let step = 0; step < 500; step += 1) {
        const maxDelta = tweenStep(current, target, 0.12);
        for (let i = 0; i < current.length; i += 1) {
            assert.ok(
                Math.abs(current[i] - target[i]) <= previous[i] + 1e-6,
                "an exponential-smoothing step must not overshoot",
            );
            previous[i] = Math.abs(target[i] - current[i]);
        }
        if (maxDelta < 1e-4) {
            converged = true;
            break;
        }
    }
    assert.ok(converged, "exponential smoothing must converge below epsilon");
});

test("latticeCacheKey / layerUpdatePlan: rebuild only on a lattice change", () => {
    const base = latticeCacheKey({ frequency: 32, theme: "dark", projectId: "globe" });
    assert.equal(
        layerUpdatePlan(base, latticeCacheKey({ frequency: 32, theme: "dark", projectId: "globe" })),
        "recolor",
        "an unchanged lattice key is a value-only recolour",
    );
    assert.equal(
        layerUpdatePlan(base, latticeCacheKey({ frequency: 64, theme: "dark", projectId: "globe" })),
        "rebuild",
        "a Hex-size change rebuilds",
    );
    assert.equal(
        layerUpdatePlan(base, latticeCacheKey({ frequency: 32, theme: "light", projectId: "globe" })),
        "rebuild",
        "a theme change rebuilds",
    );
    assert.equal(
        layerUpdatePlan(base, latticeCacheKey({ frequency: 32, theme: "dark", projectId: "other" })),
        "rebuild",
        "a project-identity change rebuilds",
    );
});

// ---------------------------------------------------------------------------
// `.50.4` — the time-based tween + the silent-auto-tick HUD throttle.

test("tweenAlpha is frame-rate independent, monotone and bounded", () => {
    assert.equal(tweenAlpha(0, 0.9), 0, "dt=0 => no motion");
    assert.equal(tweenAlpha(1, 0), 1, "tau<=0 => snap");
    const a = tweenAlpha(0.016, 0.9);
    assert.ok(a > 0 && a < 1);
    assert.ok(tweenAlpha(0.5, 0.9) < tweenAlpha(1.5, 0.9), "monotone in dt");
    assert.ok(tweenAlpha(2.7, 0.9) > 0.95, "95% by 3*tau (2.7 s)");
    assert.ok(tweenAlpha(100, 0.9) > 0.999);
    // Frame-rate independence: two half steps == one full step.
    const half = tweenAlpha(0.5, 0.9);
    assert.ok(Math.abs((1 - (1 - half) * (1 - half)) - tweenAlpha(1.0, 0.9)) < 1e-9);
});

test("tweenStepTime is monotone, converges and never overshoots", () => {
    const current = Float32Array.from([0, 10, -5, 1]);
    const target = Float32Array.from([1, 0, 5, 1]);
    const previous = Array.from(current, (value, i) => Math.abs(target[i] - value));
    let converged = false;
    for (let step = 0; step < 500; step += 1) {
        const maxDelta = tweenStepTime(current, target, 0.1, 0.9);
        for (let i = 0; i < current.length; i += 1) {
            assert.ok(
                Math.abs(current[i] - target[i]) <= previous[i] + 1e-6,
                "a time-based step must not overshoot",
            );
            previous[i] = Math.abs(target[i] - current[i]);
        }
        if (maxDelta < 1e-4) {
            converged = true;
            break;
        }
    }
    assert.ok(converged, "the time-based tween must converge");
});

test("shouldRefreshHud: auto is throttled; manual and the first paint repaint", () => {
    assert.equal(
        shouldRefreshHud({ manual: true, lastHudMs: 1000, nowMs: 1001, throttleMs: 5000 }),
        true,
        "manual always repaints",
    );
    assert.equal(
        shouldRefreshHud({ manual: false, lastHudMs: null, nowMs: 1000, throttleMs: 5000 }),
        true,
        "the first paint repaints",
    );
    assert.equal(
        shouldRefreshHud({ manual: false, lastHudMs: 1000, nowMs: 2000, throttleMs: 5000 }),
        false,
        "within the throttle the auto path does not repaint",
    );
    assert.equal(
        shouldRefreshHud({ manual: false, lastHudMs: 1000, nowMs: 6000, throttleMs: 5000 }),
        true,
        "past the throttle the auto path repaints",
    );
});

test("packLayerColors reuses a same-length out buffer (no per-tick allocation)", () => {
    const ring = [[-1, -1], [1, -0.5], [1, 0.5], [0, 1], [-1, 0.5], [-1, -0.5]];
    const cells = [{ lat: 0, lng: 0, covered: true, bytes: 1, ring }];
    const buffer = new Float32Array(cells[0].ring.length * 3 * 4);
    const packed = packLayerColors(cells, { out: buffer });
    assert.equal(packed, buffer, "the same-length out buffer is reused in place");
    const tooSmall = new Float32Array(4);
    const allocated = packLayerColors(cells, { out: tooSmall });
    assert.notEqual(allocated, tooSmall, "a wrong-length out buffer is not reused");
    assert.equal(allocated.length, buffer.length);
});

// ---------------------------------------------------------------------------
// `.50.5` — the GPU-tween shader mirror (`mix`) and its per-tick base fold.

test("mixColors is the shader mix contract: alpha 0 => base, 1 => target", () => {
    const base = Float32Array.from([0.1, 0.5, 0.9, 0.2]);
    const target = Float32Array.from([0.9, 0.1, 0.3, 0.8]);
    assert.deepEqual(
        Array.from(mixColors(Float32Array.from(base), target, 0)),
        Array.from(base),
        "mix(base, target, 0) = base",
    );
    assert.deepEqual(
        Array.from(mixColors(Float32Array.from(base), target, 1)),
        Array.from(target),
        "mix(base, target, 1) = target (the settled frame)",
    );
    const quarter = mixColors(Float32Array.from(base), target, 0.25);
    for (let i = 0; i < base.length; i += 1) {
        assert.ok(Math.abs(quarter[i] - (base[i] + (target[i] - base[i]) * 0.25)) < 1e-6);
    }
    // In place by default (`out` defaults to `base`) — the per-tick fold.
    const inPlace = Float32Array.from(base);
    assert.equal(mixColors(inPlace, target, 0.5), inPlace);
});
