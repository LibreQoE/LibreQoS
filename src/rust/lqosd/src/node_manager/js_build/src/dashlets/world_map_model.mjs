const MAX_SYMBOL_SIZE = 8;
const MIN_SYMBOL_SIZE = 1;

export function emptyFeatureCollection() {
    return { type: "FeatureCollection", features: [] };
}

export function rowsToEndpointFeatures(rows, colorForRtt) {
    if (!Array.isArray(rows)) {
        return [];
    }
    const validRows = [];
    let maxBytes = 0;
    for (let i = 0; i < rows.length; i++) {
        const lat = Number(rows[i]?.[0]);
        const lon = Number(rows[i]?.[1]);
        if (!Number.isFinite(lat) || !Number.isFinite(lon) || lat < -90 || lat > 90 || lon < -180 || lon > 180) {
            continue;
        }
        const rawBytes = Number(rows[i]?.[3] || 0);
        const bytes = Number.isFinite(rawBytes) ? rawBytes : 0;
        if (bytes > maxBytes) {
            maxBytes = bytes;
        }
        validRows.push({ row: rows[i], lat, lon, bytes });
    }
    const features = [];
    for (let i = 0; i < validRows.length; i++) {
        const { row, lat, lon, bytes } = validRows[i];
        const rawRtt = Number(row?.[4] || 0);
        const rtt = Number.isFinite(rawRtt) ? rawRtt : 0;
        let weight = 0;
        if (maxBytes > 0) {
            weight = Math.sqrt(bytes / maxBytes);
            if (!Number.isFinite(weight) || weight < 0) weight = 0;
            if (weight > 1) weight = 1;
        }
        features.push({
            type: "Feature",
            geometry: {
                type: "Point",
                coordinates: [lon, lat],
            },
            properties: {
                color: colorForRtt(rtt),
                radius: Math.round(MIN_SYMBOL_SIZE + (MAX_SYMBOL_SIZE - MIN_SYMBOL_SIZE) * weight),
                weight,
            },
        });
    }
    return features;
}
