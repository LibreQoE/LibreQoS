// Pure helpers for the Site Map's configurable tile base.
//
// The server injects window.lqosInsightBaseUrl from the config
// (long_term_stats.lts_url via normalize_insight_base_url).  The default keeps
// the historical Insight OSM cache; a malformed/empty value falls back to it.
const DEFAULT_INSIGHT_BASE = "https://insight.libreqos.com/";

export function resolveInsightBase(raw) {
    const value = (raw === undefined || raw === null) ? "" : String(raw).trim();
    if (!value) {
        return DEFAULT_INSIGHT_BASE;
    }
    const candidate = /^[a-z][a-z0-9+.-]*:\/\//i.test(value) ? value : `https://${value}`;
    try {
        const url = new URL(candidate);
        return url.href.endsWith("/") ? url.href : `${url.href}/`;
    } catch (_) {
        return DEFAULT_INSIGHT_BASE;
    }
}

// The bbox endpoint (https) and the raster tile host+path (the custom
// `insight://` MapLibre protocol swaps the scheme to https at fetch time).
export function insightTileUrls(raw) {
    const base = resolveInsightBase(raw);
    const url = new URL(base);
    return {
        base,
        bboxUrl: `${base}tiles/api/bbox`,
        tileHost: url.host,
        tilePath: `${url.pathname}tiles/{z}/{y}/{x}.png`,
    };
}
