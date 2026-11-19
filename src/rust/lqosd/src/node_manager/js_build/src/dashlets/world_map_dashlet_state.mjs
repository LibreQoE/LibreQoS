import {t, tn} from "../i18n/index.js";

/** The sentence that explains what a point's size and colour encode. */
export function worldMapEncodingDescription() {
    return t("dashlet.world_map.encoding");
}

export function worldMapStatusText(pointCount) {
    return tn("dashlet.world_map.status", pointCount, {
        count: pointCount,
        encoding: worldMapEncodingDescription(),
    });
}

export function applyWorldMapFeatureUpdate(dashlet, features, eventName) {
    const hasData = features.length > 0;
    dashlet._showEmpty(!hasData);
    if (!hasData) {
        dashlet.graph?.updateFeatures(features);
        return;
    }

    dashlet._setStatus(worldMapStatusText(features.length));
    dashlet.traceRender("onMessage", {eventName, pointCount: features.length});
    try {
        dashlet.graph.updateFeatures(features);
        dashlet.traceRender("update-ok", {eventName, pointCount: features.length});
    } catch (err) {
        dashlet.traceRender("update-error", {eventName, error: err && err.message ? err.message : String(err)});
        throw err;
    }
}
