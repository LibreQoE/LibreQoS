// Flow Globe maths (bead lqos-74t.50, card docs/flow_globe_hexbin_card.md
// §3 WG1/WG1.1, WG2.2, WG2.2-B and WG4). Browser-free (no THREE, no DOM) so the
// node contract tests can exercise the grid, aggregation and geometry packing.
//
// WG1 colour = the MEDIAN (p50) Internet-side RTT per cell (Points mode).
// WG1.1: throughput rides the hex border; the altitude is flat.
//
// WG2.2-B — the R6 custom layer. globe.gl's `hexBinPointsData` cannot
// pre-place cells (any lattice re-bins into H3 gaps) and the manual
// `polygonsData` tiling hangs the renderer (~50 polygons), so we build our own
// uniform offset-row hex grid and pack it into ONE merged non-indexed
// `BufferGeometry` (6 triangles per covered-or-baseline cell, so
// `cell = floor(faceIndex / 6)`) plus ONE colour-only `LineSegments` border.
// The DRIVER heatmap spec: a faint grey NO-DATA honeycomb is the baseline
// texture and only the active/covered cells light up, per-cell, by the NEAREST
// endpoint's Internet RTT. This module only does the maths; flow_map.js does
// the THREE/globe plumbing and colouring.
//
// Wire row: `[lat, lng, label, bytes_down, rtt_internet_nanos, bytes_up]`
// (`flow_map::FlowMapRow`; the RTT is `rtt_nanos_up` = the WAN-anchored leg).

import { toNumber } from "./lq_js_common/helpers/scaling.js";

export const HEX_BIN_RESOLUTION = 3;
export const HEX_MARGIN = 0.05; // seamless honeycomb (WG1.1, Points mode)
export const HEX_ALTITUDE = 0.009; // flat tiles, ~0.006-0.012 (WG1.1)

// `.50.7`: the near-coplanar transparent layers are altitude-separated (radius
// fractions) so they do not collide at grazing angles: country cap (~0.0035-0.004)
// < country border (+0.0005) < R6 fill (0.006) < R6 border (0.008). The fill sits
// just above the country land while the hex shell hugs the globe.
export const R6_FILL_ALTITUDE = 0.006;
export const R6_BORDER_ALTITUDE = 0.008;
export const R6_FILL_MARGIN = 0.06; // shrink each hexagon so the honeycomb reads (halved on DRIVER review)

// --- RTT colour ramp + small colour helpers (pure; used by flow_map.js) -----
// The DRIVER WG4 ramp: blue -> green -> yellow -> red, with a colorblind-safe
// (viridis-like) alternative. `rttRampRgb` returns a 0..255 triple; the callers
// blend toward the backdrop because the basic materials carry no per-vertex alpha.
export const RTT_RAMP_NORMAL = [[33, 145, 255], [76, 217, 130], [253, 216, 53], [255, 110, 118], [200, 30, 60]];
export const RTT_RAMP_COLORBLIND = [[68, 1, 84], [59, 82, 139], [33, 145, 140], [94, 201, 98], [253, 231, 37]];

export function rampStops(colorblind) {
    return colorblind ? RTT_RAMP_COLORBLIND : RTT_RAMP_NORMAL;
}

// t in [0,1] -> [r,g,b] (0..255).
export function rttRampRgb(t, colorblind = false) {
    const stops = rampStops(colorblind);
    const x = Math.max(0, Math.min(1, t));
    const idx = x * (stops.length - 1);
    const i = Math.min(stops.length - 2, Math.floor(idx));
    const f = idx - i;
    const lo = stops[i];
    const hi = stops[i + 1];
    return [
        lo[0] + f * (hi[0] - lo[0]),
        lo[1] + f * (hi[1] - lo[1]),
        lo[2] + f * (hi[2] - lo[2]),
    ];
}

// Theme colours come in two shapes: "#RRGGBB" (oceanColor) and the node_manager
// "r, g, b" triplet (hexNeutralRgb / hexBorderRgb, used as `rgba(<triplet>, a)`
// elsewhere). Accept both; a bare triplet must NOT silently parse to black.
export function parseColorValue(value) {
    if (typeof value !== "string") {
        return [0, 0, 0];
    }
    const text = value.trim();
    if (text.startsWith("#") && text.length === 7) {
        return [
            parseInt(text.slice(1, 3), 16),
            parseInt(text.slice(3, 5), 16),
            parseInt(text.slice(5, 7), 16),
        ];
    }
    const parts = text.split(",").map((part) => parseInt(part.trim(), 10));
    if (parts.length >= 3 && parts.slice(0, 3).every((n) => Number.isFinite(n))) {
        return [parts[0], parts[1], parts[2]];
    }
    return [0, 0, 0];
}

// Blend two 0..255 RGB triples (t=0 -> a, t=1 -> b).
export function mixRgb(a, b, t) {
    const f = Math.max(0, Math.min(1, t));
    return [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f];
}

export function rgbUnit(rgb255) {
    return [rgb255[0] / 255, rgb255[1] / 255, rgb255[2] / 255];
}

export function rgbCss(rgb255) {
    return `rgb(${Math.round(rgb255[0])}, ${Math.round(rgb255[1])}, ${Math.round(rgb255[2])})`;
}

// The R6 fill colour (0..255) for one cell. A covered cell with no RTT sample is
// NO-DATA (not the ramp's blue end); every covered cell returns the raw adaptive
// ramp hue — the measured vs interpolated distinction is carried entirely by the
// per-cell ALPHA (`cellAlpha`), not by desaturating the RGB (double-attenuation).
// An uncovered cell is a faint grey NO-DATA tint.
export function r6CellFillRgb(cell, options = {}) {
    const backdrop = options.backdrop ?? [0, 0, 0];
    const neutral = options.neutral ?? [128, 128, 128];
    if (!cell?.covered || !(cell.rttInternetMs > 0)) {
        return mixRgb(backdrop, neutral, 0.08);
    }
    const p5 = options.domain?.p5 ?? 0;
    const p95 = options.domain?.p95 ?? 1;
    const span = (p95 - p5) || 1;
    const t = Math.max(0, Math.min(1, (cell.rttInternetMs - p5) / span));
    return rttRampRgb(t, options.colorblind === true);
}

// Wire rows -> hexbin points; zero-geo points are dropped (the same filter the
// endpoint builders use). `bytes` is the total (down + up) throughput.
export function hexbinPointsFromRows(rows) {
    if (!Array.isArray(rows)) {
        return [];
    }
    return rows
        .map((row) => ({
            lat: toNumber(row?.[0], 0),
            lng: toNumber(row?.[1], 0),
            label: row?.[2] ? String(row[2]) : "",
            bytes: toNumber(row?.[3], 0) + toNumber(row?.[5], 0),
            rttInternetMs: toNumber(row?.[4], 0) / 1_000_000,
        }))
        .filter((point) => point.lat !== 0 || point.lng !== 0);
}

// The Points-mode per-cell colour metric: the MEDIAN Internet RTT (ms) over the
// cell's members, ignoring non-positive samples.
export function cellMedianInternetRttMs(points) {
    const values = (Array.isArray(points) ? points : [])
        .map((point) => point?.rttInternetMs ?? 0)
        .filter((value) => Number.isFinite(value) && value > 0);
    if (values.length === 0) {
        return 0;
    }
    const sorted = [...values].sort((a, b) => a - b);
    const mid = sorted.length >> 1;
    return sorted.length % 2 === 1 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

// Throughput (bytes) -> the hex border/outline intensity (0.3 floor .. 0.9 heavy;
// a higher floor than WG1.1 so a low-throughput active cell is still readable).
export function hexbinBorderAlpha(sumWeight, maxWeight) {
    const ratio = maxWeight > 0 ? Math.max(0, Math.min(1, sumWeight / maxWeight)) : 0;
    return 0.3 + 0.6 * ratio;
}

const EARTH_RADIUS_KM = 6371.0088;
const KM_PER_DEG = 111.32;
// A conservative LOWER bound on the length of one degree of latitude (the WGS84
// equatorial meridional degree ≈ 110.57 km). A search band must use the MINIMUM
// km-per-latitude-degree so a boundary candidate is never dropped, independent
// of the distance model.
const MIN_KM_PER_LAT_DEG = 110.57;

// Great-circle (haversine) distance in km; antimeridian-safe.
export function greatCircleKm(lat1, lng1, lat2, lng2) {
    const toRad = Math.PI / 180;
    const dLat = (lat2 - lat1) * toRad;
    let dLng = (lng2 - lng1) * toRad;
    if (dLng > Math.PI) dLng -= 2 * Math.PI;
    if (dLng < -Math.PI) dLng += 2 * Math.PI;
    const sinLat = Math.sin(dLat / 2);
    const sinLng = Math.sin(dLng / 2);
    const a = (sinLat * sinLat)
        + Math.cos(lat1 * toRad) * Math.cos(lat2 * toRad) * (sinLng * sinLng);
    return EARTH_RADIUS_KM * 2 * Math.asin(Math.min(1, Math.sqrt(a)));
}

// A lat/lng bucket index for nearest lookups (the longitude bucket wraps at the
// antimeridian). `ref` is carried through so callers can index their own rows.
export function buildLatLngIndex(points, bucketDeg) {
    const d = Math.max(0.5, bucketDeg || 1);
    const lngBuckets = Math.max(1, Math.ceil(360 / d));
    const buckets = new Map();
    const bKey = (lng) => Math.floor(((((lng + 180) % 360) + 360) % 360) / d);
    for (const point of (Array.isArray(points) ? points : [])) {
        if (!point || !Number.isFinite(point.lat) || !Number.isFinite(point.lng)) {
            continue;
        }
        const key = `${Math.floor(point.lat / d)}:${bKey(point.lng)}`;
        const list = buckets.get(key);
        if (list) {
            list.push(point);
        } else {
            buckets.set(key, [point]);
        }
    }
    return { buckets, bucketDeg: d, lngBuckets };
}

// Nearest row via the bucket index (expanding rings, bounded). Points that carry
// a precomputed unit vector (`_vec`) are compared by dot product (the largest dot
// is the nearest) — a large speed-up for the Top-5000 endpoint scan; otherwise
// the exact haversine is used. `distKm` is exact either way.
export function nearestInIndex(index, lat, lng, rings = 2) {
    if (!index || index.buckets.size === 0) {
        return { point: null, distKm: Infinity };
    }
    const { buckets, bucketDeg, lngBuckets } = index;
    const a0 = Math.floor(lat / bucketDeg);
    const b0 = Math.floor(((((lng + 180) % 360) + 360) % 360) / bucketDeg);
    const qv = latLngToUnitVector(lat, lng);
    let best = null;
    let bestDist = Infinity;
    let bestDot = -Infinity;
    for (let r = 0; r <= rings; r += 1) {
        let found = false;
        for (let a = a0 - r; a <= a0 + r; a += 1) {
            for (let bRaw = b0 - r; bRaw <= b0 + r; bRaw += 1) {
                if (r > 0 && Math.max(Math.abs(a - a0), Math.abs(bRaw - b0)) !== r) {
                    continue;
                }
                const b = ((bRaw % lngBuckets) + lngBuckets) % lngBuckets;
                const list = buckets.get(`${a}:${b}`);
                if (!list) {
                    continue;
                }
                found = true;
                for (const point of list) {
                    if (point._vec) {
                        const dot = (qv.x * point._vec.x) + (qv.y * point._vec.y) + (qv.z * point._vec.z);
                        if (dot > bestDot) {
                            bestDot = dot;
                            best = point;
                            bestDist = EARTH_RADIUS_KM * Math.acos(Math.max(-1, Math.min(1, dot)));
                        }
                    } else {
                        const dist = greatCircleKm(lat, lng, point.lat, point.lng);
                        if (dist < bestDist) {
                            bestDist = dist;
                            best = point;
                        }
                    }
                }
            }
        }
        if (found && r >= rings - 1) {
            break;
        }
    }
    return { point: best, distKm: bestDist };
}

// The K-th nearest-neighbour distance (km) per point, via the bucket index.
// Only distances <= `maxSearchKm` can change the clamped result (beyond it the
// halo is the cap), so each query gathers every OTHER point within that radius —
// a bounded superset over the lat/lng bucket grid (antimeridian-wrapped) — then
// takes the K-th smallest exact great-circle distance. `Infinity` means fewer
// than K neighbours exist within the radius (⇒ the caller clamps to the cap).
function kthNearestKm(points, k, maxSearchKm, bucketDeg = 5) {
    const index = buildLatLngIndex(points, bucketDeg);
    const { buckets, lngBuckets } = index;
    const d = Math.max(0.5, bucketDeg);
    // Every point within `maxSearchKm` lies within this latitude band (a great
    // circle is at least as long as its meridional span). We then scan EVERY
    // longitude bucket: a great circle can cross near a pole, so a cos(lat)
    // longitude bound is NOT a safe superset. The band is capped at the whole
    // latitude range, so the scan is bounded (≤ (180/d)·(360/d) buckets).
    const latRange = Math.min(
        Math.ceil(180 / d) + 1,
        Math.max(1, Math.ceil(maxSearchKm / (MIN_KM_PER_LAT_DEG * d))),
    );
    return points.map((p) => {
        const a0 = Math.floor(p.lat / d);
        const dists = [];
        for (let a = a0 - latRange; a <= a0 + latRange; a += 1) {
            for (let b = 0; b < lngBuckets; b += 1) {
                const list = buckets.get(`${a}:${b}`);
                if (!list) {
                    continue;
                }
                for (const q of list) {
                    if (q === p) {
                        continue;
                    }
                    const dist = greatCircleKm(p.lat, p.lng, q.lat, q.lng);
                    if (dist <= maxSearchKm) {
                        dists.push(dist);
                    }
                }
            }
        }
        if (dists.length < k) {
            return Infinity;
        }
        dists.sort((x, y) => x - y);
        return dists[k - 1];
    });
}

// Coverage radius R per endpoint = the K-th nearest-location distance, scaled by
// `c` and clamped to [minRadiusKm, maxRadiusKm]. Neighbour distance (NOT
// throughput) is the variable that bounds spatial support: a cluster earns a
// small halo, an isolated location a large one, so quiet regions glow. The
// cap-derived search radius keeps the neighbour gather bounded.
export function coverageRadiusKm(points, options = {}) {
    const k = Math.max(1, Math.floor(options.k ?? 5));
    const c = Math.max(1e-6, options.c ?? 1.5);
    const minRadiusKm = options.minRadiusKm ?? 300;
    const maxRadiusKm = options.maxRadiusKm ?? 2500;
    const endpoints = (Array.isArray(points) ? points : []).filter(
        (p) => p && Number.isFinite(p.lat) && Number.isFinite(p.lng),
    );
    if (endpoints.length === 0) {
        return [];
    }
    const maxSearchKm = maxRadiusKm / c;
    const dK = kthNearestKm(endpoints, k, maxSearchKm, options.bucketDeg ?? 5);
    return dK.map((distance) => {
        const radius = Number.isFinite(distance) ? c * distance : maxRadiusKm;
        return Math.max(minRadiusKm, Math.min(maxRadiusKm, radius));
    });
}

function latLngToUnitVector(lat, lng) {
    const latRad = (lat * Math.PI) / 180;
    const lngRad = (lng * Math.PI) / 180;
    const cosLat = Math.cos(latRad);
    return {
        x: cosLat * Math.cos(lngRad),
        y: cosLat * Math.sin(lngRad),
        z: Math.sin(latRad),
    };
}

// A pointy-top hexagon ring (6 [lng,lat] vertices, CCW) around a centre. The
// row step for pointy-top hexes is 1.5 * halfHeight = 0.75 * height; a full-height
// row step leaves a rhombus gap between every row pair (the WG2.2 bug). The grid
// is uniform in lat/lng (regular hexagons), so cells compress toward the pole;
// the pole cutoff bounds that. Vertices are unwrapped so a cell straddling the
// antimeridian keeps every vertex within 180 deg of its centre.
// Build a cell record from a precomputed ring (works for any polygon: the
// lat/lng hexagons and the spherical hexasphere's 5/6-gons).
function makeCell(lat, lng, ring, point, distKm, radiusKm) {
    let radiusSum = 0;
    for (const [vLng, vLat] of ring) {
        radiusSum += greatCircleKm(lat, lng, vLat, vLng);
    }
    const cellRadiusKm = ring.length > 0 ? radiusSum / ring.length : 0;
    return {
        lat,
        lng,
        cellRadiusKm,
        covered: !!point && distKm <= radiusKm,
        // A cell that CONTAINS an endpoint is a measurement; a cell that only
        // borrows its nearest neighbour is interpolated (same hue, dimmed).
        measured: !!point && distKm <= cellRadiusKm,
        rttInternetMs: point ? point.rttInternetMs : 0,
        bytes: point ? point.bytes : 0,
        sourceDistKm: point ? distKm : Infinity,
        sourceRadiusKm: point && Number.isFinite(radiusKm) ? radiusKm : 0,
        contributing: 0,
        // The endpoints whose nearest cell centre is this cell (filled below);
        // the click breakout lists these as the cell's locations.
        members: [],
        ring,
    };
}

// ---------------------------------------------------------------------------
// Hexasphere (Goldberg) grid. No lat/lng grid can have regular cells at every
// latitude, so for a uniform-looking honeycomb we subdivide an icosahedron and
// take its DUAL: each mesh vertex becomes a cell (a 6-gon, or a 5-gon for the
// 12 icosahedral vertices), with the cell's ring on the adjacent face centroids.
// The result is a seamless tiling of near-regular hexagons with no pole pinch.

function normalizeVec(v) {
    const len = Math.hypot(v[0], v[1], v[2]) || 1;
    return [v[0] / len, v[1] / len, v[2] / len];
}
function crossVec(a, b) {
    return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}
function dotVec(a, b) {
    return (a[0] * b[0]) + (a[1] * b[1]) + (a[2] * b[2]);
}
function directionToLatLng(v) {
    return {
        lat: Math.asin(Math.max(-1, Math.min(1, v[2]))) * (180 / Math.PI),
        lng: Math.atan2(v[1], v[0]) * (180 / Math.PI),
    };
}

function icosahedronMesh() {
    const t = (1 + Math.sqrt(5)) / 2;
    const verts = [
        [-1, t, 0], [1, t, 0], [-1, -t, 0], [1, -t, 0],
        [0, -1, t], [0, 1, t], [0, -1, -t], [0, 1, -t],
        [t, 0, -1], [t, 0, 1], [-t, 0, -1], [-t, 0, 1],
    ].map(normalizeVec);
    const faces = [
        [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
        [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
        [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
        [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
    ];
    return { verts, faces };
}

// Class-I frequency subdivision: each icosahedron face becomes f² triangles on
// a shared triangular grid, so the dual's cell count is 10·f²+2 (f need not be
// a power of two, which makes the size slider smooth).
function subdivideFrequency(verts, faces, frequency) {
    const f = Math.max(1, Math.floor(frequency));
    if (f <= 1) {
        return faces;
    }
    const edgeCache = new Map();
    const edgeIndices = (a, b) => {
        const lo = Math.min(a, b);
        const hi = Math.max(a, b);
        const key = `${lo}:${hi}`;
        let entry = edgeCache.get(key);
        if (!entry) {
            const va = verts[lo];
            const vb = verts[hi];
            const idx = [];
            for (let i = 1; i < f; i += 1) {
                const t = i / f;
                idx.push(verts.length);
                verts.push(normalizeVec([
                    va[0] + (vb[0] - va[0]) * t,
                    va[1] + (vb[1] - va[1]) * t,
                    va[2] + (vb[2] - va[2]) * t,
                ]));
            }
            entry = { lo, idx };
            edgeCache.set(key, entry);
        }
        return a < b ? entry.idx : entry.idx.slice().reverse();
    };
    const next = [];
    for (const [a, b, c] of faces) {
        const interior = new Map();
        const grid = (i, j) => {
            if (j === 0) {
                return i === 0 ? a : (i === f ? b : edgeIndices(a, b)[i - 1]);
            }
            if (i === 0) {
                return j === f ? c : edgeIndices(a, c)[j - 1];
            }
            if (i + j === f) {
                return edgeIndices(b, c)[j - 1];
            }
            const key = `${i}:${j}`;
            let idx = interior.get(key);
            if (idx === undefined) {
                const va = verts[a];
                const vb = verts[b];
                const vc = verts[c];
                const t = i / f;
                const u = j / f;
                idx = verts.length;
                verts.push(normalizeVec([
                    va[0] + (vb[0] - va[0]) * t + (vc[0] - va[0]) * u,
                    va[1] + (vb[1] - va[1]) * t + (vc[1] - va[1]) * u,
                    va[2] + (vb[2] - va[2]) * t + (vc[2] - va[2]) * u,
                ]));
                interior.set(key, idx);
            }
            return idx;
        };
        for (let i = 0; i < f; i += 1) {
            for (let j = 0; j < f - i; j += 1) {
                const p00 = grid(i, j);
                const p10 = grid(i + 1, j);
                const p01 = grid(i, j + 1);
                next.push([p00, p10, p01]);
                if (j < f - i - 1) {
                    const p11 = grid(i + 1, j + 1);
                    next.push([p10, p11, p01]);
                }
            }
        }
    }
    return next;
}

/**
 * Build the hexasphere LATTICE for `frequency` — DATA-INDEPENDENT: cell centres,
 * rings and `pentagons` only (`makeCell` with no point ⇒ `covered=false`,
 * `measured=false`, `rttInternetMs=0`, `sourceDistKm=Infinity`). `frequency` f
 * gives `10·f² + 2` cells and 12 pentagons.
 *
 * Cached PER LAYER: `mapEndpoints` mutates the cells IN PLACE, so a
 * module-global memo keyed by frequency would let a second instance/remount
 * clobber the first's derived values.
 */
export function buildHexLattice(options = {}) {
    const frequency = options.frequency ?? 32;
    const mesh = icosahedronMesh();
    const faces = subdivideFrequency(mesh.verts, mesh.faces, frequency);
    const incident = Array.from({ length: mesh.verts.length }, () => []);
    faces.forEach((face, fi) => {
        for (const v of face) {
            incident[v].push(fi);
        }
    });
    const centroids = faces.map((face) => normalizeVec([
        mesh.verts[face[0]][0] + mesh.verts[face[1]][0] + mesh.verts[face[2]][0],
        mesh.verts[face[0]][1] + mesh.verts[face[1]][1] + mesh.verts[face[2]][1],
        mesh.verts[face[0]][2] + mesh.verts[face[1]][2] + mesh.verts[face[2]][2],
    ]));

    const cells = [];
    for (let v = 0; v < mesh.verts.length; v += 1) {
        const dir = mesh.verts[v];
        const centre = directionToLatLng(dir);
        const ref = Math.abs(dir[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
        const t1 = normalizeVec(crossVec(dir, ref));
        const t2 = crossVec(dir, t1);
        const ordered = incident[v]
            .map((fi) => {
                const c = centroids[fi];
                const along = dotVec(c, dir);
                const projected = [c[0] - dir[0] * along, c[1] - dir[1] * along, c[2] - dir[2] * along];
                return { fi, angle: Math.atan2(dotVec(projected, t2), dotVec(projected, t1)) };
            })
            .sort((a, b) => a.angle - b.angle);
        const ring = ordered.map(({ fi }) => {
            const ll = directionToLatLng(centroids[fi]);
            return [ll.lng, ll.lat];
        });
        cells.push(makeCell(centre.lat, centre.lng, ring, null, Infinity, 0));
    }
    const pentagons = cells.filter((cell) => cell.ring.length === 5).length;
    return { cells, pentagons, total: cells.length };
}

/**
 * Map `points` onto a `buildHexLattice` result IN PLACE: recompute every cell's
 * `covered/measured/rttInternetMs/bytes/sourceDistKm/sourceRadiusKm` and the
 * `contributing`/`members` lists, and return `{cells, covered, measured, total,
 * pentagons}`. One `buildLatLngIndex` + one nearest probe per cell.
 *
 * Preserves the historical `buildHexasphere` empty shape when no endpoint
 * remains (`{cells:[], ...}`) so the lattice-differential oracle is byte-equal
 * for ALL inputs, including empty. A caller that already holds the mesh cells
 * must pack colours over THOSE cells (not this empty return) on a 0-endpoint
 * pass, so the target length stays equal to the `color` attribute length.
 */
export function mapEndpoints(lattice, points, options = {}) {
    const all = (Array.isArray(points) ? points : []).filter(
        (p) => p && Number.isFinite(p.lat) && Number.isFinite(p.lng),
    );
    const cells = lattice?.cells ?? [];
    if (all.length === 0) {
        return { cells: [], covered: 0, measured: 0, total: 0, pentagons: 0 };
    }
    const minRadiusKm = options.minRadiusKm ?? 300;
    const maxRadiusKm = options.maxRadiusKm ?? 2500;
    const k = options.k ?? 5;
    const c = options.c ?? 1.5;
    const maxEndpoints = options.maxEndpoints ?? 1000;
    const rings = options.rings ?? 2;
    const maxMembers = options.maxMembersPerCell ?? 200;
    const endpoints = [...all]
        .sort((a, b) => (b.bytes || 0) - (a.bytes || 0))
        .slice(0, maxEndpoints)
        .map((p) => ({ ...p, radiusKm: 0 }));
    const radii = coverageRadiusKm(endpoints, { minRadiusKm, maxRadiusKm, k, c });
    endpoints.forEach((endpoint, i) => {
        endpoint.radiusKm = radii[i];
        endpoint._vec = latLngToUnitVector(endpoint.lat, endpoint.lng);
    });
    const endpointIndex = buildLatLngIndex(endpoints, Math.max(1, maxRadiusKm / KM_PER_DEG));

    for (const cell of cells) {
        const { point, distKm } = nearestInIndex(endpointIndex, cell.lat, cell.lng, rings);
        cell.covered = !!point && distKm <= (point?.radiusKm ?? 0);
        cell.measured = !!point && distKm <= cell.cellRadiusKm;
        cell.rttInternetMs = point ? point.rttInternetMs : 0;
        cell.bytes = point ? point.bytes : 0;
        cell.sourceDistKm = point ? distKm : Infinity;
        cell.sourceRadiusKm = point && Number.isFinite(point.radiusKm) ? point.radiusKm : 0;
        cell.contributing = 0;
        cell.members = [];
    }

    // Member lists: nearest cell centre per endpoint (bounded, throughput-sorted).
    const cellIndex = buildLatLngIndex(
        cells.map((cell) => ({ lat: cell.lat, lng: cell.lng, ref: cell })),
        Math.max(0.5, 3 * Math.sqrt((4 * Math.PI) / cells.length) * (180 / Math.PI)),
    );
    for (const endpoint of endpoints) {
        const { point } = nearestInIndex(cellIndex, endpoint.lat, endpoint.lng, 1);
        if (!point?.ref) {
            continue;
        }
        point.ref.contributing += 1;
        point.ref.members.push({
            lat: endpoint.lat,
            lng: endpoint.lng,
            country: endpoint.label || "",
            bytes: endpoint.bytes || 0,
            rttInternetMs: endpoint.rttInternetMs || 0,
        });
    }
    for (const cell of cells) {
        if (cell.members.length > maxMembers) {
            cell.members.sort((a, b) => b.bytes - a.bytes);
            cell.members.length = maxMembers;
        }
    }

    const covered = cells.filter((cell) => cell.covered).length;
    const measured = cells.filter((cell) => cell.measured).length;
    return { cells, covered, measured, total: cells.length, pentagons: lattice.pentagons };
}

/**
 * Build the hexasphere grid: the composition `buildHexLattice` + `mapEndpoints`.
 * `frequency` f gives `10·f² + 2` cells (f=32 → 10242) and 12 pentagons; the
 * returned cells have the same shape as the hexasphere cells (ring of [lng,lat],
 * coverage, members), so `buildHexGeometry` can pack them. Kept for existing
 * callers/tests; the live layer uses the split (cached lattice + value pass).
 */
export function buildHexasphere(points, options = {}) {
    return mapEndpoints(buildHexLattice(options), points, options);
}

// The covered-cell RTT percentile domain (p5, p95) for the adaptive colour
// scale (WG4). Deterministic for a given cell set, so it does not jump between
// refreshes that carry the same endpoints.
export function rttPercentileDomain(cells, lo = 0.05, hi = 0.95) {
    const values = (Array.isArray(cells) ? cells : [])
        .filter((cell) => cell && cell.covered)
        .map((cell) => cell.rttInternetMs)
        .filter((value) => Number.isFinite(value) && value > 0)
        .sort((a, b) => a - b);
    if (values.length === 0) {
        return { p5: 0, p95: 1 };
    }
    const at = (q) => values[Math.min(values.length - 1, Math.max(0, Math.round(q * (values.length - 1))))];
    const p5 = at(lo);
    let p95 = at(hi);
    if (!(p95 > p5)) {
        p95 = p5 + 1;
    }
    return { p5, p95 };
}

/**
 * Pack the grid into merged geometry arrays. Non-indexed: 6 triangles per
 * EMITTED cell, in cell order, so `cell = floor(faceIndex / 6)`; nothing else
 * is merged. The border is colour-only (WebGL ignores `linewidth`) and each
 * shared edge is emitted once (max throughput wins).
 *
 * @param {Array<object>} cells from `buildHexLattice`/`mapEndpoints`
 * @param {{project:(lat:number,lng:number,alt:number)=>({x:number,y:number,z:number}|null),
 *          colorFor?:(cell:object)=>number[], alphaFor?:(cell:object)=>number,
 *          borderColorFor?:(edge:{bytes:number,alpha:number})=>number[],
 *          fillAltitude?:number, borderAltitude?:number, margin?:number,
 *          includeBorder?:boolean}} options
 * @returns {{positions:Float32Array, colors:Float32Array,
 *            borderPositions:Float32Array, borderColors:Float32Array,
 *            emittedCells:Array<object>, faceCell:number[],
 *            triangleCount:number, coveredCount:number}}
 */
export function buildHexGeometry(cells, options = {}) {
    const project = options.project;
    if (typeof project !== "function") {
        throw new Error("buildHexGeometry: options.project is required");
    }
    const fillAltitude = options.fillAltitude ?? R6_FILL_ALTITUDE;
    const borderAltitude = options.borderAltitude ?? R6_BORDER_ALTITUDE;
    const margin = Math.max(0, Math.min(0.9, options.margin ?? R6_FILL_MARGIN));
    const includeBorder = options.includeBorder !== false;
    const colorFor = typeof options.colorFor === "function" ? options.colorFor : () => [1, 1, 1];
    // Per-vertex alpha (RGBA; MeshBasicMaterial supports it when the color
    // attribute has itemSize 4 — three r157 `vertexAlphas`).
    const alphaFor = typeof options.alphaFor === "function" ? options.alphaFor : () => 1;
    const borderColorFor = typeof options.borderColorFor === "function" ? options.borderColorFor : () => [1, 1, 1];
    const shrink = 1 - margin;

    const positions = [];
    const colors = [];
    const emittedCells = [];
    // One triangle per ring edge (fan from the centre); `faceCell[i]` is the
    // cell that owns triangle i (rings are 5 or 6 long, so not face/6).
    const faceCell = [];
    for (const cell of (Array.isArray(cells) ? cells : [])) {
        const centre = project(cell.lat, cell.lng, fillAltitude);
        if (!centre) {
            continue;
        }
        const ring = cell.ring.map(([lng, lat]) => project(lat, lng, fillAltitude));
        const sides = ring.length;
        if (sides < 3 || ring.some((point) => !point)) {
            continue;
        }
        const rgb = colorFor(cell) || [1, 1, 1];
        const alpha = Math.max(0, Math.min(1, alphaFor(cell)));
        for (let i = 0; i < sides; i += 1) {
            const a = ring[i];
            const b = ring[(i + 1) % sides];
            positions.push(
                centre.x, centre.y, centre.z,
                centre.x + (a.x - centre.x) * shrink,
                centre.y + (a.y - centre.y) * shrink,
                centre.z + (a.z - centre.z) * shrink,
                centre.x + (b.x - centre.x) * shrink,
                centre.y + (b.y - centre.y) * shrink,
                centre.z + (b.z - centre.z) * shrink,
            );
            for (let k = 0; k < 3; k += 1) {
                colors.push(rgb[0], rgb[1], rgb[2], alpha);
            }
            faceCell.push(emittedCells.length);
        }
        emittedCells.push(cell);
    }

    const borderPositions = [];
    const borderColors = [];
    if (includeBorder) {
        const edges = new Map();
        for (const cell of (Array.isArray(cells) ? cells : [])) {
            if (!cell.covered) {
                continue;
            }
            const cellAlpha = Math.max(0, Math.min(1, alphaFor(cell)));
            const sides = cell.ring.length;
            for (let i = 0; i < sides; i += 1) {
                const p = cell.ring[i];
                const q = cell.ring[(i + 1) % sides];
                const keyA = `${p[1].toFixed(4)},${p[0].toFixed(4)}`;
                const keyB = `${q[1].toFixed(4)},${q[0].toFixed(4)}`;
                const key = keyA < keyB ? `${keyA}|${keyB}` : `${keyB}|${keyA}`;
                const bytes = Number.isFinite(cell.bytes) ? cell.bytes : 0;
                const existing = edges.get(key);
                if (existing) {
                    if (bytes > existing.bytes) {
                        existing.bytes = bytes;
                    }
                    if (cellAlpha > existing.alpha) {
                        existing.alpha = cellAlpha;
                    }
                } else {
                    edges.set(key, { a: p, b: q, bytes, alpha: cellAlpha });
                }
            }
        }
        for (const edge of edges.values()) {
            const a = project(edge.a[1], edge.a[0], borderAltitude);
            const b = project(edge.b[1], edge.b[0], borderAltitude);
            if (!a || !b) {
                continue;
            }
            const rgb = borderColorFor({ bytes: edge.bytes, alpha: edge.alpha }) || [1, 1, 1];
            borderPositions.push(a.x, a.y, a.z, b.x, b.y, b.z);
            for (let k = 0; k < 2; k += 1) {
                borderColors.push(rgb[0], rgb[1], rgb[2]);
            }
        }
    }

    return {
        positions: Float32Array.from(positions),
        colors: Float32Array.from(colors),
        borderPositions: Float32Array.from(borderPositions),
        borderColors: Float32Array.from(borderColors),
        emittedCells,
        faceCell,
        triangleCount: faceCell.length,
        coveredCount: emittedCells.filter((cell) => cell.covered).length,
    };
}

// ---------------------------------------------------------------------------
// `.50.3` smooth-refresh helpers (pure; used by flow_map.js). The layer caches
// the lattice per instance, packs the target RGBA in `buildHexGeometry`'s pack
// order, and eases the live `color` attribute toward it.

/**
 * Pack per-vertex RGBA exactly as `buildHexGeometry` does: for each cell in
 * order, `ring.length` fan triangles, 3 vertices each, 4 components — so the
 * result length equals the `color` attribute length for the same cells. Used as
 * the tween TARGET, so the settled frame is byte-equal to the current colouring
 * (`cellFillRgb` + `cellAlpha`).
 */
export function packLayerColors(cells, options = {}) {
    const rgbFor = typeof options.rgbFor === "function" ? options.rgbFor : () => [1, 1, 1];
    const alphaFor = typeof options.alphaFor === "function" ? options.alphaFor : () => 1;
    const list = Array.isArray(cells) ? cells : [];
    const vertexCount = list.reduce((n, cell) => n + (cell.ring?.length ?? 0) * 3, 0);
    // Reuse `options.out` when it already has the right length (the value-only
    // refresh path) so the 1 s tick does not allocate a fresh buffer each time.
    const out = options.out instanceof Float32Array && options.out.length === vertexCount * 4
        ? options.out
        : new Float32Array(vertexCount * 4);
    let offset = 0;
    for (const cell of list) {
        const rgb = rgbFor(cell) || [1, 1, 1];
        const alpha = Math.max(0, Math.min(1, alphaFor(cell)));
        const sides = cell.ring.length;
        for (let i = 0; i < sides; i += 1) {
            for (let v = 0; v < 3; v += 1) {
                out[offset] = rgb[0];
                out[offset + 1] = rgb[1];
                out[offset + 2] = rgb[2];
                out[offset + 3] = alpha;
                offset += 4;
            }
        }
    }
    return out;
}

/**
 * One exponential-smoothing step: `current[i] += (target[i] - current[i]) * k`,
 * in place. Returns the largest `|current - target|` AFTER the step (the
 * convergence signal). `0 < k < 1` ⇒ monotone approach, never overshoots.
 */
export function tweenStep(current, target, k = 0.12) {
    const t = Math.max(0, Math.min(1, k));
    const n = Math.min(current.length, target.length);
    let maxDelta = 0;
    for (let i = 0; i < n; i += 1) {
        const delta = target[i] - current[i];
        current[i] += delta * t;
        const abs = Math.abs(target[i] - current[i]);
        if (abs > maxDelta) {
            maxDelta = abs;
        }
    }
    return maxDelta;
}

/**
 * The frame-rate-independent exponential alpha for a time step `dt` (seconds)
 * and time constant `tau` (seconds): `1 - exp(-dt/tau)`. `dt <= 0 => 0`;
 * `tau <= 0 => 1` (snap). Satisfies `1-(1-a(dt1))(1-a(dt2)) = a(dt1+dt2)`, so
 * the motion is independent of the frame cadence.
 */
export function tweenAlpha(dtSeconds, tauSeconds) {
    if (!(tauSeconds > 0)) {
        return 1;
    }
    if (!(dtSeconds > 0)) {
        return 0;
    }
    return 1 - Math.exp(-dtSeconds / tauSeconds);
}

/** `.50.4` time-based tween step: `tweenStep` with the time-derived alpha. */
export function tweenStepTime(current, target, dtSeconds, tauSeconds) {
    return tweenStep(current, target, tweenAlpha(dtSeconds, tauSeconds));
}

/**
 * `.50.5` the JS mirror of the GPU shader's `mix(base, target, alpha)`
 * (`gl_FragColor`-equivalent blend), written into `out` (defaults to `base`).
 * `alpha=0 -> base`, `alpha=1 -> target`. Used by the per-tick base fold in
 * `flow_map.js`; the GLSL `vColor = mix(aBaseColor, color, uTweenAlpha)` runs it
 * on the GPU per frame.
 */
export function mixColors(base, target, alpha, out = base) {
    const t = Math.max(0, Math.min(1, alpha));
    const n = Math.min(base.length, target.length);
    for (let i = 0; i < n; i += 1) {
        out[i] = base[i] + (target[i] - base[i]) * t;
    }
    return out;
}

/**
 * `.50.4` Lever 1: should the (heavier) HUD/legend repaint now? `manual`
 * (a manual Refresh or a user-driven recolour) always repaints; the auto tick
 * is throttled to `throttleMs`. The caller MUST record `nowMs` as its
 * `lastHudMs` on every repaint, or the throttle is a no-op.
 */
export function shouldRefreshHud({ manual = false, lastHudMs = null, nowMs = 0, throttleMs = 0 } = {}) {
    if (manual) {
        return true;
    }
    if (lastHudMs === null || lastHudMs === undefined) {
        return true;
    }
    return (nowMs - lastHudMs) >= throttleMs;
}

/**
 * The geometry cache key: the mesh (positions/`faceCell`) depends only on the
 * lattice — `frequency` + `theme` + the per-instance project identity. A change
 * ⇒ rebuild; an unchanged key ⇒ value-only recolour (geometry stable).
 */
export function latticeCacheKey({ frequency = 32, theme = "light", projectId = "default" } = {}) {
    return `${frequency}|${theme}|${projectId}`;
}

/** `"rebuild"` on a lattice-key change, else `"recolor"` (pure §4.4 decision). */
export function layerUpdatePlan(prevKey, nextKey) {
    return prevKey === nextKey ? "recolor" : "rebuild";
}
