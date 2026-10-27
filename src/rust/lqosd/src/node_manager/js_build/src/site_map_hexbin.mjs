// `.67.12`-`.67.14` — a FLAT 2D planar hexbin for the Site Map.
//
// `.67.13` filled EVERY lattice cell with the nearest site's RTT across a 40 km
// reach (a big uniform block). `.67.14` makes the fill a PER-SITE GLOW KERNEL:
// every site `i` has a halo radius `R_i = clamp(baseKm*sqrt(tp_i/tpRef), minKm,
// maxKm)` (busy sites glow far, quiet sites little). A cell is drawn only if SOME
// site's halo reaches it; it takes the RTT of the site with the STRONGEST coverage
// (`max` over covering sites of `alpha = 1 - d_i/R_i`, the "max wins" pin — a
// nearest-only rule would clip a big halo to its Voronoi cell). Cells outside
// every halo are EMPTY (no feature) so the basemap shows through.
//
// The in-repo helpers reused: `buildLatLngIndex`/`greatCircleKm` (the nearest
// scan). NO new dependency. The PLANAR pointy-top geometry is the WG2.2
// `0.75*latStep` honeycomb the Flow Globe describes (its own `buildHexLattice` is
// the SPHERICAL hexasphere, unusable for a regional flat map).
//
// `.67.15`: the fill colour is the CALLER's metric scale — for RTT,
// `colorByRttMs` (`resolveRttThresholds`, the config_rtt.html thresholds) — the
// SAME scale the markers use (ONE scale; the `.67.3` field-vs-marker
// agree-invariant), with NO data-range normalisation.

import {
    buildLatLngIndex,
    greatCircleKm,
} from "./flow_globe_hexbins.mjs";

// PINNED halo shape: R_i = clamp(baseKm*sqrt(tp_i/tpRef), minKm, maxKm).
export const HALO_BASE_KM = 12;
export const HALO_MIN_KM = 2;
export const HALO_MAX_KM = 45;
// PINNED fixed hex HEIGHT (deg lat) + the lattice cap (uniform; coarsens to fit).
export const PLANAR_HEX_LAT_STEP_DEG = 0.005;
export const PLANAR_HEX_MAX_CELLS = 30000;
// The per-cell fill opacity is `alpha * HALO_OPACITY` (0.25 may read faint).
export const HALO_OPACITY = 0.35;

const DEG = Math.PI / 180;
const SQRT3 = Math.sqrt(3);
const KM_PER_DEG = 111.32;

function clamp(value, lo, hi) {
    return Math.max(lo, Math.min(hi, value));
}

// The pointy-top hexagon RING (closed, [lng,lat]) around a centre.
export function hexRing(lat, lng, rLat, rLng, scale = 1) {
    const ring = [];
    for (let k = 0; k < 6; k += 1) {
        const ang = (90 + (60 * k)) * DEG; // pointy-top: a vertex at the top
        ring.push([lng + (Math.cos(ang) * rLng * scale), lat + (Math.sin(ang) * rLat * scale)]);
    }
    ring.push(ring[0].slice()); // close the polygon
    return ring;
}

// The per-site halo radius (km): the `coverageRadiusKm` CLAMP shape, throughput-derived.
export function haloRadiusKm(throughput, tpRef, baseKm = HALO_BASE_KM, minKm = HALO_MIN_KM, maxKm = HALO_MAX_KM) {
    const ref = Math.max(1, Number(tpRef) || 1);
    const tp = Math.max(0, Number(throughput) || 0);
    return clamp(baseKm * Math.sqrt(tp / ref), minKm, maxKm);
}

// Every site in the 3x3 bucket ring around (lat,lng), with its great-circle km.
function sitesAround(index, lat, lng, ring = 1) {
    const out = [];
    if (!index || index.buckets.size === 0) {
        return out;
    }
    const { buckets, bucketDeg, lngBuckets } = index;
    const a0 = Math.floor(lat / bucketDeg);
    const b0 = Math.floor(((((lng + 180) % 360) + 360) % 360) / bucketDeg);
    for (let a = a0 - ring; a <= a0 + ring; a += 1) {
        for (let b = b0 - ring; b <= b0 + ring; b += 1) {
            const wrapped = ((b % lngBuckets) + lngBuckets) % lngBuckets;
            const list = buckets.get(`${a}:${wrapped}`);
            if (!list) {
                continue;
            }
            for (const point of list) {
                out.push({ point, distKm: greatCircleKm(lat, lng, point.lat, point.lng) });
            }
        }
    }
    return out;
}

// Build the halo hexbin over the site bbox.
//   options.colorblind -> the CVD ramp
//   options.latStep / maxCells / baseKm / minKm / maxKm / opacity
export function planarHaloHexbin(points, options = {}) {
    const list = (Array.isArray(points) ? points : []).filter(
        (p) => p
            && Number.isFinite(p.lng)
            && Number.isFinite(p.lat)
            && Number.isFinite(p.value),
    );
    if (list.length === 0) {
        return { features: [], cells: 0, tpRef: 1, latStep: 0 };
    }

    let west = Infinity;
    let east = -Infinity;
    let south = Infinity;
    let north = -Infinity;
    let tpRef = 1;
    for (const p of list) {
        west = Math.min(west, p.lng);
        east = Math.max(east, p.lng);
        south = Math.min(south, p.lat);
        north = Math.max(north, p.lat);
        tpRef = Math.max(tpRef, Number(p.throughput) || 0);
    }
    const baseKm = Number(options.baseKm) > 0 ? Number(options.baseKm) : HALO_BASE_KM;
    const minKm = Number(options.minKm) > 0 ? Number(options.minKm) : HALO_MIN_KM;
    const maxKm = Number(options.maxKm) > 0 ? Number(options.maxKm) : HALO_MAX_KM;
    const opacity = Number.isFinite(Number(options.opacity)) ? Number(options.opacity) : HALO_OPACITY;
    const cap = Number(options.maxCells) > 0 ? Number(options.maxCells) : PLANAR_HEX_MAX_CELLS;
    const cosMid = Math.max(0.05, Math.cos(((north + south) / 2) * DEG));

    // PIL: attach each site's halo radius (busy => far).
    const sites = list.map((p) => ({
        lat: p.lat,
        lng: p.lng,
        value: p.value,
        throughput: Number(p.throughput) || 0,
        radiusKm: haloRadiusKm(p.throughput, tpRef, baseKm, minKm, maxKm),
    }));
    const index = buildLatLngIndex(sites, Math.max(0.25, maxKm / KM_PER_DEG));

    // PINNED fixed size; coarsen UNIFORMLY only to respect the cap.
    let latStep = Number(options.latStep) > 0 ? Number(options.latStep) : PLANAR_HEX_LAT_STEP_DEG;
    let rowStep = 0.75 * latStep;
    let rLat = latStep / 2;
    let rLng = rLat / cosMid;
    let colStep = SQRT3 * rLng;
    let rows = 0;
    let cols = 0;
    for (;;) {
        rowStep = 0.75 * latStep;
        rLat = latStep / 2;
        rLng = rLat / cosMid;
        colStep = SQRT3 * rLng;
        rows = Math.ceil((north - south) / rowStep) + 3;
        cols = Math.ceil((east - west) / colStep) + 3;
        if ((rows * cols) <= cap || latStep >= 5) {
            break;
        }
        latStep *= 1.25;
    }
    const anchorLat = south - rowStep;
    const anchorLng = west - colStep;

    // PASS 1: the covered cells (max coverage over the sites whose halo reaches it).
    const covered = [];
    for (let row = 0; row < rows; row += 1) {
        const cLat = anchorLat + (row * rowStep);
        if (cLat > north + rowStep) {
            break;
        }
        const offset = (row % 2) ? colStep / 2 : 0;
        for (let col = 0; col < cols; col += 1) {
            const cLng = anchorLng + (col * colStep) + offset;
            if (cLng > east + colStep) {
                break;
            }
            let best = null; // strongest coverage
            for (const { point, distKm } of sitesAround(index, cLat, cLng)) {
                if (distKm > point.radiusKm) {
                    continue;
                }
                const alpha = 1 - (distKm / point.radiusKm);
                if (!best || alpha > best.alpha) {
                    best = { alpha, site: point };
                }
            }
            if (!best) {
                continue; // outside every halo => EMPTY (the basemap shows)
            }
            covered.push({
                cLat,
                cLng,
                rtt: best.site.value,
                alpha: best.alpha,
                throughput: best.site.throughput,
            });
        }
    }

    // PASS 2: colour each halo cell with the CALLER's scale (`.67.15`: the CONFIG
    // `colorByRttMs` / `resolveRttThresholds`, the SAME scale the markers use).
    const colorFor = typeof options.colorFor === "function"
        ? options.colorFor
        : () => "rgb(136, 147, 165)";
    const features = covered.map((c) => ({
        type: "Feature",
        geometry: {
            type: "Polygon",
            coordinates: [hexRing(c.cLat, c.cLng, rLat, rLng, 1)],
        },
        properties: {
            rtt: c.rtt,
            alpha: c.alpha,
            throughput: c.throughput,
            metricColor: colorFor(c.rtt),
            // `.67.14`: the fill is `alpha * opacity` (a radial gradient).
            opacity: c.alpha * opacity,
        },
    }));

    return { features, cells: features.length, tpRef, latStep };
}
