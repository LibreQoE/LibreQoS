import assert from "node:assert/strict";
import {test} from "node:test";
import {
    buildFieldGrid,
    glowRadiusKm,
    gridLatitude,
    inverseMercatorY,
    kmBetween,
    mercatorY,
} from "./site_map_field.mjs";

const BBOX = {west: -100, east: -90, south: 30, north: 40};

test("mercator y round-trips through its inverse", () => {
    for (const lat of [-80, -45, -1, 0, 12.5, 40, 60, 84]) {
        assert.ok(Math.abs(inverseMercatorY(mercatorY(lat)) - lat) < 1e-9, `lat ${lat}`);
    }
});

test("grid rows are mercator-spaced, not linear-in-latitude", () => {
    const height = 100;
    const lat = 35;
    // The row whose mercator position matches `lat`.
    const yNorth = mercatorY(BBOX.north);
    const ySouth = mercatorY(BBOX.south);
    const mercFraction = (yNorth - mercatorY(lat)) / (yNorth - ySouth);
    const gyMerc = Math.round((mercFraction * height) - 0.5);
    // A naive linear-in-latitude grid would pick a different row.
    const linFraction = (BBOX.north - lat) / (BBOX.north - BBOX.south);
    const gyLin = Math.round((linFraction * height) - 0.5);
    assert.notEqual(gyMerc, gyLin, "the projection fix must change the row");
    // And the mercator row's latitude is within half a cell of the true lat.
    const halfCellDeg = Math.abs(gridLatitude(BBOX, gyMerc, height) - lat);
    assert.ok(halfCellDeg < (BBOX.north - BBOX.south) / height, `off by ${halfCellDeg}`);
});

test("unsmoothed (s=0) d->0 guard returns the site's own value and full alpha", () => {
    const lat = 35;
    const lng = -95;
    // A 1x1 grid centred exactly on the site (symmetric in mercator-y).
    const bbox = {
        west: lng - 0.5,
        east: lng + 0.5,
        north: inverseMercatorY(mercatorY(lat) + 1),
        south: inverseMercatorY(mercatorY(lat) - 1),
    };
    const grid = buildFieldGrid([{lng, lat, value: 123}], bbox, {
        width: 1,
        height: 1,
        k: 8,
        falloffKm: 25,
        smoothKm: 0,
        colorFor: (v) => [v, 0, 0],
    });
    assert.equal(grid.width, 1);
    assert.equal(grid.height, 1);
    assert.equal(grid.data[0], 123);
    assert.equal(grid.data[3], 255, "alpha fades to full at the site");
});

test("the smoothing length-scale flattens the on-site cell off its raw value (no bullseye)", () => {
    const a = {lng: 0, lat: 0, value: 0};
    const b = {lng: 1, lat: 0, value: 100};
    const bbox = {
        west: -0.5,
        east: 0.5,
        north: inverseMercatorY(mercatorY(0) + 1),
        south: inverseMercatorY(mercatorY(0) - 1),
    };
    const opts = {width: 1, height: 1, k: 8, power: 2, falloffKm: 500, colorFor: (v) => [v, 0, 0]};
    // Unsmoothed: the d->0 guard nails A's own value.
    const bare = buildFieldGrid([a, b], bbox, {...opts, smoothKm: 0});
    assert.equal(bare.data[0], 0);
    // Smoothed: A's own cell is pulled toward B (no singularity, no bullseye),
    // but alpha stays full at the site (the on-site alpha floor is preserved).
    const smoothed = buildFieldGrid([a, b], bbox, {...opts, smoothKm: 20});
    assert.ok(smoothed.data[0] > bare.data[0], `smoothed ${smoothed.data[0]} > bare ${bare.data[0]}`);
    assert.equal(smoothed.data[3], 255, "alpha floor preserved at the site");
});

test("smoothing keeps an isolated hotspot the reddest cell", () => {
    const points = [
        {lng: 0, lat: 0, value: 20},
        {lng: 0.5, lat: 0, value: 200},
        {lng: 1, lat: 0, value: 20},
    ];
    const grid = buildFieldGrid(points, {west: 0, east: 1, south: -0.5, north: 0.5}, {
        width: 3,
        height: 1,
        k: 8,
        power: 1.5,
        falloffKm: 500,
        smoothKm: 5,
        colorFor: (v) => [v, 0, 0],
    });
    const left = grid.data[0];
    const hotspot = grid.data[4];
    const right = grid.data[8];
    assert.ok(hotspot > left && hotspot > right, `hotspot is the local max: ${left},${hotspot},${right}`);
    assert.ok(hotspot > 100, `hotspot still reads hot: ${hotspot}`);
});

test("k is clamped to the number of sites and IDW averages them", () => {
    const points = [
        {lng: -95, lat: 35, value: 10},
        {lng: -94, lat: 35, value: 30},
    ];
    const grid = buildFieldGrid(points, {
        west: -96, east: -93, south: 34, north: 36,
    }, {width: 4, height: 4, k: 8, power: 2, falloffKm: 200, colorFor: (v) => [v, v, v]});
    const painted = [];
    for (let i = 0; i < grid.data.length; i += 4) {
        if (grid.data[i + 3] > 0) {
            painted.push(grid.data[i]);
        }
    }
    assert.ok(painted.length > 0, "some cells are painted");
    // Every painted value is an IDW mean of 10 and 30 -> inside [10, 30].
    assert.ok(painted.every((v) => v >= 10 && v <= 30), `values in range: ${painted}`);
});

test("alpha fades to zero beyond the falloff radius", () => {
    const grid = buildFieldGrid([{lng: 0, lat: 0, value: 50}], {
        west: 10, east: 11, south: 10, north: 11,
    }, {width: 8, height: 8, k: 8, falloffKm: 25, colorFor: (v) => [v, 0, 0]});
    for (let i = 3; i < grid.data.length; i += 4) {
        assert.equal(grid.data[i], 0, "nothing is painted far from every site");
    }
});

test("empty points yield a transparent grid", () => {
    const grid = buildFieldGrid([], BBOX, {width: 2, height: 2, colorFor: () => [1, 2, 3]});
    assert.deepEqual(Array.from(grid.data), new Array(2 * 2 * 4).fill(0));
});

test("equirectangular distance is symmetric and zero at a point", () => {
    assert.equal(kmBetween(-95, 35, -95, 35), 0);
    assert.ok(Math.abs(kmBetween(-95, 35, -94, 35) - kmBetween(-94, 35, -95, 35)) < 1e-9);
});

// `.67.5`: per-site glow radius ∝ sqrt(circuit count), clamped.
test("glow radius is sqrt-monotonic in count and clamped", () => {
    const r = (count) => glowRadiusKm(count, 100, 25, 5, 40);
    assert.equal(r(-5), 5, "min clamp keeps a zero/absent count visible");
    assert.equal(r(0), 5);
    assert.ok(Math.abs(r(25) - 12.5) < 1e-9, "25*sqrt(25/100) = 12.5");
    assert.ok(Math.abs(r(100) - 25) < 1e-9, "countRef => the base radius");
    assert.equal(r(1000), 40, "max clamp: a huge site cannot paint the map");
    assert.ok(r(10) <= r(50), "sqrt is monotonic");
    assert.equal(glowRadiusKm(10, 0, 25, 5, 40), 40, "countRef floor 1 => huge but clamped");
});

// `.67.5`: the nearest point's radius drives the alpha fade.
test("a point's radiusKm sets that point's alpha fade", () => {
    // bbox centre ~ (0.05, 0), i.e. ~5.566 km from a point at lng 0.
    const bbox = {west: 0.04, east: 0.06, south: -0.01, north: 0.01};
    const mk = (radiusKm) => buildFieldGrid(
        [{lng: 0, lat: 0, value: 50, radiusKm}],
        bbox,
        {width: 1, height: 1, k: 4, falloffKm: 25, smoothKm: 0, colorFor: (v) => [v, 0, 0]},
    );
    const tooSmall = mk(4);
    const small = mk(10);
    const large = mk(40);
    assert.equal(tooSmall.data[3], 0, "radius < distance => nothing painted");
    assert.ok(small.data[3] > 0, "radius > distance => painted");
    assert.ok(large.data[3] > small.data[3], "a bigger radius paints more alpha");
});

test("alpha uses the NEAREST point's radius, not a farther point's", () => {
    // bbox centre ~ (0.05, 0): nearest is lng 0.04 (~1.11 km), the big-radius
    // point sits farther at lng 0 (~5.57 km).
    const bbox = {west: 0.04, east: 0.06, south: -0.01, north: 0.01};
    const grid = buildFieldGrid([
        {lng: 0.04, lat: 0, value: 10, radiusKm: 3},
        {lng: 0, lat: 0, value: 10, radiusKm: 40},
    ], bbox, {width: 1, height: 1, k: 4, falloffKm: 25, smoothKm: 0, colorFor: (v) => [v, 0, 0]});
    // nearest radius 3 => alpha = 1 - 1.113/3 ~= 0.629 (~160); using the far 40
    // would give ~0.972 (~248).
    assert.ok(grid.data[3] > 0 && grid.data[3] < Math.round(0.9 * 255), `nearest radius wins: ${grid.data[3]}`);
});
