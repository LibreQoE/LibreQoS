// The words for a CAKE chart's series names.
//
// The chart series carry CAKE's tin names as identifiers, not as display text:
// `CAKE_TIN_LEGEND_PAIRS`, `cakeTinIndexFor` and the tooltip's `" Up"` suffix all
// match on them, and translating the identifiers would break every one of those
// comparisons. This module is the single place an identifier becomes a label, so
// the legend and the tooltip translate together and nothing else has to know.
//
// It lives beside `cake_tin_order.mjs` rather than in `cake_history.js` because
// that file's imports are bundle-only: a pure module is what lets the Node
// contract test render a tin label in both locales.

import {t} from "../i18n/index.js";

/**
 * The legend/tooltip text for a CAKE series identifier. One literal `t()` call
 * per branch: the catalog extractor reads keys, it does not run code.
 * @param {string} name Series identifier, e.g. "Bulk" or "RT Voice Up".
 * @returns {string} The label, or the identifier when it is not a mapped series.
 */
// translatable: false — "Bulk" "Best Effort" "RT Video" "RT Voice" "Voice" "Bulk Up" "Best Effort Up" "RT Video Up" "RT Voice Up" "Queue Length" "Queue Length Up" "Down" "Up" — the CAKE series identifiers these comparisons match on; every label they return comes from t()
export function cakeTinLabel(name) {
    switch (name) {
        case "Bulk": return t("graph.cake.tin.bulk");
        case "Best Effort": return t("graph.cake.tin.best_effort");
        case "RT Video": return t("graph.cake.tin.rt_video");
        // CAKE names the voice tin's upload series `RT Voice Up` and its download
        // series `Voice`, so stripping the suffix in `cakeSeriesDisplay` lands on
        // `RT Voice`. Both names are that one tin and share its label; the legend
        // keeps CAKE's own series names.
        case "Voice":
        case "RT Voice": return t("graph.cake.tin.voice");
        case "Bulk Up": return t("graph.cake.tin.bulk_up");
        case "Best Effort Up": return t("graph.cake.tin.best_effort_up");
        case "RT Video Up": return t("graph.cake.tin.rt_video_up");
        case "RT Voice Up": return t("graph.cake.tin.rt_voice_up");
        case "Queue Length": return t("graph.cake.queue_length");
        case "Queue Length Up": return t("graph.cake.queue_length_up");
        default: return name;
    }
}

/**
 * The label and direction one tooltip row shows for a series identifier: a tin
 * series reads "Bulk (upload)"; the per-direction `Down`/`Up` series carry no
 * tin, so they read as the direction alone.
 * @param {string} seriesName
 * @returns {{label: string, direction: string}}
 */
export function cakeSeriesDisplay(seriesName) {
    const isUpload = seriesName.endsWith(" Up") || seriesName === "Up";
    const direction = isUpload ? t("graph.series.upload") : t("graph.series.download");
    if (seriesName === "Down" || seriesName === "Up") {
        return {label: "", direction};
    }
    // The paired upload series carries its direction in the identifier suffix;
    // the tooltip renders it once, in the parenthetical.
    return {label: cakeTinLabel(isUpload ? seriesName.slice(0, -3) : seriesName), direction};
}

/**
 * ECharts legend `formatter`: legend text comes from the series name, which is
 * the identifier.
 * @param {string} name Series identifier.
 * @returns {string}
 */
export function cakeLegendFormatter(name) {
    if (name === "Down" || name === "Up") {
        return cakeSeriesDisplay(name).direction;
    }
    return cakeTinLabel(name);
}
