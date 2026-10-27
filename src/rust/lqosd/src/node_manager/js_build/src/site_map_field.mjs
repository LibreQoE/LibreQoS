// Pure geometry + kernel for the Site Map's value-interpolated field (`.67.3`).
//
// The field is a MapLibre `image` source whose four lon/lat corners MapLibre
// maps LINEARLY IN WEB-MERCATOR onto the canvas.  A canvas that is linear in
// LATITUDE therefore draws a site's own cell at the wrong latitude versus its
// marker (worst mid-bbox, growing with the lat span).  To align the field with
// the markers, the grid rows are built in Mercator-y: x is linear in lon, which
// is linear in Mercator-x, so only the y axis changes.  With an axis-aligned
// bbox the image transform is affine, so the residual alignment error is within
// one grid cell (the "alignment budget") across the whole extent.
//
// Distances use an equirectangular km approximation (fine at site/city scale).
// The value at a cell is IDW — a weighted MEAN of the k nearest sites (p=2) —
// chosen for smoothness.  It is NOT a median and it is NOT measured between
// sites, so the UI must label it "estimated (interpolated)".

const WEB_MERCATOR_MAX_LAT = 85.05112878;
const DEG = Math.PI / 180;

// Web-Mercator y for a latitude (degrees). Monotonic in latitude; the exact
// offset/scale is irrelevant because it is only interpolated between two lats.
export function mercatorY(lat) {
    const clamped = Math.max(
        -WEB_MERCATOR_MAX_LAT,
        Math.min(WEB_MERCATOR_MAX_LAT, Number(lat)),
    );
    return Math.log(Math.tan((Math.PI / 4) + ((clamped * DEG) / 2)));
}

export function inverseMercatorY(y) {
    return (2 * Math.atan(Math.exp(Number(y))) - (Math.PI / 2)) / DEG;
}

// Equirectangular km distance between two lon/lat points.
export function kmBetween(lng1, lat1, lng2, lat2) {
    const dx = (lng2 - lng1) * Math.cos(((lat1 + lat2) / 2) * DEG) * 111.32;
    const dy = (lat2 - lat1) * 110.574;
    return Math.sqrt((dx * dx) + (dy * dy));
}

// `.67.5`/`.67.8`: the per-site glow radius. Extent ONLY — the field color is
// unchanged. radius = baseKm * sqrt(value / valueRef), clamped to [minKm, maxKm]
// so a huge site cannot paint the map and a small site stays visible. `value`
// is the glow source (`.67.5` circuit count; `.67.8` live throughput). sqrt is
// monotonic: a larger value => an equal-or-greater radius.
export function glowRadiusKm(value, valueRef, baseKm, minKm, maxKm) {
    const c = Math.max(0, Number(value) || 0);
    const ref = Math.max(1, Number(valueRef) || 1);
    const base = Math.max(0, Number(baseKm) || 0);
    const min = Math.max(0, Number(minKm) || 0);
    const max = Math.max(min, Number(maxKm) || min);
    const scaled = base * Math.sqrt(c / ref);
    return Math.min(max, Math.max(min, scaled));
}

// The latitude of grid row `gy` (0 = north) for a Mercator-aligned image.
export function gridLatitude(bbox, gy, height) {
    const yNorth = mercatorY(bbox.north);
    const ySouth = mercatorY(bbox.south);
    const y = yNorth - (((gy + 0.5) / height) * (yNorth - ySouth));
    return inverseMercatorY(y);
}

// Rasterize the (smoothed) IDW field into an RGBA byte array (row-major, north
// first) to feed a canvas ImageData. `options`:
//   width/height  grid size (downsample for large point sets)
//   k             max nearest sites considered (clamped to points.length)
//   power         IDW exponent (p; 2 = inverse-square)
//   falloffKm     default alpha reach (used when a point carries no `radiusKm`)
//   smoothKm      smoothing LENGTH-SCALE s: weight = 1/(d^2 + s^2)^(p/2)
//   colorFor(v)   -> [r,g,b] the metric color scale (marker mapping)
//
// A point may carry a per-point `radiusKm` (`.67.5`): the alpha fade uses the
// NEAREST point's radius when present, else `falloffKm`.
//
// The length-scale `s` is what removes the per-point bullseye: it bounds the
// weight at a site (no 1/d^p singularity), so a site no longer dominates its own
// cells — the field flattens while an isolated hotspot keeps alpha=1 and the
// local-maximum value. `s=0` falls back to classic IDW with the d->0 guard.
//
// Cost is O(grid cells x sites); the caller downsamples the grid above a point
// threshold. There is no spatial index (a k-d tree is the future bound).
export function buildFieldGrid(points, bbox, options = {}) {
    const width = Math.max(1, Math.round(Number(options.width) || 1));
    const height = Math.max(1, Math.round(Number(options.height) || 1));
    const data = new Uint8ClampedArray(width * height * 4);
    const list = Array.isArray(points) ? points : [];
    if (list.length === 0) {
        return { width, height, data };
    }
    const k = Math.max(1, Math.min(Math.round(Number(options.k) || 8), list.length));
    const power = Number.isFinite(Number(options.power)) ? Number(options.power) : 2;
    const falloffKm = Math.max(1e-3, Number(options.falloffKm) || 25);
    const smooth = Math.max(0, Number(options.smoothKm) || 0);
    const colorFor = typeof options.colorFor === "function" ? options.colorFor : () => [0, 0, 0];
    const alphaCutoff = Number.isFinite(Number(options.alphaCutoff)) ? Number(options.alphaCutoff) : 0.01;

    const spanLng = bbox.east - bbox.west;
    const yNorth = mercatorY(bbox.north);
    const ySouth = mercatorY(bbox.south);
    const spanMerc = yNorth - ySouth;
    const bestD = new Float64Array(k);
    const bestI = new Int32Array(k);

    for (let gy = 0; gy < height; gy += 1) {
        const y = yNorth - (((gy + 0.5) / height) * spanMerc);
        const lat = inverseMercatorY(y);
        for (let gx = 0; gx < width; gx += 1) {
            const lng = bbox.west + (((gx + 0.5) / width) * spanLng);
            for (let i = 0; i < k; i += 1) {
                bestD[i] = Infinity;
                bestI[i] = -1;
            }
            for (let pi = 0; pi < list.length; pi += 1) {
                const pt = list[pi];
                const d = kmBetween(lng, lat, pt.lng, pt.lat);
                if (d < bestD[k - 1]) {
                    let j = k - 1;
                    while (j > 0 && bestD[j - 1] > d) {
                        bestD[j] = bestD[j - 1];
                        bestI[j] = bestI[j - 1];
                        j -= 1;
                    }
                    bestD[j] = d;
                    bestI[j] = pi;
                }
            }
            if (!Number.isFinite(bestD[0])) {
                continue;
            }
            // `.67.5`: fade using the NEAREST point's per-point radius when it
            // carries one (a site's glow extends with its circuit count); else
            // the global `falloffKm`. The smoothing length-scale `s` stays global.
            // Bounded: nearest-radius can leave a Voronoi hole where a
            // small-radius site abuts a bigger one; max-over-k is a follow-up.
            const nearest = bestI[0] >= 0 ? list[bestI[0]] : null;
            const nearestRadiusRaw = Number(nearest?.radiusKm);
            const nearestRadius = Number.isFinite(nearestRadiusRaw) && nearestRadiusRaw > 0
                ? nearestRadiusRaw
                : falloffKm;
            const alpha = Math.max(0, Math.min(1, 1 - (bestD[0] / nearestRadius)));
            if (alpha <= alphaCutoff) {
                continue;
            }
            let num = 0;
            let den = 0;
            for (let i = 0; i < k; i += 1) {
                const di = bestD[i];
                if (!Number.isFinite(di) || bestI[i] < 0) {
                    break;
                }
                const denom = (di * di) + (smooth * smooth);
                // d->0 guard for UNsmoothed IDW (s=0): exact site value.
                if (denom <= 0) {
                    num = list[bestI[i]].value;
                    den = 1;
                    break;
                }
                const weight = 1 / Math.pow(denom, power / 2);
                num += weight * list[bestI[i]].value;
                den += weight;
            }
            if (den <= 0) {
                continue;
            }
            const value = num / den;
            if (!Number.isFinite(value)) {
                continue;
            }
            const [r, g, b] = colorFor(value);
            const idx = ((gy * width) + gx) * 4;
            data[idx] = r;
            data[idx + 1] = g;
            data[idx + 2] = b;
            data[idx + 3] = Math.round(alpha * 255);
        }
    }
    return { width, height, data };
}
