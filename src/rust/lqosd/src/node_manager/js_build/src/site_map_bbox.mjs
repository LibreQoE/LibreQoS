// Pure helpers for the Site Map's offline-degradable bbox fallback.
//
// The map's initial center/zoom comes from the Insight bbox endpoint
// (`insight.libreqos.com/tiles/api/bbox`).  When that endpoint (and its raster
// tiles) is unreachable — e.g. a host whose DNS fails for the Insight domain —
// the map must still initialize and render the site overlay.  This computes a
// center + zoom directly from the sites' own coordinates.
export function localBboxCenter(pairs) {
    const lats = [];
    const lons = [];
    if (Array.isArray(pairs)) {
        for (const pair of pairs) {
            if (!Array.isArray(pair) || pair.length < 2) {
                continue;
            }
            const lat = Number(pair[0]);
            const lon = Number(pair[1]);
            if (Number.isFinite(lat) && Number.isFinite(lon)) {
                lats.push(lat);
                lons.push(lon);
            }
        }
    }
    if (lats.length === 0) {
        return { lat: 0, lon: 0, zoom: 2 };
    }
    const minLat = Math.min(...lats);
    const maxLat = Math.max(...lats);
    const minLon = Math.min(...lons);
    const maxLon = Math.max(...lons);
    const span = Math.max(maxLat - minLat, maxLon - minLon, 0.0001);
    return {
        lat: (minLat + maxLat) / 2,
        lon: (minLon + maxLon) / 2,
        zoom: zoomForSpan(span),
    };
}

// A coarse degrees-span -> zoom map (one step per ~2x), clamped to a sane range.
function zoomForSpan(span) {
    if (!Number.isFinite(span) || span <= 0) {
        return 2;
    }
    if (span >= 60) return 2;
    if (span >= 30) return 3;
    if (span >= 15) return 4;
    if (span >= 8) return 5;
    if (span >= 4) return 6;
    if (span >= 2) return 7;
    if (span >= 1) return 8;
    if (span >= 0.5) return 9;
    if (span >= 0.25) return 10;
    return 11;
}
