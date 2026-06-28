export const WORLD_MAP_ENCODING_DESCRIPTION = "Larger points indicate more traffic, and point colors indicate RTT.";

export function worldMapStatusText(pointCount) {
    const noun = pointCount === 1 ? "location" : "locations";
    return `Showing ${pointCount} recent endpoint ${noun}. ${WORLD_MAP_ENCODING_DESCRIPTION}`;
}

export function applyWorldMapFeatureUpdate(dashlet, features, eventName) {
    const hasData = features.length > 0;
    dashlet._showEmpty(!hasData);
    if (!hasData) {
        dashlet.graph?.updateFeatures(features);
        return;
    }

    dashlet._setStatus(worldMapStatusText(features.length));
    dashlet.traceRender("onMessage", { eventName, pointCount: features.length });
    try {
        dashlet.graph.updateFeatures(features);
        dashlet.traceRender("update-ok", { eventName, pointCount: features.length });
    } catch (err) {
        dashlet.traceRender("update-error", { eventName, error: err && err.message ? err.message : String(err) });
        throw err;
    }
}
