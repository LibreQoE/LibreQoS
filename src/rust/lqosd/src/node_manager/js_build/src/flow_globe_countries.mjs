// `lqos-74t.50.7` Stage 2 — merged country-layer geometry helpers (pure; no THREE,
// no DOM) so the node contract tests can exercise the winding / antimeridian
// logic.
//
// The merged country cap triangulation (`THREE.ShapeUtils.triangulateShape`) needs
// the OUTER ring counter-clockwise (positive shoelace area) and its HOLES
// clockwise (negative) — Natural-Earth ring winding is NOT guaranteed. A ring that
// crosses the antimeridian is unwrapped around its first vertex; a ring that still
// spans > 180 deg is dropped (never a globe-spanning quad).

/** Shoelace signed area of a `[lng,lat]` ring (positive = counter-clockwise). */
export function signedRingArea(ring) {
    if (!Array.isArray(ring) || ring.length < 3) {
        return 0;
    }
    let area = 0;
    for (let i = 0, j = ring.length - 1; i < ring.length; j = i, i += 1) {
        const [x1, y1] = ring[j];
        const [x2, y2] = ring[i];
        area += x1 * y2 - x2 * y1;
    }
    return area / 2;
}

/**
 * Normalize a polygon's ring winding for `triangulateShape` (the pure equivalent
 * of `ShapeUtils.isClockWise`): ring 0 (the outer contour) CCW, every hole CW.
 * Returns new arrays; does not mutate `polygon`.
 */
export function normalizePolygonRings(polygon) {
    const rings = Array.isArray(polygon) ? polygon : [];
    return rings.map((ring, index) => {
        const copy = Array.isArray(ring) ? ring.slice() : [];
        const wantPositive = index === 0;
        const positive = signedRingArea(copy) > 0;
        return positive === wantPositive ? copy : copy.reverse();
    });
}

/**
 * Unwrap a ring's longitudes around its first vertex so a ring crossing the
 * antimeridian is contiguous. Returns `null` if the ring then spans > 180 deg (a
 * genuinely globe-spanning ring — the caller skips it).
 */
export function unwrapRing(ring) {
    if (!Array.isArray(ring) || ring.length === 0) {
        return null;
    }
    const base = ring[0][0];
    const out = ring.map(([lng, lat]) => {
        let x = lng;
        while (x - base > 180) x -= 360;
        while (base - x > 180) x += 360;
        return [x, lat];
    });
    let min = Infinity;
    let max = -Infinity;
    for (const [x] of out) {
        if (x < min) min = x;
        if (x > max) max = x;
    }
    return (max - min) > 180 ? null : out;
}

/** Parse the alpha of a CSS `rgb()/rgba()` colour string (default `fallback`). */
export function colorAlpha(value, fallback = 1) {
    if (typeof value !== "string") {
        return fallback;
    }
    const match = value.match(/rgba?\([^,]+,[^,]+,[^,]+,\s*([0-9.]+)\s*\)/i);
    if (!match) {
        return fallback;
    }
    const alpha = Number.parseFloat(match[1]);
    return Number.isFinite(alpha) ? Math.max(0, Math.min(1, alpha)) : fallback;
}

/** Map one GeoJSON geometry to its polygons (each = outer ring + its holes). */
export function geometryPolygons(geometry) {
    if (!geometry) {
        return [];
    }
    if (geometry.type === "Polygon") {
        return [geometry.coordinates];
    }
    if (geometry.type === "MultiPolygon") {
        return Array.isArray(geometry.coordinates) ? geometry.coordinates : [];
    }
    return [];
}

/** Split one `[lng,lat]` triangle at its edge midpoints (4 sub-triangles). */
export function subdivideTriangle([a, b, c]) {
    const ab = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
    const bc = [(b[0] + c[0]) / 2, (b[1] + c[1]) / 2];
    const ca = [(c[0] + a[0]) / 2, (c[1] + a[1]) / 2];
    return [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]];
}

/** Approximate surface arc of one edge in degrees (lng scaled by cos(lat)). */
function edgeSpanDeg(a, b) {
    const dLat = Math.abs(a[1] - b[1]);
    let dLng = Math.abs(a[0] - b[0]) % 360;
    if (dLng > 180) dLng = 360 - dLng;
    const midLat = ((a[1] + b[1]) / 2) * (Math.PI / 180);
    return Math.max(dLat, dLng * Math.abs(Math.cos(midLat)));
}

/** The longest edge of a `[lng,lat]` triangle, in approximate surface degrees. */
export function triangleMaxEdgeDeg(tri) {
    const [a, b, c] = tri;
    return Math.max(edgeSpanDeg(a, b), edgeSpanDeg(b, c), edgeSpanDeg(c, a));
}

/**
 * `lqos-74t.50.7`: subdivide a `[lng,lat]` triangle until its longest edge is
 * `<= maxEdgeDeg` (bounded by `maxDepth`). A coarse earcut triangle has a chord
 * sag `R(1 - cos(theta/2))` that can exceed the country altitude, so the flat cap
 * sinks through the opaque globe (dark patches). Bounding the edge bounds the sag.
 */
export function subdivideTriangleToMaxEdge(tri, maxEdgeDeg, maxDepth = 6) {
    let tris = [tri];
    for (let depth = 0; depth < maxDepth; depth += 1) {
        let split = false;
        const next = [];
        for (const t of tris) {
            if (triangleMaxEdgeDeg(t) > maxEdgeDeg) {
                next.push(...subdivideTriangle(t));
                split = true;
            } else {
                next.push(t);
            }
        }
        tris = next;
        if (!split) {
            break;
        }
    }
    return tris;
}
