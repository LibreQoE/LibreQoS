import { colorByQoqScore, colorByRttMs, resolveRttThresholds } from "./helpers/color_scales";
import { isColorBlindMode } from "./helpers/colorblind";
import { isDarkMode } from "./helpers/dark_mode";
import { isRedacted } from "./helpers/redact";
import { effectiveMax } from "./helpers/network_rates.mjs";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { lerpGreenToRedViaOrange } from "./helpers/scaling";
import { get_ws_client, subscribeWS } from "./pubsub/ws";
import { localBboxCenter } from "./site_map_bbox.mjs";
import { insightTileUrls } from "./site_map_config.mjs";
import { buildFieldGrid, glowRadiusKm } from "./site_map_field.mjs";
import { planarHaloHexbin } from "./site_map_hexbin.mjs";

const wsClient = get_ws_client();

const INITIAL_REQUEST_TIMEOUT_MS = 2500;
const HISTORY_WINDOW_MS = 30_000;

// The tile base is CONFIGURABLE: the server injects window.lqosInsightBaseUrl
// from long_term_stats.lts_url.  The default keeps the historical Insight OSM
// cache; a host whose DNS fails for the base still gets a working map via the
// local-bbox fallback in ensureMapInitialized.
const INSIGHT_TILES = insightTileUrls(
    typeof window !== "undefined" ? window.lqosInsightBaseUrl : undefined
);
const TILE_BBOX_URL = INSIGHT_TILES.bboxUrl;
// NOTE: This key is intentionally non-secret. The remote OSM cache uses it as a lightweight gate.
// It must match both the bbox Authorization token and the tile `key=` query param.
const OSM_CACHE_KEY = "LibreQoSRocks";
const WEB_MERCATOR_MAX_LAT = 85.05112878;
const INSIGHT_TILE_PROTOCOL = "insight";
// Insight OSM cache tiles: `z/y/x.png` (Cloudflare cache-friendly). See `src/lts2/rust/osm_cache/src/http.rs`.
// The Insight tile server returns `503 + Retry-After` while it fetches missing tiles, so we route
// requests through a custom MapLibre protocol handler that retries before surfacing errors.
const TILE_URL_TEMPLATE = `${INSIGHT_TILE_PROTOCOL}://${INSIGHT_TILES.tileHost}${INSIGHT_TILES.tilePath}?key=${encodeURIComponent(OSM_CACHE_KEY)}`;
const TILE_ATTRIBUTION = "© OpenStreetMap contributors";
const TILE_MAX_ZOOM = 17;

const INITIAL_FIT_PADDING = 70;
const INITIAL_FIT_MAX_ZOOM = 11.5;
const SINGLE_POINT_INITIAL_ZOOM = 11.5;
const SITE_LABEL_MIN_ZOOM = 6;
const SELECTED_SITE_LABEL_MIN_ZOOM = 4;
const MAX_SITE_LABELS = 24;

const urlParams = new URLSearchParams(window.location.search);
const FIXTURE_MODE = urlParams.get("fixture") === "1";
const FIXTURE_NETWORK_TREE = [
    [0, {
        id: "site-central",
        name: "CENTRAL",
        type: "Site",
        immediate_parent: null,
        latitude: 39.0997,
        longitude: -92.2196,
        configured_max_throughput: [2000, 800],
        current_throughput: [20000000, 4000000],
        qoo: [96.5, 97.2],
        rtts: [18.4, 22.1],
    }],
    [1, {
        id: "site-north",
        name: "NORTH",
        type: "Site",
        immediate_parent: 0,
        latitude: 39.2564,
        longitude: -92.1842,
        configured_max_throughput: [1500, 400],
        current_throughput: [80000000, 12000000],
        qoo: [88.0, 84.0],
        rtts: [35.0, 41.0],
    }],
    [2, {
        id: "site-east",
        name: "EAST",
        type: "Site",
        immediate_parent: 0,
        latitude: 39.0482,
        longitude: -91.9631,
        configured_max_throughput: [500, 100],
        current_throughput: [6000000, 1000000],
        qoo: [62.0, 70.0],
        rtts: [120.0, 165.0],
    }],
    [3, {
        id: "site-rural",
        name: "RURAL",
        type: "Site",
        immediate_parent: 1,
        latitude: 39.3689,
        longitude: -92.4509,
        configured_max_throughput: [100, 20],
        current_throughput: [1200000, 240000],
        qoo: [41.0, 52.0],
        rtts: [55.0, 60.0],
    }],
    [4, {
        id: "site-unmapped",
        name: "UNMAPPED",
        type: "Site",
        immediate_parent: 0,
        configured_max_throughput: [50, 10],
        current_throughput: [0, 0],
        qoo: [null, null],
        rtts: [],
    }],
    [5, {
        id: "ap-north-1",
        name: "NORTH AP 1",
        type: "AP",
        immediate_parent: 1,
        latitude: 39.2753,
        longitude: -92.2116,
        current_throughput: [15000000, 2000000],
        qoo: [92.0, 90.0],
        rtts: [28.0, 33.0],
    }],
    [6, {
        id: "ap-north-2",
        name: "NORTH AP 2",
        type: "AP",
        immediate_parent: 1,
        current_throughput: [10000000, 1000000],
        qoo: [null, 80.0],
        rtts: [25.0, 29.0],
    }],
    [7, {
        id: "ap-east-1",
        name: "EAST AP",
        type: "AP",
        immediate_parent: 2,
        latitude: 39.0349,
        longitude: -91.9387,
        current_throughput: [3000000, 400000],
        qoo: [55.0, 60.0],
        rtts: [180.0, 220.0],
    }],
];

const OSM_RASTER_SOURCE_ID = "site-map-osm";
const OSM_RASTER_LAYER_ID = "site-map-osm-tiles";
const SITE_SOURCE_ID = "site-map-sites";
const AP_SOURCE_ID = "site-map-aps";
const SITE_LINK_SOURCE_ID = "site-map-site-links";
const FANOUT_LINE_SOURCE_ID = "site-map-fanout-lines";
const FANOUT_POINT_SOURCE_ID = "site-map-fanout-points";
const SITE_CLUSTER_LAYER_ID = "site-map-site-clusters";
const SITE_POINTS_LAYER_ID = "site-map-site-points";
const AP_POINTS_LAYER_ID = "site-map-ap-points";
const FANOUT_LINE_LAYER_ID = "site-map-fanout-lines-layer";
const FANOUT_POINT_LAYER_ID = "site-map-fanout-points-layer";
const SITE_LINK_LAYER_ID = "site-map-site-links-line";
// `.67`: the site heat field (a native MapLibre heatmap layer, below the markers).
// `.67.12`: the flat 2D HEXBIN (per-cell color = RTT, size = throughput) — a
// native MapLibre `fill` over a GeoJSON of scaled hexagons. Replaces the density
// heatmap in Heatmap mode.
const SITE_HEX_SOURCE_ID = "site-map-hex-source";
const SITE_HEX_LAYER_ID = "site-map-hex-layer";
// `.67.1` (ii): a PARALLEL unclustered site source + layer so the Cluster
// toggle can show individual points without hiding everything at low zoom.
const SITE_PLAIN_SOURCE_ID = "site-map-sites-plain";
const SITE_PLAIN_POINTS_LAYER_ID = "site-map-sites-plain-points";
// `.67.3`: the value-interpolated RTT field — IDW on a grid rendered to a canvas
// and served as a MapLibre `image` source (below the markers). NOT density.
const SITE_FIELD_SOURCE_ID = "site-map-rtt-field-source";
const SITE_FIELD_LAYER_ID = "site-map-rtt-field-layer";
const FIELD_GRID_W = 256;
const FIELD_GRID_H = 160;
const FIELD_GRID_W_LARGE = 160;
const FIELD_GRID_H_LARGE = 100;
const FIELD_LARGE_POINT_COUNT = 200;
// `.67.4` Lever 3: the IDW neighbourhood is widened + the exponent lowered, and
// — the real fix for the per-point bullseye — a SMOOTHING LENGTH-SCALE `s` is
// added to the kernel (`w = 1/(d^2 + s^2)^(p/2)`, smoothed IDW / Shepard with a
// Gaussian-like length-scale). That removes the `d->0` singularity so a site no
// longer dominates its own cells (no bullseye) while an isolated hotspot still
// keeps alpha=1 and the local-maximum value (reads red).
const FIELD_IDW_K = 12;
const FIELD_IDW_POWER = 1.5;
const FIELD_SMOOTH_MIN_KM = 0.5;
const FIELD_SMOOTH_FRACTION = 0.08;
// `.67.4`: marker polish (Lever 1 + 2). Sites are the ONLY ringed markers:
// small, near-uniform, dark-ringed, opaque dots. In VALUE-field mode the
// ambient AP markers become tiny RING-LESS dots (a PAINT change only — the AP
// source still feeds `fieldPoints()`), fading in above AP_DECLUTTER_MIN_ZOOM.
// The selection-gated FANOUT_POINT overlay is intentionally left untouched.
const SITE_DOT_OPACITY = 0.92;
const SITE_RING_WIDTH = 1.2;
const AP_DECLUTTER_DOT_RADIUS = 2.2;
const AP_DECLUTTER_MIN_ZOOM = 7;
const AP_DECLUTTER_DOT_OPACITY = 0.55;
// `.67.5`: per-site glow radius ∝ sqrt(circuit count), clamped to a fraction of
// the base falloff so a huge site cannot paint the map and a small site (or an
// AP / count=0 point) stays visible. countRef = the max count among the plotted
// sites (self-scaling; floor 1).
const GLOW_RADIUS_MIN_FRACTION = 0.3;
const GLOW_RADIUS_MAX_FRACTION = 1.6;

const INSIGHT_TILE_MAX_PARALLEL_FETCHES = 2;
let insightTileFetchActive = 0;
const insightTileFetchWaiters = [];
let insightTileRateLimitUntil = 0;

async function withInsightTileFetchSlot(fn) {
    while (Date.now() < insightTileRateLimitUntil) {
        await sleepMs(Math.min(30_000, insightTileRateLimitUntil - Date.now()));
    }
    if (insightTileFetchActive >= INSIGHT_TILE_MAX_PARALLEL_FETCHES) {
        await new Promise((resolve) => insightTileFetchWaiters.push(resolve));
    }
    insightTileFetchActive += 1;
    try {
        return await fn();
    } finally {
        insightTileFetchActive = Math.max(0, insightTileFetchActive - 1);
        const next = insightTileFetchWaiters.shift();
        if (next) {
            next();
        }
    }
}

function listenOnceWithTimeout(eventName, timeoutMs, handler, onTimeout) {
    let done = false;
    const wrapped = (msg) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    const timer = setTimeout(() => {
        if (done) return;
        done = true;
        wsClient.off(eventName, wrapped);
        onTimeout();
    }, timeoutMs);
    wsClient.on(eventName, wrapped);
}

function escapeHtml(value) {
    return String(value ?? "")
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function parseRetryAfterMs(headerValue) {
    if (headerValue === null || headerValue === undefined) {
        return null;
    }
    const raw = String(headerValue).trim();
    if (!raw) {
        return null;
    }
    const seconds = Number(raw);
    if (Number.isFinite(seconds)) {
        return Math.max(0, Math.round(seconds * 1000));
    }
    const parsed = Date.parse(raw);
    if (!Number.isFinite(parsed)) {
        return null;
    }
    return Math.max(0, parsed - Date.now());
}

function formatBitsPerSecond(bitsPerSecond) {
    return scaleNumber(Math.max(0, toNumber(bitsPerSecond, 0)), 1);
}

function formatPercent(value, digits = 0) {
    const numeric = Number(value);
    if (!Number.isFinite(numeric)) {
        return "No data";
    }
    return `${numeric.toFixed(digits)}%`;
}

function formatMs(value, digits = 1) {
    const numeric = Number(value);
    if (!Number.isFinite(numeric)) {
        return "No data";
    }
    return `${numeric.toFixed(digits)} ms`;
}

function rectsOverlap(left, right) {
    return !(left.right <= right.left
        || left.left >= right.right
        || left.bottom <= right.top
        || left.top >= right.bottom);
}

function nowText(ts) {
    if (!ts) return "Not updated yet";
    const elapsed = Math.max(0, Math.round((Date.now() - ts) / 1000));
    if (elapsed < 2) return "Updated just now";
    return `Updated ${elapsed}s ago`;
}

function throughputRadiusPx(bitsPerSecond, maxBitsPerSecond) {
    if (maxBitsPerSecond <= 0) {
        return 7;
    }
    const ratio = Math.max(0, Math.min(1, bitsPerSecond / maxBitsPerSecond));
    return Math.round(6 + (Math.log10(1 + (ratio * 99)) * 10));
}

// `.67.4` Lever 2: the DISPLAYED marker radius. The traffic scaling (6..26 px)
// is heavily damped toward a small near-uniform dot (6..9 px) so markers read as
// crisp dots, not blobs. Both the layer paint (`dotRadius`) and the size legend
// use this, so the legend cannot lie.
function displayMarkerRadiusPx(rawRadius) {
    const r = Number.isFinite(Number(rawRadius)) ? Number(rawRadius) : 7;
    return Math.round((6 + ((r - 6) * 0.15)) * 100) / 100;
}

function worstQoo(qooDown, qooUp) {
    const values = [qooDown, qooUp].filter((value) => Number.isFinite(value));
    if (!values.length) return null;
    return Math.min(...values);
}

function worstRtt(rttDownMs, rttUpMs) {
    const values = [rttDownMs, rttUpMs].filter((value) => Number.isFinite(value));
    if (!values.length) return null;
    return Math.max(...values);
}

// `.67.3`: the RTT marker/value color comes from `resolveRttThresholds()`
// (the CONFIG source), NOT the `.67.1` three-handle override. The value field
// calls the same, so a site's marker and its own cell cannot disagree.
function metricColorForMode(mode, metricValue, max = null) {
    if (mode === "qoo") {
        return colorByQoqScore(metricValue);
    }
    if (metricValue === null || metricValue === undefined) {
        return "#8893a5";
    }
    const numeric = Number(metricValue);
    if (!Number.isFinite(numeric)) {
        return "#8893a5";
    }
    if (mode === "throughput") {
        return lerpGreenToRedViaOrange(numeric, Math.max(1, Number(max) || 1));
    }
    return colorByRttMs(numeric);
}

// `.67.12`: the native density-heatmap helpers (`defaultHeatRange`,
// `heatWeightExpression`, `heatColorExpression`, `heatColorCssGradient`) were
// RETIRED with the `heatmap` layer — the hexbin uses the metric ramp
// (`colorByRttMs` → `resolveRttThresholds`) + `hexbinBorderAlpha` instead.

// `.67.3`: parse a CSS color (`#rrggbb`, `#rgb`, `rgb()`, `rgba()`) into [r,g,b].
function parseRgb(color) {
    const s = String(color || "").trim();
    if (s.startsWith("#")) {
        const hex = s.slice(1);
        const full = hex.length === 3 ? hex.split("").map((c) => c + c).join("") : hex;
        return [
            parseInt(full.slice(0, 2), 16) || 0,
            parseInt(full.slice(2, 4), 16) || 0,
            parseInt(full.slice(4, 6), 16) || 0,
        ];
    }
    const m = s.match(/rgba?\(([^)]+)\)/i);
    if (m) {
        const parts = m[1].split(",").map((v) => Number(v.trim()));
        return [parts[0] || 0, parts[1] || 0, parts[2] || 0];
    }
    return [136, 147, 165];
}

// `.67.3`: an equirectangular km distance (fine at site/city scale) lives in
// the pure site_map_field helper (shared with its unit test).

function averageOrNull(sum, count) {
    return count > 0 ? (sum / count) : null;
}

function hasMeaningfulLatLon(lat, lon) {
    if (!Number.isFinite(lat) || !Number.isFinite(lon)) {
        return false;
    }
    // Many integrations default to (0,0) when coordinates are unknown.
    if (lat === 0 && lon === 0) {
        return false;
    }
    if (lat < -WEB_MERCATOR_MAX_LAT || lat > WEB_MERCATOR_MAX_LAT) {
        return false;
    }
    if (lon < -180 || lon > 180) {
        return false;
    }
    return true;
}

function stableNodeKey(index, node) {
    return node.id || `${index}:${node.name}`;
}

function asNodeType(node) {
    return String(node?.type || node?.node_type || "").toLowerCase();
}

function immediateParentIndex(node) {
    const raw = node?.immediate_parent ?? node?.immediateParent;
    if (raw === null || raw === undefined) {
        return null;
    }
    const numeric = Number(raw);
    return Number.isFinite(numeric) ? numeric : null;
}

function displayNodeName(name, nodeType) {
    if (!name) {
        return "";
    }
    return isRedacted() && String(nodeType || "").toLowerCase() === "site"
        ? "[redacted]"
        : name;
}

function displayParentName(name, parentType) {
    if (!name) {
        return "";
    }
    return isRedacted() && String(parentType || "").toLowerCase() === "site"
        ? "[redacted]"
        : name;
}

function redactableHtml(text) {
    return `<span class="redactable">${escapeHtml(text ?? "")}</span>`;
}

function treePageHrefForNodeId(nodeId) {
    const normalizedId = String(nodeId || "").trim();
    if (!normalizedId) {
        return null;
    }
    const params = new URLSearchParams();
    params.set("parent", "0");
    params.set("nodeId", normalizedId);
    return `/tree.html?${params.toString()}`;
}

function findNearestAncestorSiteIndex(indexMap, startIndex) {
    let currentIndex = startIndex;
    const visited = new Set();
    while (currentIndex !== null && currentIndex !== undefined) {
        const numeric = Number(currentIndex);
        if (!Number.isFinite(numeric)) {
            return null;
        }
        if (visited.has(numeric)) {
            return null;
        }
        visited.add(numeric);
        const node = indexMap.get(numeric);
        if (!node) {
            return null;
        }
        if (asNodeType(node) === "site") {
            return numeric;
        }
        currentIndex = immediateParentIndex(node);
    }
    return null;
}

function markerPalette() {
    // `.67.4` Lever 2: the site ring is DARK (match the mockup), not white — a
    // thin dark outline around a small bright dot. AP/fanout keep their ring in
    // the non-field modes (in VALUE-field mode the AP ring is zeroed).
    if (isDarkMode()) {
        return {
            siteStroke: "rgba(2, 6, 23, 0.72)",
            apStroke: "rgba(244, 248, 255, 0.54)",
            link: "rgba(15, 23, 42, 0.35)",
        };
    }
    return {
        siteStroke: "rgba(15, 23, 42, 0.58)",
        apStroke: "rgba(15, 23, 42, 0.28)",
        link: "rgba(15, 23, 42, 0.35)",
    };
}

function linkColorExpression() {
    const stops = isDarkMode()
        ? [
            0.0, "rgba(76, 164, 255, 0.40)",
            0.55, "rgba(245, 184, 79, 0.52)",
            1.0, "rgba(217, 75, 91, 0.62)",
        ]
        : [
            0.0, "rgba(76, 164, 255, 0.32)",
            0.55, "rgba(245, 184, 79, 0.44)",
            1.0, "rgba(217, 75, 91, 0.54)",
        ];
    return [
        "interpolate",
        ["linear"],
        ["coalesce", ["get", "utilizationRatio"], 0],
        ...stops,
    ];
}

function linkWidthExpression() {
    const ratio = ["min", 1, ["max", 0, ["coalesce", ["get", "trafficRatio"], 0]]];
    const trafficFactor = ["interpolate", ["exponential", 1.6], ratio, 0, 1.0, 1, 2.0];
    // MapLibre only permits zoom expressions at the top level of a step/interpolate. Keep it there.
    const w2 = ["*", 1.8, trafficFactor];
    const w6 = ["*", 3.2, trafficFactor];
    const w10 = ["*", 4.6, trafficFactor];
    return ["interpolate", ["linear"], ["zoom"], 2, w2, 6, w6, 10, w10];
}

function rasterThemePaint() {
    if (isDarkMode()) {
        return {
            "raster-opacity": 0.9,
            "raster-brightness-min": 0.02,
            "raster-brightness-max": 0.24,
            "raster-contrast": 0.38,
            "raster-saturation": -0.5,
            "raster-hue-rotate": 208,
        };
    }
    return {
        "raster-opacity": 1,
        "raster-brightness-min": 0,
        "raster-brightness-max": 1,
        "raster-contrast": 0,
        "raster-saturation": 0,
        "raster-hue-rotate": 0,
    };
}

function buildOsmRasterStyle() {
    const rasterPaint = rasterThemePaint();
    return {
        version: 8,
        sources: {
            [OSM_RASTER_SOURCE_ID]: {
                type: "raster",
                tiles: [TILE_URL_TEMPLATE],
                tileSize: 256,
                attribution: TILE_ATTRIBUTION,
                maxzoom: TILE_MAX_ZOOM,
            },
        },
        layers: [
            {
                id: "site-map-background",
                type: "background",
                paint: {
                    "background-color": isDarkMode() ? "#0b1220" : "#ffffff",
                },
            },
            {
                id: OSM_RASTER_LAYER_ID,
                type: "raster",
                source: OSM_RASTER_SOURCE_ID,
                minzoom: 0,
                maxzoom: TILE_MAX_ZOOM,
                paint: rasterPaint,
            },
        ],
    };
}

function sleepMs(ms) {
    return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function installInsightTileProtocolOnce() {
    if (!window.maplibregl?.addProtocol) {
        return;
    }
    if (window.__lqosInsightTileProtocolInstalled) {
        return;
    }
    window.__lqosInsightTileProtocolInstalled = true;

    window.maplibregl.addProtocol(INSIGHT_TILE_PROTOCOL, (params, abortControllerOrCallback) => {
        const url = String(params?.url ?? "").replace(`${INSIGHT_TILE_PROTOCOL}://`, "https://");
        const responseType = String(params?.type || "arrayBuffer");
        const method = params?.method || "GET";
        const body = params?.body;

        const makeHeaders = () => {
            try {
                return new Headers(params?.headers ?? {});
            } catch (_) {
                return new Headers();
            }
        };

        const fetchWithRetries = async (signal) => {
            const deadlineMs = Date.now() + (5 * 60_000);
            let consecutive429 = 0;
            while (Date.now() < deadlineMs) {
                if (signal?.aborted) {
                    throw new Error("AbortError");
                }
                const headers = makeHeaders();
                if (responseType === "json" && !headers.has("Accept")) {
                    headers.set("Accept", "application/json");
                }
                const resp = await withInsightTileFetchSlot(() => fetch(url, {
                    method,
                    body,
                    headers,
                    credentials: "omit",
                    cache: params?.cache,
                    signal,
                }));

                if (resp.status === 429) {
                    consecutive429 += 1;
                    const retryAfterMs = parseRetryAfterMs(resp.headers.get("retry-after"));
                    const fallbackDelayMs = 10_000 * Math.pow(2, Math.min(consecutive429 - 1, 4));
                    const baseDelayMs = Math.max(retryAfterMs ?? 0, fallbackDelayMs);
                    const delayMs = Math.min(Math.max(baseDelayMs, 500), 180_000) + Math.round(Math.random() * 1000);
                    insightTileRateLimitUntil = Math.max(insightTileRateLimitUntil, Date.now() + delayMs);
                    await sleepMs(Math.max(0, insightTileRateLimitUntil - Date.now()));
                    if (signal?.aborted) {
                        throw new Error("AbortError");
                    }
                    continue;
                }

                if (resp.status === 503) {
                    consecutive429 = 0;
                    const retryAfterMs = parseRetryAfterMs(resp.headers.get("retry-after"));
                    const baseDelayMs = retryAfterMs ?? 1000;
                    const delayMs = Math.min(Math.max(baseDelayMs, 250), 15_000) + Math.round(Math.random() * 250);
                    await sleepMs(delayMs);
                    if (signal?.aborted) {
                        throw new Error("AbortError");
                    }
                    continue;
                }

                consecutive429 = 0;

                if (!resp.ok) {
                    throw new Error(`tile request failed: ${resp.status}`);
                }

                let data;
                if (responseType === "json") {
                    data = await resp.json();
                } else if (responseType === "text") {
                    data = await resp.text();
                } else {
                    data = await resp.arrayBuffer();
                }

                if (signal?.aborted) {
                    throw new Error("AbortError");
                }

                return {
                    data,
                    cacheControl: resp.headers.get("cache-control") ?? undefined,
                    expires: resp.headers.get("expires") ?? undefined,
                };
            }
            throw new Error("tile request retries exhausted");
        };

        // MapLibre's protocol handler signature differs by version:
        // - newer: (params, abortController) => Promise<{data, cacheControl, expires}>
        // - older: (params, callback) => { cancel() }
        if (typeof abortControllerOrCallback === "function") {
            const callback = abortControllerOrCallback;
            const controller = new AbortController();
            fetchWithRetries(controller.signal)
                .then((result) => callback(null, result.data, result.cacheControl, result.expires))
                .catch((err) => {
                    if (controller.signal.aborted) return;
                    callback(err);
                });
            return { cancel: () => controller.abort() };
        }

        const signal = abortControllerOrCallback?.signal;
        return fetchWithRetries(signal);
    });
}

function normalizeBboxResponse(data) {
    // Newer/expected shape (see osm_cache): { center: { lat, lon }, zoom }
    if (data && typeof data === "object" && data.center && typeof data.center === "object") {
        const lat = Number(data.center.lat ?? data.center.latitude);
        const lon = Number(data.center.lon ?? data.center.lng ?? data.center.longitude);
        const zoom = Number(data.zoom);
        return { lat, lon, zoom };
    }
    if (Array.isArray(data) && data.length >= 3) {
        const [lat, lon, zoom] = data;
        return { lat: Number(lat), lon: Number(lon), zoom: Number(zoom) };
    }
    if (data && typeof data === "object") {
        const lat = Number(data.lat ?? data.latitude);
        const lon = Number(data.lon ?? data.lng ?? data.longitude);
        const zoom = Number(data.zoom);
        return { lat, lon, zoom };
    }
    return null;
}

async function requestOsmCenterFromBbox(siteLatLonPairs, timeoutMs = 2500) {
    if (!Array.isArray(siteLatLonPairs) || siteLatLonPairs.length === 0) {
        throw new Error("No site coordinates supplied");
    }

    const points = siteLatLonPairs
        .map((pair) => {
            if (!Array.isArray(pair) || pair.length < 2) {
                return null;
            }
            const [lat, lon] = pair;
            const latN = Number(lat);
            const lonN = Number(lon);
            if (!hasMeaningfulLatLon(latN, lonN)) {
                return null;
            }
            return { lat: latN, lon: lonN };
        })
        .filter((item) => item !== null);

    if (points.length === 0) {
        throw new Error("No valid site coordinates supplied");
    }

    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), timeoutMs);

    try {
        const resp = await fetch(TILE_BBOX_URL, {
            method: "POST",
            headers: {
                "Content-Type": "application/json",
                Authorization: `Bearer ${OSM_CACHE_KEY}`,
            },
            body: JSON.stringify(points),
            signal: controller.signal,
        });
        if (!resp.ok) {
            throw new Error(`bbox request failed: ${resp.status}`);
        }
        const json = await resp.json();
        const normalized = normalizeBboxResponse(json);
        if (!normalized
            || !Number.isFinite(normalized.lat)
            || !Number.isFinite(normalized.lon)
            || !Number.isFinite(normalized.zoom)) {
            throw new Error("bbox returned invalid center");
        }
        return normalized;
    } finally {
        window.clearTimeout(timeoutId);
    }
}

class SiteMapPage {
    constructor() {
        // `.67`: the Site Map opens as a LIVE RTT heatmap by default.
        this.mode = "rtt";
        this.renderMode = "heatmap";
        // `.67.12`: the Heatmap is the HEXBIN (per-cell color = RTT, size =
        // throughput). The settings panel is gone (`.67.8`); the native density
        // `heatmap` layer and the value field are retired, so the former
        // Radius/Intensity/heatMaxThroughput fields are gone too.
        this.fieldMode = "density";
        this.fieldFalloffKm = 25;
        // `.67.8`: glow radius ∝ live throughput (replaces the `.67.5` circuit
        // count); always on (no toggle).
        this.glowByThroughput = true;
        this.fieldOpacity = 0.85;
        // `.67.10`: ONE persistent canvas backs the `canvas` source (no data URL).
        this.fieldCanvas = null;
        this.filters = {
            siteType: "all",
            showMarkers: true,
            showLabels: true,
            onlyIssues: false,
            cluster: false,
        };
        this.history = [];
        this.latestSnapshot = null;
        this.subscription = null;
        this.map = null;
        this.popup = null;
        this.selectedFeature = null;
        this.hasFitOnce = false;
        this.lastUpdateAt = 0;
        this.latestRender = null;
        this.unmappedOpen = false;
        this.mapInitPromise = null;
        this.mapBootstrapped = false;
        this.attachedInheritedApsBySite = new Map();
        this.fanoutSiteKey = null;
        this.lastBboxAttemptAt = 0;
        this.fixtureMode = FIXTURE_MODE;
        this.siteLabelMarkers = new Map();
        this.clusterCountMarkers = new Map();
        this.pendingTextOverlayFrame = null;

        this.canvas = document.getElementById("siteMapCanvas");
        this.statusChip = document.getElementById("siteMapStatusChip");
        this.updatedChip = document.getElementById("siteMapUpdatedChip");
        this.unmappedBadge = document.getElementById("siteMapUnmappedBadge");
        this.unmappedPanel = document.getElementById("siteMapUnmappedPanel");
        this.unmappedSummary = document.getElementById("siteMapUnmappedSummary");
        this.unmappedList = document.getElementById("siteMapUnmappedList");
        this.detailsPanel = document.getElementById("siteMapDetails");
        this.detailsTitle = document.getElementById("siteMapDetailsTitle");
        this.detailsSubtitle = document.getElementById("siteMapDetailsSubtitle");
        this.attachedList = document.getElementById("siteMapAttachedList");
        this.detailsGrid = document.getElementById("siteMapDetailsGrid");
        this.legendGradient = document.getElementById("siteMapLegendGradient");
        this.legendLow = document.getElementById("siteMapLegendLow");
        this.legendHigh = document.getElementById("siteMapLegendHigh");
        this.legendTitle = document.getElementById("siteMapLegendTitle");
        this.legendCaveat = document.getElementById("siteMapLegendCaveat");
        this.legendStops = document.getElementById("siteMapLegendStops");
        this.legendScale = document.getElementById("siteMapLegendScale");
        this.fieldCaveat = document.getElementById("siteMapFieldCaveat");
    }

    init() {
        this.bindControls();
        this.refreshLegend();
        if (this.fixtureMode) {
            this.processTreeMessage({ data: FIXTURE_NETWORK_TREE }, { fixture: true });
        } else {
            this.requestInitialTree();
        }
        this.startUpdatedClock();
        this.observeThemeChanges();
    }

    bindControls() {
        const toggleGroup = (buttons, active) => {
            buttons.forEach(([id, value]) => {
                const el = document.getElementById(id);
                el?.classList.toggle("btn-primary", value === active);
                el?.classList.toggle("btn-outline-secondary", value !== active);
            });
        };
        // `.67.16`: the metric selector is GONE — the metric is FIXED to RTT
        // (`this.mode` defaults to "rtt").
        // `.67`: Heatmap | Points | Connections (Heatmap default).
        const setRenderMode = (mode) => {
            this.renderMode = mode;
            toggleGroup([
                ["siteMapRenderHeatmap", "heatmap"],
                ["siteMapRenderPoints", "points"],
                ["siteMapRenderConnections", "connections"],
            ], mode);
            this.refreshLegend();
            this.applyRenderMode();
        };
        document.getElementById("siteMapRenderHeatmap")?.addEventListener("click", () => setRenderMode("heatmap"));
        document.getElementById("siteMapRenderPoints")?.addEventListener("click", () => setRenderMode("points"));
        document.getElementById("siteMapRenderConnections")?.addEventListener("click", () => setRenderMode("connections"));
        toggleGroup([
            ["siteMapRenderHeatmap", "heatmap"],
            ["siteMapRenderPoints", "points"],
            ["siteMapRenderConnections", "connections"],
        ], this.renderMode);
        document.getElementById("siteMapUnmappedToggle")?.addEventListener("click", () => {
            this.unmappedOpen = !this.unmappedOpen;
            this.syncUnmappedPanel();
        });
        document.getElementById("siteMapUnmappedClose")?.addEventListener("click", () => {
            this.unmappedOpen = false;
            this.syncUnmappedPanel();
        });
        document.getElementById("siteMapDetailsClose")?.addEventListener("click", () => {
            this.selectedFeature = null;
            this.fanoutSiteKey = null;
            this.syncFanoutOverlays();
            this.scheduleTextOverlaySync();
            this.renderDetails(null);
        });
        window.addEventListener("colorBlindModeChanged", () => {
            this.refreshLegend();
            this.renderFromHistory();
        });
        window.addEventListener("redact-change", () => {
            this.popup?.remove();
            this.renderFromHistory();
        });
    }

    initMap(center, zoom) {
        if (!Array.isArray(center)
            || center.length < 2
            || !Number.isFinite(center[0])
            || !Number.isFinite(center[1])
            || !Number.isFinite(zoom)) {
            throw new Error("Invalid map center/zoom");
        }

        installInsightTileProtocolOnce();
        if (typeof window.maplibregl?.setMaxParallelImageRequests === "function") {
            window.maplibregl.setMaxParallelImageRequests(INSIGHT_TILE_MAX_PARALLEL_FETCHES);
        }
        this.map = new window.maplibregl.Map({
            container: this.canvas,
            style: buildOsmRasterStyle(),
            center,
            zoom,
            maxZoom: TILE_MAX_ZOOM,
            attributionControl: false,
            customAttribution: "LibreQoS Insight tile cache",
        });
        this.map.addControl(new window.maplibregl.NavigationControl({ visualizePitch: false }), "bottom-left");
        this.map.addControl(new window.maplibregl.AttributionControl({ compact: true }), "bottom-left");

        this.popup = new window.maplibregl.Popup({
            closeButton: false,
            closeOnMove: false,
            closeOnClick: false,
            maxWidth: "320px",
            className: "site-map-popup",
        });

        const bootstrapOverlays = () => {
            if (!this.map) {
                return;
            }
            // `.67.9`: adding sources/layers before the style is loaded throws
            // "Style is not done loading" and (via the catch below) skips ALL
            // layers. `load`/`styledata` re-invoke this once it is ready.
            if (!this.map.isStyleLoaded()) {
                return;
            }
            try {
                this.installSourcesAndLayers();
            } catch (err) {
                // `.67.9` R2: do NOT silently swallow — a throw here skips ALL
                // MapLibre layers (the DOM labels persist) => "labels only, no
                // field". Log it so it is visible, then bail.
                console.error("[site-map] installSourcesAndLayers failed; MapLibre layers skipped", err);
                return;
            }
            if (!this.map.getSource(SITE_SOURCE_ID) || !this.map.getLayer(SITE_POINTS_LAYER_ID)) {
                return;
            }
            if (!this.mapBootstrapped) {
                this.installInteractions();
                this.mapBootstrapped = true;
            }
            this.applyTheme();
            this.applyRenderMode();
            this.renderFromHistory();
        };

        // `load` can be delayed while raster tiles are still coming in; `styledata` fires earlier.
        this.map.on("load", bootstrapOverlays);
        this.map.on("styledata", bootstrapOverlays);
        bootstrapOverlays();
    }

    observeThemeChanges() {
        const observer = new MutationObserver(() => {
            this.applyTheme();
            this.renderFromHistory();
        });
        observer.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-bs-theme"],
        });
    }

    applyTheme() {
        if (!this.map || !this.map.isStyleLoaded()) {
            return;
        }
        const palette = markerPalette();
        this.map.setPaintProperty("site-map-background", "background-color", isDarkMode() ? "#0b1220" : "#ffffff");
        if (this.map.getLayer(OSM_RASTER_LAYER_ID)) {
            const rasterPaint = rasterThemePaint();
            Object.entries(rasterPaint).forEach(([key, value]) => {
                this.map.setPaintProperty(OSM_RASTER_LAYER_ID, key, value);
            });
        }
        if (this.map.getLayer(SITE_LINK_LAYER_ID)) {
            this.map.setPaintProperty(SITE_LINK_LAYER_ID, "line-color", linkColorExpression());
        }
        if (this.map.getLayer(SITE_POINTS_LAYER_ID)) {
            this.map.setPaintProperty(SITE_POINTS_LAYER_ID, "circle-stroke-color", palette.siteStroke);
        }
        if (this.map.getLayer(AP_POINTS_LAYER_ID)) {
            this.map.setPaintProperty(AP_POINTS_LAYER_ID, "circle-stroke-color", palette.apStroke);
        }
        if (this.map.getLayer(SITE_PLAIN_POINTS_LAYER_ID)) {
            this.map.setPaintProperty(SITE_PLAIN_POINTS_LAYER_ID, "circle-stroke-color", palette.siteStroke);
        }
        if (this.map.getLayer(SITE_HEX_LAYER_ID)) {
            this.map.setPaintProperty(SITE_HEX_LAYER_ID, "fill-color", ["get", "metricColor"]);
        }
    }

    // `.67.12`: the retired `heatRange()` / `applyHeatPaint()` (the native heatmap
    // weight/color/radius) are gone with the `heatmap` layer.

    // `.67` / `.67.1`: Heatmap | Points | Connections × the Data Filters. The heat
    // field is on only in Heatmap mode; the topology lines only in Connections;
    // the markers + labels stay on top in every mode.
    applyRenderMode() {
        if (!this.map) {
            return;
        }
        const show = (layerId, visible) => {
            if (this.map.getLayer(layerId)) {
                this.map.setLayoutProperty(layerId, "visibility", visible ? "visible" : "none");
            }
        };
        const heat = this.renderMode === "heatmap";
        const links = this.renderMode === "connections";
        const markers = this.filters.showMarkers;
        // `.67.7`: in Heatmap mode the marker DOTS are hidden on top of the field;
        // the labels are a separate DOM overlay (syncSiteLabelMarkers, gated by
        // showLabels) and persist. The "Show site markers" checkbox only governs
        // the NON-heatmap modes, and the selection-gated fanout overlay hides too
        // (consistent).
        const showSites = !heat && markers && this.filters.siteType !== "aps";
        const showAps = !heat && markers && this.filters.siteType !== "sites";
        const valueField = heat && this.fieldMode === "value";
        // `.67.12`: Heatmap mode shows the HEXBIN fill (replacing the density heat).
        show(SITE_FIELD_LAYER_ID, valueField);
        show(SITE_HEX_LAYER_ID, heat);
        show(SITE_POINTS_LAYER_ID, showSites && this.filters.cluster);
        show(SITE_CLUSTER_LAYER_ID, showSites && this.filters.cluster);
        show(SITE_PLAIN_POINTS_LAYER_ID, showSites && !this.filters.cluster);
        show(AP_POINTS_LAYER_ID, showAps);
        show(FANOUT_POINT_LAYER_ID, showAps);
        show(SITE_LINK_LAYER_ID, links);
        show(FANOUT_LINE_LAYER_ID, links);
        this.applyMarkerStyles();
        this.scheduleTextOverlaySync();
    }

    // `.67.4`: marker styling per render/field mode.
    //   Lever 1 (R1) — in Heatmap VALUE-field mode the ambient AP markers become
    //     tiny RING-LESS dots via a PAINT change (constant radius + stroke-width
    //     0). The AP source is untouched, so `fieldPoints()` still sees every AP;
    //     Z = AP_DECLUTTER_MIN_ZOOM (7) below which the dots are faded out — the
    //     field feed is unchanged (it reads `latestRender`, not the layer). The
    //     selection-gated FANOUT_POINT overlay is deliberately left as-is.
    //   Lever 2 — sites are small, near-uniform, dark-ringed, opaque dots.
    applyMarkerStyles() {
        if (!this.map) {
            return;
        }
        const set = (layerId, prop, value) => {
            if (this.map.getLayer(layerId)) {
                this.map.setPaintProperty(layerId, prop, value);
            }
        };
        const valueField = this.renderMode === "heatmap" && this.fieldMode === "value";
        const siteRing = markerPalette().siteStroke;

        [SITE_POINTS_LAYER_ID, SITE_PLAIN_POINTS_LAYER_ID].forEach((layerId) => {
            set(layerId, "circle-radius", ["get", "dotRadius"]);
            set(layerId, "circle-opacity", SITE_DOT_OPACITY);
            set(layerId, "circle-stroke-color", siteRing);
            set(layerId, "circle-stroke-width", SITE_RING_WIDTH);
            set(layerId, "circle-blur", 0);
        });
        set(SITE_CLUSTER_LAYER_ID, "circle-stroke-color", siteRing);

        if (valueField) {
            set(AP_POINTS_LAYER_ID, "circle-radius", AP_DECLUTTER_DOT_RADIUS);
            set(AP_POINTS_LAYER_ID, "circle-opacity", [
                "interpolate", ["linear"], ["zoom"],
                AP_DECLUTTER_MIN_ZOOM - 1, 0,
                AP_DECLUTTER_MIN_ZOOM, AP_DECLUTTER_DOT_OPACITY,
                12, Math.min(1, AP_DECLUTTER_DOT_OPACITY + 0.2),
            ]);
            set(AP_POINTS_LAYER_ID, "circle-stroke-width", 0);
            set(AP_POINTS_LAYER_ID, "circle-blur", 0);
        } else {
            const sparse = (this.latestRender?.apFeatures?.length || 0) < 24;
            set(AP_POINTS_LAYER_ID, "circle-radius", ["get", "markerRadius"]);
            set(AP_POINTS_LAYER_ID, "circle-opacity", sparse
                ? ["interpolate", ["linear"], ["zoom"], 0, 0.02, 6, 0.22, 8, 0.78, 10, 0.92]
                : ["interpolate", ["linear"], ["zoom"], 0, 0.0, 6, 0.18, 8, 0.82, 10, 0.96]);
            set(AP_POINTS_LAYER_ID, "circle-stroke-width", 1.0);
            set(AP_POINTS_LAYER_ID, "circle-blur", 0.05);
        }
    }

    installSourcesAndLayers() {
        if (!this.map.getSource(SITE_LINK_SOURCE_ID)) {
            this.map.addSource(SITE_LINK_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
            });
        }
        if (!this.map.getSource(SITE_SOURCE_ID)) {
            this.map.addSource(SITE_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
                cluster: true,
                clusterRadius: 50,
                clusterMaxZoom: 12,
            });
        }
        // `.67.1` (ii): the PARALLEL unclustered site source so the Cluster
        // toggle can show individual points at low zoom (the clustered source
        // emits only cluster features there).
        if (!this.map.getSource(SITE_PLAIN_SOURCE_ID)) {
            this.map.addSource(SITE_PLAIN_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
            });
        }
        if (!this.map.getSource(AP_SOURCE_ID)) {
            this.map.addSource(AP_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
            });
        }
        if (!this.map.getSource(FANOUT_LINE_SOURCE_ID)) {
            this.map.addSource(FANOUT_LINE_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
            });
        }
        if (!this.map.getSource(FANOUT_POINT_SOURCE_ID)) {
            this.map.addSource(FANOUT_POINT_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
            });
        }
        // `.67.12`: the hexbin GeoJSON (scaled hexagon polygons).
        if (!this.map.getSource(SITE_HEX_SOURCE_ID)) {
            this.map.addSource(SITE_HEX_SOURCE_ID, {
                type: "geojson",
                data: { type: "FeatureCollection", features: [] },
            });
        }

        if (!this.map.getLayer(SITE_LINK_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_LINK_LAYER_ID,
                type: "line",
                source: SITE_LINK_SOURCE_ID,
                layout: {
                    "line-join": "round",
                    "line-cap": "round",
                },
                paint: {
                    "line-color": linkColorExpression(),
                    "line-width": linkWidthExpression(),
                    "line-opacity": [
                        "interpolate", ["linear"], ["zoom"],
                        2, 0.35,
                        6, 0.55,
                        10, 0.72,
                    ],
                },
            });
        }

        if (!this.map.getLayer(SITE_POINTS_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_POINTS_LAYER_ID,
                type: "circle",
                source: SITE_SOURCE_ID,
                filter: ["!", ["has", "point_count"]],
                paint: {
                    "circle-color": ["get", "metricColor"],
                    // `.67.4` Lever 2: small, near-uniform, dark-ringed dots.
                    "circle-radius": ["get", "dotRadius"],
                    "circle-opacity": SITE_DOT_OPACITY,
                    "circle-stroke-color": markerPalette().siteStroke,
                    "circle-stroke-width": SITE_RING_WIDTH,
                    "circle-blur": 0,
                },
            });
        }

        // `.67.12`: the HEXBIN fill, inserted via `beforeId` = the site POINTS
        // layer so it sits UNDER the markers (and above the basemap). It REPLACES
        // the native density heatmap (which had only ONE per-point weight, so
        // blob size and color were the same channel).
        if (!this.map.getLayer(SITE_HEX_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_HEX_LAYER_ID,
                type: "fill",
                source: SITE_HEX_SOURCE_ID,
                paint: {
                    "fill-color": ["get", "metricColor"],
                    // `.67.13`: per-cell opacity (neutral cells are faint).
                    "fill-opacity": ["get", "opacity"],
                },
            }, SITE_POINTS_LAYER_ID);
        }
        // `.67.13` R1: the THROUGHPUT GLOW/BLEED — a SEPARATE `line` layer. A
        // `fill-outline-color` is a FIXED hairline (no data-driven width/blur), so
        // it cannot glow; the in-repo `hexbinBorderAlpha` (0.3..0.9) drives BOTH
        // the line width and opacity, and `line-blur` softens it into a bleed.
        if (!this.map.getLayer(SITE_HEX_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_HEX_LAYER_ID,
                type: "fill",
                source: SITE_HEX_SOURCE_ID,
                paint: {
                    // `.67.14`: the per-site glow halo (fill only; the radius now
                    // carries throughput, so the `.67.13` border glow was dropped).
                    "fill-color": ["get", "metricColor"],
                    "fill-opacity": ["get", "opacity"],
                },
            }, SITE_POINTS_LAYER_ID);
        }

        // `.67.1` (ii): the parallel UNCLUSTERED site points layer (same paint as
        // the clustered site points). Shown only when the Cluster toggle is OFF.
        if (!this.map.getLayer(SITE_PLAIN_POINTS_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_PLAIN_POINTS_LAYER_ID,
                type: "circle",
                source: SITE_PLAIN_SOURCE_ID,
                layout: { visibility: "none" },
                paint: {
                    "circle-color": ["get", "metricColor"],
                    // `.67.4` Lever 2: small, near-uniform, dark-ringed dots.
                    "circle-radius": ["get", "dotRadius"],
                    "circle-opacity": SITE_DOT_OPACITY,
                    "circle-stroke-color": markerPalette().siteStroke,
                    "circle-stroke-width": SITE_RING_WIDTH,
                    "circle-blur": 0,
                },
            });
        }

        if (!this.map.getLayer(SITE_CLUSTER_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_CLUSTER_LAYER_ID,
                type: "circle",
                source: SITE_SOURCE_ID,
                filter: ["has", "point_count"],
                paint: {
                    "circle-color": [
                        "step",
                        ["get", "point_count"],
                        "#4ca4ff",
                        8, "#f5b84f",
                        20, "#d94b5b",
                    ],
                    "circle-radius": [
                        "step",
                        ["get", "point_count"],
                        18,
                        8, 24,
                        20, 30,
                    ],
                    "circle-opacity": 0.9,
                    "circle-stroke-color": markerPalette().siteStroke,
                    "circle-stroke-width": 1.3,
                },
            });
        }

        if (!this.map.getLayer(AP_POINTS_LAYER_ID)) {
            this.map.addLayer({
                id: AP_POINTS_LAYER_ID,
                type: "circle",
                source: AP_SOURCE_ID,
                paint: {
                    "circle-color": ["get", "metricColor"],
                    "circle-radius": ["get", "markerRadius"],
                    "circle-opacity": [
                        "interpolate", ["linear"], ["zoom"],
                        0, 0.0,
                        5, 0.08,
                        7, 0.55,
                        9, 0.86,
                    ],
                    "circle-stroke-color": markerPalette().apStroke,
                    "circle-stroke-width": 1.0,
                    "circle-blur": 0.05,
                },
            });
        }

        if (!this.map.getLayer(FANOUT_LINE_LAYER_ID)) {
            this.map.addLayer({
                id: FANOUT_LINE_LAYER_ID,
                type: "line",
                source: FANOUT_LINE_SOURCE_ID,
                layout: {
                    "line-cap": "round",
                    "line-join": "round",
                },
                paint: {
                    "line-color": isDarkMode() ? "rgba(248, 250, 252, 0.46)" : "rgba(15, 23, 42, 0.34)",
                    "line-width": 1.4,
                    "line-dasharray": [1.2, 1.2],
                    "line-opacity": 0.72,
                },
            });
        }

        if (!this.map.getLayer(FANOUT_POINT_LAYER_ID)) {
            this.map.addLayer({
                id: FANOUT_POINT_LAYER_ID,
                type: "circle",
                source: FANOUT_POINT_SOURCE_ID,
                paint: {
                    "circle-color": ["get", "metricColor"],
                    "circle-radius": ["get", "markerRadius"],
                    "circle-opacity": 0.95,
                    "circle-stroke-color": markerPalette().apStroke,
                    "circle-stroke-width": 1.0,
                },
            });
        }
    }

    installInteractions() {
        const pointLayers = [SITE_POINTS_LAYER_ID, SITE_PLAIN_POINTS_LAYER_ID, AP_POINTS_LAYER_ID, FANOUT_POINT_LAYER_ID];

        pointLayers.forEach((layerId) => {
            this.map.on("mouseenter", layerId, () => {
                this.map.getCanvas().style.cursor = "pointer";
            });
            this.map.on("mouseleave", layerId, () => {
                this.map.getCanvas().style.cursor = "";
                this.popup.remove();
            });
            this.map.on("mousemove", layerId, (event) => {
                const feature = event.features?.[0];
                if (!feature) return;
                this.popup
                    .setLngLat(event.lngLat)
                    .setHTML(this.pointPopupHtml(feature.properties))
                    .addTo(this.map);
            });
            this.map.on("click", layerId, (event) => {
                const feature = event.features?.[0];
                if (!feature) return;
                this.selectFeature(feature.properties);
            });
        });

        [SITE_CLUSTER_LAYER_ID].forEach((layerId) => {
            this.map.on("mouseenter", layerId, () => {
                this.map.getCanvas().style.cursor = "pointer";
            });
            this.map.on("mouseleave", layerId, () => {
                this.map.getCanvas().style.cursor = "";
            });
            this.map.on("click", layerId, async (event) => {
                const feature = event.features?.[0];
                const clusterId = feature?.properties?.cluster_id;
                if (!Number.isFinite(clusterId)) {
                    return;
                }
                const source = this.map.getSource(SITE_SOURCE_ID);
                if (!source?.getClusterExpansionZoom) {
                    return;
                }
                const expansionZoom = await source.getClusterExpansionZoom(clusterId);
                this.map.easeTo({
                    center: feature.geometry.coordinates,
                    zoom: expansionZoom,
                    duration: 350,
                });
            });
        });

        this.map.on("zoomend", () => {
            if (this.fanoutSiteKey) {
                this.fanoutSiteKey = null;
                if (this.selectedFeature) {
                    this.renderDetails(this.selectedFeature);
                }
            }
            this.syncFanoutOverlays();
            this.scheduleTextOverlaySync();
        });
        this.map.on("moveend", () => this.scheduleTextOverlaySync());
    }

    requestInitialTree() {
        this.setStatus("Waiting for data", "spinner");
        if (!this.subscription) {
            this.subscription = subscribeWS(["NetworkTreeLite"], (liveMsg) => {
                if (liveMsg.event === "NetworkTreeLite") {
                    this.processTreeMessage(liveMsg);
                }
            });
        }
        listenOnceWithTimeout("NetworkTreeLite", INITIAL_REQUEST_TIMEOUT_MS, (msg) => {
            this.processTreeMessage(msg);
        }, () => {
            this.setStatus("No data received yet", "warning");
        });
        wsClient.send({ NetworkTreeLite: {} });
    }

    processTreeMessage(msg, options = {}) {
        const fixture = options?.fixture === true;
        const data = Array.isArray(msg?.data) ? msg.data : [];
        this.history.push({ timestamp: Date.now(), data });
        const cutoff = Date.now() - HISTORY_WINDOW_MS;
        this.history = this.history.filter((entry) => entry.timestamp >= cutoff);
        this.latestSnapshot = data;
        this.lastUpdateAt = Date.now();
        this.setStatus(fixture ? "Fixture" : "Live", "success");
        this.ensureMapInitialized();
        this.renderFromHistory();
    }

    ensureMapInitialized() {
        if (this.map || this.mapInitPromise) {
            return;
        }
        const now = Date.now();
        if (this.lastBboxAttemptAt && (now - this.lastBboxAttemptAt) < 15_000) {
            return;
        }
        this.lastBboxAttemptAt = now;
        const siteLatLonPairs = this.latestSnapshot
            .filter((entry) => Array.isArray(entry) && entry.length >= 2)
            .map(([, node]) => node)
            .filter((node) => asNodeType(node) === "site")
            .map((node) => {
                const lat = Number(node.latitude);
                const lon = Number(node.longitude);
                return hasMeaningfulLatLon(lat, lon) ? [lat, lon] : null;
            })
            .filter((pair) => Array.isArray(pair));

        if (siteLatLonPairs.length === 0) {
            this.setStatus("No mapped sites", "warning");
            return;
        }

        if (this.fixtureMode) {
            const [lat, lon] = siteLatLonPairs[0];
            this.initMap([lon, lat], 9);
            return;
        }

        this.mapInitPromise = (async () => {
            try {
                if (siteLatLonPairs.length === 1) {
                    const [lat, lon] = siteLatLonPairs[0];
                    siteLatLonPairs.push([lat + 0.0001, lon + 0.0001]);
                }
                const center = await requestOsmCenterFromBbox(siteLatLonPairs, 4000);
                this.initMap([center.lon, center.lat], center.zoom);
            } catch (err) {
                // Offline-degradable: the Insight bbox endpoint (and its raster
                // tiles) may be unreachable — e.g. a host whose DNS fails for
                // the Insight domain.  Fall back to the sites' own bounding box
                // so the map + site overlay still initialize; the tiles stay
                // blank (the plain background is unchanged).
                console.error("Site map bbox request failed; using local site bounds.", err);
                const fallback = localBboxCenter(siteLatLonPairs);
                this.setStatus("Map tiles unavailable", "warning");
                this.initMap([fallback.lon, fallback.lat], fallback.zoom);
            } finally {
                this.mapInitPromise = null;
            }
        })();
    }

    buildAggregates() {
        if (!this.history.length) {
            return null;
        }

        const aggregate = new Map();
        let latestIndexMap = new Map();
        const attachedInheritedApsBySite = new Map();

        this.history.forEach((entry) => {
            const snapshotIndexMap = new Map();
            entry.data.forEach(([index, node]) => {
                snapshotIndexMap.set(index, node);
                const nodeType = asNodeType(node);
                if (nodeType !== "site" && nodeType !== "ap") {
                    return;
                }

                const key = stableNodeKey(index, node);
                let target = aggregate.get(key);
                if (!target) {
                    target = {
                        key,
                        latestIndex: index,
                        latestNode: node,
                        throughputDown: 0,
                        throughputUp: 0,
                        throughputSamples: 0,
                        qooDownSum: 0,
                        qooDownCount: 0,
                        qooUpSum: 0,
                        qooUpCount: 0,
                        rttDownSum: 0,
                        rttDownCount: 0,
                        rttUpSum: 0,
                        rttUpCount: 0,
                    };
                    aggregate.set(key, target);
                }

                target.latestIndex = index;
                target.latestNode = node;
                target.throughputDown += toNumber(node.current_throughput?.[0], 0) * 8;
                target.throughputUp += toNumber(node.current_throughput?.[1], 0) * 8;
                target.throughputSamples += 1;

                const qooDown = node.qoo?.[0];
                const qooUp = node.qoo?.[1];
                if (Number.isFinite(qooDown)) {
                    target.qooDownSum += qooDown;
                    target.qooDownCount += 1;
                }
                if (Number.isFinite(qooUp)) {
                    target.qooUpSum += qooUp;
                    target.qooUpCount += 1;
                }

                const rttDown = Number.isFinite(node.rtts?.[0]) ? node.rtts[0] : null;
                const rttUp = Number.isFinite(node.rtts?.[1]) ? node.rtts[1] : null;
                if (Number.isFinite(rttDown)) {
                    target.rttDownSum += rttDown;
                    target.rttDownCount += 1;
                }
                if (Number.isFinite(rttUp)) {
                    target.rttUpSum += rttUp;
                    target.rttUpCount += 1;
                }
            });
            latestIndexMap = snapshotIndexMap;
        });

        const features = [];
        const unmappedSites = [];
        const unmappedAps = [];
        let maxBitsPerSecond = 0;
        let maxSiteBitsPerSecond = 0;
        const byIndex = new Map();

        aggregate.forEach((value) => {
            const node = value.latestNode;
            const nodeType = asNodeType(node);
            const avgDown = value.throughputSamples > 0 ? (value.throughputDown / value.throughputSamples) : 0;
            const avgUp = value.throughputSamples > 0 ? (value.throughputUp / value.throughputSamples) : 0;
            const throughputCombined = avgDown + avgUp;
            maxBitsPerSecond = Math.max(maxBitsPerSecond, throughputCombined);
            if (nodeType === "site") {
                maxSiteBitsPerSecond = Math.max(maxSiteBitsPerSecond, throughputCombined);
            }

            const qooDown = averageOrNull(value.qooDownSum, value.qooDownCount);
            const qooUp = averageOrNull(value.qooUpSum, value.qooUpCount);
            const rttDownMs = averageOrNull(value.rttDownSum, value.rttDownCount);
            const rttUpMs = averageOrNull(value.rttUpSum, value.rttUpCount);
            const maxMbps = effectiveMax(node);
            const limitDownMbps = toNumber(maxMbps?.[0], 0);
            const limitUpMbps = toNumber(maxMbps?.[1], 0);

            const normalized = {
                key: value.key,
                index: value.latestIndex,
                name: node.name,
                id: node.id || null,
                type: nodeType,
                immediateParent: immediateParentIndex(node),
                latitude: null,
                longitude: null,
                throughputDown: avgDown,
                throughputUp: avgUp,
                throughputCombined,
                qooDown,
                qooUp,
                qooWorst: worstQoo(qooDown, qooUp),
                rttDownMs,
                rttUpMs,
                rttWorst: worstRtt(rttDownMs, rttUpMs),
                parentName: null,
                parentType: null,
                inheritedCoords: false,
                limitDownMbps,
                limitUpMbps,
            };
            const lat = Number(node.latitude);
            const lon = Number(node.longitude);
            if (hasMeaningfulLatLon(lat, lon)) {
                normalized.latitude = lat;
                normalized.longitude = lon;
            }
            byIndex.set(value.latestIndex, normalized);
        });

        byIndex.forEach((node) => {
            if (node.type === "ap" && (node.latitude === null || node.longitude === null)) {
                const parent = byIndex.get(node.immediateParent);
                if (parent && parent.type === "site" && parent.latitude !== null && parent.longitude !== null) {
                    node.latitude = parent.latitude;
                    node.longitude = parent.longitude;
                    node.parentName = parent.name;
                    node.parentType = parent.type;
                    node.inheritedCoords = true;
                }
            } else if (node.immediateParent !== null && node.immediateParent !== undefined) {
                const parent = byIndex.get(node.immediateParent);
                if (parent) {
                    node.parentName = parent.name;
                    node.parentType = parent.type;
                }
            }
        });

        byIndex.forEach((node) => {
            if (node.type !== "ap" || !node.inheritedCoords) {
                return;
            }
            const parent = byIndex.get(node.immediateParent);
            if (!parent || parent.type !== "site") {
                return;
            }
            const bucket = attachedInheritedApsBySite.get(parent.key) || [];
            bucket.push(node);
            attachedInheritedApsBySite.set(parent.key, bucket);
        });

        byIndex.forEach((node) => {
            if (node.latitude === null || node.longitude === null) {
                const listTarget = node.type === "site" ? unmappedSites : unmappedAps;
                listTarget.push(node);
                return;
            }

            if (node.type === "ap" && node.inheritedCoords) {
                return;
            }

            const metricValue = this.mode === "qoo"
                ? node.qooWorst
                : this.mode === "throughput" ? node.throughputCombined : node.rttWorst;
            const metricColor = metricColorForMode(this.mode, metricValue, maxBitsPerSecond);
            const feature = {
                type: "Feature",
                geometry: {
                    type: "Point",
                    coordinates: [node.longitude, node.latitude],
                },
                properties: {
                    key: node.key,
                    nodeId: node.id || "",
                    name: node.name,
                    displayName: displayNodeName(node.name, node.type),
                    nodeType: node.type,
                    parentName: node.parentName || "",
                    parentType: node.parentType || "",
                    inheritedCoords: node.inheritedCoords ? 1 : 0,
                    attachedInheritedApCount: attachedInheritedApsBySite.get(node.key)?.length || 0,
                    throughputDown: node.throughputDown,
                    throughputUp: node.throughputUp,
                    throughputCombined: node.throughputCombined,
                    qooDown: node.qooDown,
                    qooUp: node.qooUp,
                    rttDownMs: node.rttDownMs,
                    rttUpMs: node.rttUpMs,
                    markerRadius: throughputRadiusPx(node.throughputCombined, Math.max(maxBitsPerSecond, 1)),
                    dotRadius: displayMarkerRadiusPx(throughputRadiusPx(node.throughputCombined, Math.max(maxBitsPerSecond, 1))),
                    metricColor,
                },
            };
            if (Number.isFinite(node.qooWorst)) {
                feature.properties.qooWorst = node.qooWorst;
            }
            if (Number.isFinite(node.rttWorst)) {
                feature.properties.rttWorst = node.rttWorst;
            }
            features.push(feature);
        });

        const siteLinkFeatures = [];
        const emittedLinks = new Set();
        byIndex.forEach((node) => {
            if (node.type !== "site") {
                return;
            }
            if (node.latitude === null || node.longitude === null) {
                return;
            }
            const parentSiteIndex = findNearestAncestorSiteIndex(latestIndexMap, node.immediateParent);
            if (parentSiteIndex === null) {
                return;
            }
            const parent = byIndex.get(parentSiteIndex);
            if (!parent || parent.type !== "site" || parent.latitude === null || parent.longitude === null) {
                return;
            }
            const key = `${node.key}->${parent.key}`;
            if (emittedLinks.has(key)) {
                return;
            }
            emittedLinks.add(key);
            const downLimitBits = node.limitDownMbps > 0 ? node.limitDownMbps * 1000 * 1000 : 0;
            const upLimitBits = node.limitUpMbps > 0 ? node.limitUpMbps * 1000 * 1000 : 0;
            const downRatio = downLimitBits > 0 ? (node.throughputDown / downLimitBits) : null;
            const upRatio = upLimitBits > 0 ? (node.throughputUp / upLimitBits) : null;
            const utilizationRatio = (downRatio === null && upRatio === null)
                ? null
                : Math.max(0, Math.min(1, Math.max(downRatio ?? 0, upRatio ?? 0)));
            const trafficRatio = maxSiteBitsPerSecond > 0
                ? Math.max(0, Math.min(1, node.throughputCombined / maxSiteBitsPerSecond))
                : 0;
            siteLinkFeatures.push({
                type: "Feature",
                geometry: {
                    type: "LineString",
                    coordinates: [
                        [node.longitude, node.latitude],
                        [parent.longitude, parent.latitude],
                    ],
                },
                properties: {
                    key,
                    fromName: node.name,
                    toName: parent.name,
                    utilizationRatio,
                    trafficRatio,
                },
            });
        });

        const siteFeatures = features.filter((feature) => feature.properties.nodeType === "site");
        const apFeatures = features.filter((feature) => feature.properties.nodeType === "ap");
        return {
            siteFeatures,
            apFeatures,
            siteLinkFeatures,
            unmappedSites,
            unmappedAps,
            attachedInheritedApsBySite,
            maxBitsPerSecond,
            maxSiteBitsPerSecond,
        };
    }

    renderFromHistory() {
        if (!this.map) {
            return;
        }
        // MapLibre may report the style as not-yet-loaded while raster tiles are still fetching
        // (503/429). The overlays only require the GeoJSON sources/layers to exist.
        if (!this.map.getSource(SITE_SOURCE_ID)
            || !this.map.getSource(AP_SOURCE_ID)
            || !this.map.getSource(SITE_LINK_SOURCE_ID)
            || !this.map.getSource(SITE_PLAIN_SOURCE_ID)
            || !this.map.getSource(SITE_HEX_SOURCE_ID)) {
            return;
        }
        const aggregate = this.buildAggregates();
        if (!aggregate) {
            return;
        }
        this.latestRender = aggregate;
        this.attachedInheritedApsBySite = aggregate.attachedInheritedApsBySite || new Map();
        this.updateSources(aggregate);
        this.updateSelection();
        this.renderUnmapped(aggregate.unmappedSites, aggregate.unmappedAps);
        if (!this.hasFitOnce) {
            this.fitToDataIfNeeded(aggregate.siteFeatures, aggregate.apFeatures);
        }
    }

    updateSources(aggregate) {
        // `.67.1`: sources are (re)built from the aggregate + the current filters
        // and threshold range, so a filter/range change re-sources them too.
        this.rebuildSources();
        // `.67.3`: recompute the IDW value field on every snapshot refresh.
        this.rebuildValueField();
        // `.67.12`: recompute the flat 2D hexbin on every snapshot refresh.
        this.rebuildHexbin();

        // `.67.4`: marker styling (site dots + the field-mode AP de-clutter) is
        // owned by `applyMarkerStyles`; the traffic-scaled opacities moved there.
        this.applyMarkerStyles();

        this.syncFanoutOverlays();
        this.scheduleTextOverlaySync();
        this.syncMarkerSizeLegend(aggregate.maxBitsPerSecond);
    }

    // `.67.1`: (re)build the site / AP / plain / heat sources from the latest
    // snapshot + the Data Filters. Mechanisms:
    //   Site-Type  -> the HEAT source (APs => apFeatures) + the marker sources;
    //   onlyIssues -> `rtt >= resolveRttThresholds().redMs` (heat + markers);
    //   Cluster    -> a SEPARATE unclustered source/layer (not layer visibility).
    rebuildSources() {
        if (!this.map || !this.latestRender) {
            return;
        }
        const aggregate = this.latestRender;
        const siteSource = this.map.getSource(SITE_SOURCE_ID);
        const apSource = this.map.getSource(AP_SOURCE_ID);
        const linkSource = this.map.getSource(SITE_LINK_SOURCE_ID);
        const plainSource = this.map.getSource(SITE_PLAIN_SOURCE_ID);
        if (!siteSource || !apSource || !linkSource) {
            return;
        }
        const issueRed = resolveRttThresholds().redMs;
        const includeSites = this.filters.siteType !== "aps";
        const includeAps = this.filters.siteType !== "sites";
        const issueOk = (feature) => !this.filters.onlyIssues
            || Number(feature?.properties?.rttWorst) >= issueRed;
        const siteFeatures = includeSites ? (aggregate.siteFeatures || []).filter(issueOk) : [];
        const apFeatures = includeAps ? (aggregate.apFeatures || []).filter(issueOk) : [];
        siteSource.setData({ type: "FeatureCollection", features: siteFeatures });
        if (plainSource) {
            plainSource.setData({ type: "FeatureCollection", features: siteFeatures });
        }
        apSource.setData({ type: "FeatureCollection", features: apFeatures });
        linkSource.setData({
            type: "FeatureCollection",
            features: includeSites ? (aggregate.siteLinkFeatures ?? []) : [],
        });
        // `.67.12`: the hexbin is rebuilt separately in `rebuildHexbin`.
    }

    // `.67.3`: the value-field points for the CURRENT metric (finite values only),
    // honoring the Site-Type + only-issues filters (same predicate as the heat).
    fieldPoints() {
        if (!this.latestRender) {
            return [];
        }
        const issueRed = resolveRttThresholds().redMs;
        const includeSites = this.filters.siteType !== "aps";
        const includeAps = this.filters.siteType !== "sites";
        const out = [];
        const add = (features) => {
            for (const feature of features || []) {
                const props = feature.properties || {};
                if (this.filters.onlyIssues && !(Number(props.rttWorst) >= issueRed)) {
                    continue;
                }
                const coords = feature.geometry?.coordinates || [];
                const lng = Number(coords[0]);
                const lat = Number(coords[1]);
                if (!Number.isFinite(lng) || !Number.isFinite(lat)) {
                    continue;
                }
                const value = this.metricValueFor(props);
                if (!Number.isFinite(value)) {
                    continue;
                }
                const throughput = Math.max(0, toNumber(props.throughputCombined, 0));
                out.push({ lng, lat, value, throughput });
            }
        };
        if (includeSites) {
            add(this.latestRender.siteFeatures);
        }
        if (includeAps) {
            add(this.latestRender.apFeatures);
        }
        // `.67.8`: per-point influence radius ∝ sqrt(live throughput / tpRef),
        // clamped. tpRef = the max throughput among the plotted points (floor 1);
        // rMin/rMax are fractions of the base falloff. A no-traffic point ⇒ the
        // min clamp keeps it visible. Always on (no toggle); the smoothing
        // length-scale stays global and the metric color is unchanged.
        if (this.glowByThroughput && out.length > 0) {
            const base = Math.max(1, Number(this.fieldFalloffKm) || 25);
            const minKm = Math.max(0.5, base * GLOW_RADIUS_MIN_FRACTION);
            const maxKm = Math.max(minKm + 0.5, base * GLOW_RADIUS_MAX_FRACTION);
            // `reduce` (not a spread) so a huge point set cannot overflow the stack.
            const tpRef = Math.max(1, out.reduce((acc, point) => Math.max(acc, point.throughput), 0));
            for (const point of out) {
                point.radiusKm = glowRadiusKm(point.throughput, tpRef, base, minKm, maxKm);
            }
        }
        return out;
    }

    metricValueFor(props) {
        const raw = this.mode === "qoo"
            ? props.qooWorst
            : this.mode === "throughput" ? props.throughputCombined : props.rttWorst;
        const n = Number(raw);
        return Number.isFinite(n) ? n : NaN;
    }

    #rgbForValue(value) {
        if (this.mode === "qoo") {
            return parseRgb(colorByQoqScore(value));
        }
        if (this.mode === "throughput") {
            // Same scale the MARKERS use (the all-node max), so field <=> marker
            // colors agree at the same place.
            const max = Math.max(1, Number(this.latestRender?.maxBitsPerSecond) || 1);
            return parseRgb(lerpGreenToRedViaOrange(value, max));
        }
        // VALUE mode uses the SAME config thresholds the MARKERS use
        // (`resolveRttThresholds`), never the `.67.1` density handles.
        return parseRgb(colorByRttMs(value));
    }

    // `.67.10`: rebuild the IDW value field into ONE persistent canvas and hand it
    // to a MapLibre `canvas` source — NO `data:` URL, NO fetch, so the node-manager
    // CSP `connect-src` cannot block it. IDW (k-nearest) over the data bbox; alpha
    // fades with distance to the nearest site; color by the metric scale.
    rebuildValueField() {
        // `.67.12`: the Heatmap is the HEXBIN, so the value field is not shown —
        // skip the O(grid x sites) raster + canvas upload entirely (it was wasted
        // work on every snapshot refresh).
        if (!this.map || this.fieldMode !== "value") {
            return;
        }
        const points = this.fieldPoints();
        const fieldSource = this.map.getSource(SITE_FIELD_SOURCE_ID);
        if (points.length === 0) {
            // `.67.9` R3: no plottable points (e.g. no finite RTT yet) — surface an
            // explicit notice, NEVER a silent blank; log the cause. Clear the
            // persistent canvas so no stale field lingers.
            this.setFieldNotice(true);
            console.warn("[site-map] value field empty — no plottable points", {
                mode: this.mode,
                sites: this.latestRender?.siteFeatures?.length || 0,
                aps: this.latestRender?.apFeatures?.length || 0,
            });
            if (this.fieldCanvas) {
                const ctx = this.fieldCanvas.getContext("2d");
                ctx.clearRect(0, 0, this.fieldCanvas.width, this.fieldCanvas.height);
            }
            this.#refreshCanvasSource(fieldSource, null);
            return;
        }
        this.setFieldNotice(false);
        let west = Infinity;
        let east = -Infinity;
        let south = Infinity;
        let north = -Infinity;
        for (const p of points) {
            west = Math.min(west, p.lng);
            east = Math.max(east, p.lng);
            south = Math.min(south, p.lat);
            north = Math.max(north, p.lat);
        }
        const padX = Math.max(1e-4, (east - west) * 0.08);
        const padY = Math.max(1e-4, (north - south) * 0.08);
        const bbox = { west: west - padX, east: east + padX, south: south - padY, north: north + padY };
        const canvas = this.#drawFieldCanvas(points, bbox);
        const coordinates = [
            [bbox.west, bbox.north],
            [bbox.east, bbox.north],
            [bbox.east, bbox.south],
            [bbox.west, bbox.south],
        ];
        if (!fieldSource) {
            // `.67.10`: add the `canvas` source ONCE (no fetch). `animate:false`
            // keeps it static until `#refreshCanvasSource` explicitly updates it.
            this.map.addSource(SITE_FIELD_SOURCE_ID, { type: "canvas", canvas, coordinates, animate: false });
        } else {
            this.#refreshCanvasSource(fieldSource, coordinates);
        }
        // `.67.9` R2: add the field layer INDEPENDENT of the marker layer (use a
        // beforeId only when the marker layer exists, else append).
        if (!this.map.getLayer(SITE_FIELD_LAYER_ID)) {
            const beforeId = this.map.getLayer(SITE_POINTS_LAYER_ID) ? SITE_POINTS_LAYER_ID : undefined;
            const layer = {
                id: SITE_FIELD_LAYER_ID,
                type: "raster",
                source: SITE_FIELD_SOURCE_ID,
                paint: {
                    "raster-opacity": Number(this.fieldOpacity) || 0.85,
                    "raster-fade-duration": 0,
                },
            };
            if (beforeId) {
                this.map.addLayer(layer, beforeId);
            } else {
                this.map.addLayer(layer);
            }
            this.applyRenderMode();
        }
    }

    // `.67.12`: rebuild the flat 2D hexbin (per-cell color = the metric, size =
    // throughput) into the native MapLibre `fill`. Pure JS — no worker/fetch; the
    // same point set the value field uses (filters + metric honored).
    rebuildHexbin() {
        if (!this.map) {
            return;
        }
        const source = this.map.getSource(SITE_HEX_SOURCE_ID);
        if (!source) {
            return;
        }
        const points = this.fieldPoints();
        // `.67.15`: the fill colour = the CALLER's CONFIG scale (`colorByRttMs` via
        // `#rgbForValue`) — the SAME scale the markers use (ONE scale; the `.67.3`
        // agree-invariant restored). The domain is the config, NOT the data.
        const { features } = planarHaloHexbin(points, {
            colorFor: (value) => {
                const [r, g, b] = this.#rgbForValue(value);
                return `rgb(${Math.round(r)}, ${Math.round(g)}, ${Math.round(b)})`;
            },
        });
        source.setData({ type: "FeatureCollection", features });
    }

    // `.67.10`: with `animate:false` the `canvas` source does NOT re-read the
    // canvas on its own, so after a redraw force a one-shot texture upload
    // (`play()` marks it live; `pause()` runs the `prepare()` that uploads, then
    // stops) and an explicit repaint. `triggerRepaint()` alone would leave a
    // STALE texture.
    #refreshCanvasSource(source, coordinates) {
        if (!source) {
            return;
        }
        if (coordinates && typeof source.setCoordinates === "function") {
            source.setCoordinates(coordinates);
        }
        if (typeof source.play === "function" && typeof source.pause === "function") {
            source.play();
            source.pause();
        }
        this.map.triggerRepaint();
    }

    // `.67.9` R3: the value-field notice element (shown when the field is empty).
    setFieldNotice(visible) {
        const el = document.getElementById("siteMapFieldNotice");
        if (el) {
            el.style.display = visible ? "" : "none";
        }
    }

    // `.67.10`: draw the IDW field into the ONE persistent canvas (no `data:` URL).
    // ★ The kernel builds grid ROWS in Mercator-y so the fixed-bbox `canvas` source
    // lines the field up with the markers — a linear-in-latitude canvas would
    // misplace a site's own cell in latitude. Residual error is within one grid
    // cell. Cost is O(grid x sites) with no spatial index; the grid downsamples
    // above FIELD_LARGE_POINT_COUNT (160x100).
    #drawFieldCanvas(points, bbox) {
        const large = points.length > FIELD_LARGE_POINT_COUNT;
        const width = large ? FIELD_GRID_W_LARGE : FIELD_GRID_W;
        const height = large ? FIELD_GRID_H_LARGE : FIELD_GRID_H;
        if (!this.fieldCanvas) {
            this.fieldCanvas = document.createElement("canvas");
        }
        const canvas = this.fieldCanvas;
        if (canvas.width !== width || canvas.height !== height) {
            canvas.width = width;
            canvas.height = height;
        }
        const grid = buildFieldGrid(points, bbox, {
            width,
            height,
            k: FIELD_IDW_K,
            power: FIELD_IDW_POWER,
            falloffKm: Math.max(1, Number(this.fieldFalloffKm) || 25),
            smoothKm: Math.max(FIELD_SMOOTH_MIN_KM, Math.max(1, Number(this.fieldFalloffKm) || 25) * FIELD_SMOOTH_FRACTION),
            colorFor: (value) => this.#rgbForValue(value),
        });
        const ctx = canvas.getContext("2d");
        const image = ctx.createImageData(width, height);
        image.data.set(grid.data);
        ctx.putImageData(image, 0, 0);
        return canvas;
    }

    syncMarkerSizeLegend(maxBitsPerSecond) {
        const dots = [this.sizeDotSmall, this.sizeDotMedium, this.sizeDotLarge];
        if (dots.some((dot) => !dot)) {
            return;
        }

        const max = Math.max(1, toNumber(maxBitsPerSecond, 1));
        // `.67.4` Lever 2: sample the DISPLAYED (damped) marker radius, so the
        // legend cannot lie now that the traffic scaling is damped.
        const ratios = [0, 0.1, 1.0];
        const diameters = ratios.map((ratio) => displayMarkerRadiusPx(throughputRadiusPx(max * ratio, max)) * 2);

        const apply = (el, diameter) => {
            const sizePx = Math.max(10, Math.min(64, Math.round(toNumber(diameter, 12))));
            el.style.width = `${sizePx}px`;
            el.style.height = `${sizePx}px`;
        };

        apply(this.sizeDotSmall, diameters[0]);
        apply(this.sizeDotMedium, diameters[1]);
        apply(this.sizeDotLarge, diameters[2]);
    }

    attachedApsForSite(siteKey) {
        const aps = this.attachedInheritedApsBySite.get(siteKey) || [];
        return [...aps].sort((left, right) => left.name.localeCompare(right.name));
    }

    buildFanoutFeatures(siteProps, attachedAps) {
        if (!this.map || !siteProps || !Array.isArray(attachedAps) || attachedAps.length === 0) {
            return { lines: [], points: [] };
        }

        const siteCoord = this.latestRender?.siteFeatures
            ?.find((feature) => feature.properties?.key === siteProps.key)
            ?.geometry?.coordinates;
        if (!Array.isArray(siteCoord) || siteCoord.length < 2) {
            return { lines: [], points: [] };
        }

        const center = this.map.project(siteCoord);
        const radiusPx = Math.max(42, Math.min(94, 34 + (attachedAps.length * 4)));
        const baseAngle = -Math.PI / 2;
        const lines = [];
        const points = [];

        attachedAps.forEach((node, index) => {
            const angle = attachedAps.length === 1
                ? baseAngle
                : baseAngle + ((Math.PI * 2 * index) / attachedAps.length);
            const projected = [
                center.x + (Math.cos(angle) * radiusPx),
                center.y + (Math.sin(angle) * radiusPx),
            ];
            const lngLat = this.map.unproject(projected);
            const metricValue = this.mode === "qoo"
                ? node.qooWorst
                : this.mode === "throughput" ? node.throughputCombined : node.rttWorst;
            const metricColor = metricColorForMode(this.mode, metricValue, this.latestRender?.maxBitsPerSecond);

            lines.push({
                type: "Feature",
                geometry: {
                    type: "LineString",
                    coordinates: [siteCoord, [lngLat.lng, lngLat.lat]],
                },
                properties: {
                    key: `${siteProps.key}->${node.key}`,
                },
            });

            const point = {
                type: "Feature",
                geometry: {
                    type: "Point",
                    coordinates: [lngLat.lng, lngLat.lat],
                },
                properties: {
                    key: node.key,
                    nodeId: node.id || "",
                    name: node.name,
                    nodeType: node.type,
                    parentName: node.parentName || siteProps.name || "",
                    parentType: node.parentType || "site",
                    inheritedCoords: 1,
                    throughputDown: node.throughputDown,
                    throughputUp: node.throughputUp,
                    throughputCombined: node.throughputCombined,
                    qooDown: node.qooDown,
                    qooUp: node.qooUp,
                    rttDownMs: node.rttDownMs,
                    rttUpMs: node.rttUpMs,
                    markerRadius: throughputRadiusPx(
                        node.throughputCombined,
                        Math.max(this.latestRender?.maxBitsPerSecond || 1, 1),
                    ),
                    metricColor,
                    isFanoutAp: 1,
                },
            };
            if (Number.isFinite(node.qooWorst)) {
                point.properties.qooWorst = node.qooWorst;
            }
            if (Number.isFinite(node.rttWorst)) {
                point.properties.rttWorst = node.rttWorst;
            }
            points.push(point);
        });

        return { lines, points };
    }

    syncFanoutOverlays() {
        if (!this.map) {
            return;
        }

        const lineSource = this.map.getSource(FANOUT_LINE_SOURCE_ID);
        const pointSource = this.map.getSource(FANOUT_POINT_SOURCE_ID);
        if (!lineSource || !pointSource) {
            return;
        }

        if (!this.fanoutSiteKey || !this.selectedFeature || this.selectedFeature.nodeType !== "site") {
            lineSource.setData({ type: "FeatureCollection", features: [] });
            pointSource.setData({ type: "FeatureCollection", features: [] });
            return;
        }

        const attachedAps = this.attachedApsForSite(this.fanoutSiteKey);
        const fanout = this.buildFanoutFeatures(this.selectedFeature, attachedAps);
        lineSource.setData({ type: "FeatureCollection", features: fanout.lines });
        pointSource.setData({ type: "FeatureCollection", features: fanout.points });
    }

    clearMarkerSet(markerSet) {
        markerSet.forEach((marker) => marker.remove());
        markerSet.clear();
    }

    scheduleTextOverlaySync() {
        if (!this.map) {
            return;
        }
        if (this.pendingTextOverlayFrame !== null) {
            window.cancelAnimationFrame(this.pendingTextOverlayFrame);
        }
        this.pendingTextOverlayFrame = window.requestAnimationFrame(() => {
            this.pendingTextOverlayFrame = null;
            this.syncClusterCountMarkers();
            this.syncSiteLabelMarkers();
        });
    }

    destroyTextOverlays() {
        if (this.pendingTextOverlayFrame !== null) {
            window.cancelAnimationFrame(this.pendingTextOverlayFrame);
            this.pendingTextOverlayFrame = null;
        }
        this.clearMarkerSet(this.siteLabelMarkers);
        this.clearMarkerSet(this.clusterCountMarkers);
    }

    // `.67.1`: labels prioritize the HIGHER-RTT sites (the card's "labels for the
    // higher-RTT sites"), with the selected site pinned on top.
    siteLabelPriority(feature, selectedKey) {
        const props = feature?.properties || {};
        const rtt = toNumber(props.rttWorst, 0);
        const isSelected = props.key === selectedKey;
        return (isSelected ? 1e18 : 0) + rtt;
    }

    estimateSiteLabelRect(feature, displayName, isSelected) {
        if (!this.map || !feature?.geometry?.coordinates) {
            return null;
        }
        const projected = this.map.project(feature.geometry.coordinates);
        const textLength = Math.max(4, String(displayName || "").length);
        const width = Math.min(220, Math.max(76, (textLength * 7.1) + 26));
        const height = isSelected ? 28 : 24;
        const bottom = projected.y - 16;
        const top = bottom - height;
        return {
            left: projected.x - (width / 2),
            right: projected.x + (width / 2),
            top,
            bottom,
        };
    }

    createTextMarker(className, text, lngLat, offset) {
        const el = document.createElement("div");
        el.className = className;
        el.textContent = text;
        return new window.maplibregl.Marker({
            element: el,
            anchor: "bottom",
            offset,
        }).setLngLat(lngLat);
    }

    syncClusterCountMarkers() {
        // `.67.1`: the cluster count overlay is a DOM marker; gate on the Cluster
        // filter (the clustered SOURCE is what emits clusters).
        if (!this.map || this.map.getZoom() < 0 || !this.filters.cluster) {
            this.clearMarkerSet(this.clusterCountMarkers);
            return;
        }

        const clusterFeatures = this.map.queryRenderedFeatures({ layers: [SITE_CLUSTER_LAYER_ID] }) || [];
        const nextKeys = new Set();
        clusterFeatures.forEach((feature) => {
            const clusterId = feature?.properties?.cluster_id;
            if (!Number.isFinite(clusterId)) {
                return;
            }
            const key = `cluster:${clusterId}`;
            nextKeys.add(key);
            if (this.clusterCountMarkers.has(key)) {
                return;
            }
            const text = String(feature.properties?.point_count_abbreviated ?? feature.properties?.point_count ?? "");
            const marker = this.createTextMarker(
                "site-map-cluster-count-marker",
                text,
                feature.geometry.coordinates,
                [0, 0],
            );
            marker.addTo(this.map);
            this.clusterCountMarkers.set(key, marker);
        });

        this.clusterCountMarkers.forEach((marker, key) => {
            if (!nextKeys.has(key)) {
                marker.remove();
                this.clusterCountMarkers.delete(key);
            }
        });
    }

    syncSiteLabelMarkers() {
        // `.67.1` (c): the labels are a DOM overlay (`site-map-site-label-marker`,
        // via scheduleTextOverlaySync), NOT a MapLibre layer — the "Show labels"
        // toggle clears them.
        if (!this.map || !this.latestRender || !this.filters.showLabels) {
            this.clearMarkerSet(this.siteLabelMarkers);
            return;
        }

        const zoom = this.map.getZoom();
        const selectedSiteFeature = this.selectedFeature?.nodeType === "site"
            ? this.latestRender.siteFeatures?.find((feature) => feature.properties?.key === this.selectedFeature.key)
            : null;

        const candidates = [];
        if (zoom >= SITE_LABEL_MIN_ZOOM) {
            // `.67.7`: in Heatmap mode the site/plain layers are hidden, so
            // `queryRenderedFeatures` is empty — label from the data instead,
            // limited to the current viewport (mirrors "rendered").
            let visibleFeatures;
            if (this.renderMode === "heatmap") {
                const bounds = this.map.getBounds();
                visibleFeatures = (this.latestRender.siteFeatures || []).filter(
                    (feature) => feature.geometry?.coordinates
                        && bounds.contains(feature.geometry.coordinates),
                );
            } else {
                const siteLayer = this.filters.cluster ? SITE_POINTS_LAYER_ID : SITE_PLAIN_POINTS_LAYER_ID;
                visibleFeatures = this.map.queryRenderedFeatures({ layers: [siteLayer] }) || [];
            }
            visibleFeatures.forEach((feature) => {
                const props = feature?.properties || {};
                if (!props.key) {
                    return;
                }
                const displayName = props.displayName || displayNodeName(props.name, props.nodeType);
                candidates.push({
                    key: props.key,
                    feature,
                    displayName,
                    selected: props.key === this.selectedFeature?.key,
                    priority: this.siteLabelPriority(feature, this.selectedFeature?.key),
                });
            });
        }

        if (selectedSiteFeature && zoom >= SELECTED_SITE_LABEL_MIN_ZOOM) {
            const alreadyPresent = candidates.some((candidate) => candidate.key === selectedSiteFeature.properties?.key);
            if (!alreadyPresent) {
                candidates.push({
                    key: selectedSiteFeature.properties?.key,
                    feature: selectedSiteFeature,
                    displayName: selectedSiteFeature.properties?.displayName
                        || displayNodeName(selectedSiteFeature.properties?.name, "site"),
                    selected: true,
                    priority: this.siteLabelPriority(selectedSiteFeature, this.selectedFeature?.key),
                });
            }
        }

        candidates.sort((left, right) => right.priority - left.priority);

        const accepted = [];
        const acceptedRects = [];
        for (const candidate of candidates) {
            if (!candidate.selected && accepted.length >= MAX_SITE_LABELS) {
                continue;
            }
            const rect = this.estimateSiteLabelRect(candidate.feature, candidate.displayName, candidate.selected);
            if (!rect) {
                continue;
            }
            if (!candidate.selected && acceptedRects.some((existing) => rectsOverlap(existing, rect))) {
                continue;
            }
            accepted.push(candidate);
            acceptedRects.push(rect);
        }

        this.clearMarkerSet(this.siteLabelMarkers);
        accepted.forEach((candidate) => {
            const marker = this.createTextMarker(
                candidate.selected ? "site-map-site-label-marker is-selected" : "site-map-site-label-marker",
                candidate.displayName,
                candidate.feature.geometry.coordinates,
                [0, -18],
            );
            marker.addTo(this.map);
            this.siteLabelMarkers.set(candidate.key, marker);
        });
    }

    fitToDataIfNeeded(siteFeatures, apFeatures) {
        if (!this.map) {
            return;
        }

        const features = [
            ...siteFeatures,
            // Include only APs with their own coordinates so a bad parent/duplicate doesn't pull the view out.
            ...apFeatures.filter((feature) => feature?.properties?.inheritedCoords !== 1),
        ];
        if (!features.length) {
            this.hasFitOnce = true;
            return;
        }

        this.map.resize();

        if (features.length === 1) {
            this.map.easeTo({
                center: features[0].geometry.coordinates,
                zoom: SINGLE_POINT_INITIAL_ZOOM,
                duration: 0,
            });
            this.hasFitOnce = true;
            return;
        }

        let viewBounds;
        try {
            viewBounds = this.map.getBounds();
        } catch (_) {
            viewBounds = null;
        }

        if (viewBounds) {
            const allInside = features.every((feature) => viewBounds.contains(feature.geometry.coordinates));
            if (allInside) {
                this.hasFitOnce = true;
                return;
            }
        }

        const bounds = new window.maplibregl.LngLatBounds();
        features.forEach((feature) => bounds.extend(feature.geometry.coordinates));
        const canvas = this.map.getCanvas();
        const minDimension = Math.max(
            320,
            Math.min(canvas?.clientWidth || 0, canvas?.clientHeight || 0),
        );
        const adaptivePadding = Math.max(
            INITIAL_FIT_PADDING,
            Math.round(minDimension * 0.08),
        );
        this.map.fitBounds(bounds, {
            padding: adaptivePadding,
            maxZoom: INITIAL_FIT_MAX_ZOOM,
            duration: 0,
        });
        this.hasFitOnce = true;
    }

    pointPopupHtml(props) {
        const attachedApCount = toNumber(props.attachedInheritedApCount, 0);
        const displayName = displayNodeName(props.name, props.nodeType);
        return `
            <div class="small">
                <div class="fw-semibold">${escapeHtml(displayName)}</div>
                <div class="text-muted mb-2">${escapeHtml(String(props.nodeType || "").toUpperCase())}</div>
                <div><strong>Throughput:</strong> ${escapeHtml(formatBitsPerSecond(props.throughputCombined))}</div>
                ${attachedApCount > 0 ? `<div><strong>Attached APs:</strong> ${escapeHtml(String(attachedApCount))}</div>` : ""}
                ${this.mode === "throughput" ? "" : `<div><strong>${this.mode === "qoo" ? "QoO" : "RTT"}:</strong> ${escapeHtml(this.mode === "qoo" ? formatPercent(Math.min(toNumber(props.qooDown, NaN), toNumber(props.qooUp, NaN))) : formatMs(Math.max(toNumber(props.rttDownMs, NaN), toNumber(props.rttUpMs, NaN))))}</div>`}
            </div>`;
    }

    selectFeature(props) {
        if (!props) {
            return;
        }
        const attachedAps = props.nodeType === "site" ? this.attachedApsForSite(props.key) : [];
        const sameSelection = this.selectedFeature?.key === props.key;

        this.selectedFeature = props;
        if (props.nodeType === "site" && attachedAps.length > 0) {
            this.fanoutSiteKey = sameSelection && this.fanoutSiteKey === props.key ? null : props.key;
        } else {
            this.fanoutSiteKey = null;
        }
        this.syncFanoutOverlays();
        this.scheduleTextOverlaySync();
        this.renderDetails(props);
    }

    updateSelection() {
        if (!this.selectedFeature || !this.latestRender) {
            return;
        }
        const current = [...this.latestRender.siteFeatures, ...this.latestRender.apFeatures]
            .find((feature) => feature.properties.key === this.selectedFeature.key);
        if (!current) {
            this.selectedFeature = null;
            this.fanoutSiteKey = null;
            this.syncFanoutOverlays();
            this.scheduleTextOverlaySync();
            this.renderDetails(null);
            return;
        }
        this.selectedFeature = current.properties;
        if (this.selectedFeature.nodeType !== "site" || this.selectedFeature.key !== this.fanoutSiteKey) {
            this.fanoutSiteKey = null;
        }
        this.syncFanoutOverlays();
        this.scheduleTextOverlaySync();
        this.renderDetails(current.properties);
    }

    renderDetails(props) {
        if (!props) {
            this.detailsPanel.style.display = "none";
            if (this.attachedList) {
                this.attachedList.innerHTML = "";
            }
            return;
        }
        this.detailsPanel.style.display = "block";
        const detailsTitleText = displayNodeName(props.name, props.nodeType);
        const treePageHref = treePageHrefForNodeId(props.nodeId);
        this.detailsTitle.innerHTML = treePageHref
            ? `<a class="link-body-emphasis text-decoration-none" href="${escapeHtml(treePageHref)}">${redactableHtml(detailsTitleText)}</a>`
            : redactableHtml(detailsTitleText);
        const parentName = displayParentName(props.parentName, props.parentType);
        const subtitleBits = [
            escapeHtml(String(props.nodeType || "").toUpperCase()),
            parentName ? ` · parent ${redactableHtml(parentName)}` : "",
            props.inheritedCoords ? " · using parent site coordinates" : "",
        ];
        this.detailsSubtitle.innerHTML = subtitleBits.join("");
        const attachedAps = props.nodeType === "site" ? this.attachedApsForSite(props.key) : [];
        if (this.attachedList) {
            if (attachedAps.length === 0) {
                this.attachedList.innerHTML = "";
            } else {
                const preview = attachedAps
                    .slice(0, 6)
                    .map((node) => `<div class="site-map-list-item">${redactableHtml(displayNodeName(node.name, node.type))}</div>`)
                    .join("");
                const more = attachedAps.length > 6
                    ? `<div class="site-map-empty">+${attachedAps.length - 6} more APs attached to this site</div>`
                    : "";
                const fanoutHint = this.fanoutSiteKey === props.key
                    ? "Click the selected site again or zoom the map to collapse them."
                    : "Click the selected site marker again to fan them out on the map.";
                this.attachedList.innerHTML = `<div class="site-map-empty">${attachedAps.length} AP${attachedAps.length === 1 ? "" : "s"} inherit this site's coordinates. ${fanoutHint}</div>${preview}${more}`;
            }
        }
        this.detailsGrid.innerHTML = [
            this.metricCard("Combined throughput", formatBitsPerSecond(props.throughputCombined)),
            this.metricCard("Download throughput", formatBitsPerSecond(props.throughputDown)),
            this.metricCard("Upload throughput", formatBitsPerSecond(props.throughputUp)),
            props.nodeType === "site"
                ? this.metricCard("Attached APs", String(attachedAps.length))
                : this.metricCard("Coordinate source", props.inheritedCoords ? "Inherited from site" : "Explicit"),
            this.metricCard("QoO download", formatPercent(props.qooDown)),
            this.metricCard("QoO upload", formatPercent(props.qooUp)),
            this.metricCard("RTT download", formatMs(props.rttDownMs)),
            this.metricCard("RTT upload", formatMs(props.rttUpMs)),
        ].join("");
    }

    metricCard(label, value) {
        return `<div class="site-map-metric"><label>${escapeHtml(label)}</label><div>${escapeHtml(value)}</div></div>`;
    }

    renderUnmapped(unmappedSites, unmappedAps) {
        const total = unmappedSites.length + unmappedAps.length;
        this.unmappedBadge.textContent = String(total);
        if (total === 0) {
            this.unmappedSummary.textContent = "All mapped nodes have display coordinates.";
            this.unmappedList.innerHTML = "";
            this.syncUnmappedPanel();
            return;
        }
        this.unmappedSummary.textContent = `${total} nodes cannot be placed on the map yet.`;
        const groups = [];
        if (unmappedSites.length) {
            groups.push(this.renderUnmappedGroup("Sites", unmappedSites));
        }
        if (unmappedAps.length) {
            groups.push(this.renderUnmappedGroup("APs", unmappedAps));
        }
        this.unmappedList.innerHTML = groups.join("");
        this.syncUnmappedPanel();
    }

    renderUnmappedGroup(label, nodes) {
        const items = nodes
            .sort((a, b) => a.name.localeCompare(b.name))
            .map((node) => `<div class="site-map-list-item">${redactableHtml(displayNodeName(node.name, node.type))}${node.parentName ? `<div class="text-muted small">${redactableHtml(displayParentName(node.parentName, node.parentType))}</div>` : ""}</div>`)
            .join("");
        return `<div class="site-map-list-group"><h6>${escapeHtml(label)}</h6>${items}</div>`;
    }

    syncUnmappedPanel() {
        this.unmappedPanel.classList.toggle("is-open", this.unmappedOpen);
    }

    setStatus(text, kind) {
        const icon = kind === "success"
            ? "fa-circle-check"
            : kind === "warning"
                ? "fa-triangle-exclamation"
                : "fa-spinner fa-spin";
        this.statusChip.innerHTML = `<i class="fa ${icon}"></i> ${escapeHtml(text)}`;
    }

    startUpdatedClock() {
        setInterval(() => {
            this.updatedChip.innerHTML = `<i class="fa fa-clock"></i> ${escapeHtml(nowText(this.lastUpdateAt))}`;
        }, 1000);
    }

    refreshLegend() {
        const cvd = isColorBlindMode();
        const heat = this.renderMode === "heatmap";
        const value = heat && this.fieldMode === "value";
        // REQUIRED-3 / card v3: ONE threshold source PER MODE.
        //   VALUE   (the default) => `resolveRttThresholds()` (the CONFIG) for
        //            the field, the markers, and this legend.
        //   DENSITY => the `.67.1` three handles for the weight + this legend.
        // The value legend must NOT read the handles, or a handle drag relabels
        // it without recoloring any pixel.
        // `.67.8`: the `.67.1` handles are gone — one CONFIG threshold source.
        // `.67.9` R1: the legend reads `.greenMs/.yellowMs/.redMs`, so it must use
        // `resolveRttThresholds()` (which returns those keys) — NOT
        // `defaultHeatRange()` (which returns `{green,yellow,red}` and printed
        // "undefined ms").
        const thresholds = resolveRttThresholds();
        const metricLabel = this.mode === "qoo" ? "QoO" : this.mode === "throughput" ? "Throughput" : "RTT";
        if (this.legendTitle) {
            this.legendTitle.textContent = heat
                ? (value ? `${metricLabel} field (estimated · interpolated)` : `${metricLabel} hexbin`)
                : `${metricLabel} markers`;
        }
        if (heat) {
            // `.67.15`: the hexbin fill uses the CALLER's CONFIG scale — the SAME
            // ramp + thresholds the markers use (ONE scale), so the legend gradient
            // matches the marker legend, and low/high are the CONFIGURED ms.
            this.legendGradient.style.background = cvd
                ? "linear-gradient(90deg, #440154 0%, #3b528b 30%, #21918c 55%, #5ec962 78%, #fde725 100%)"
                : (this.mode === "qoo"
                    ? "linear-gradient(90deg, #ff0000 0%, #bf4000 25%, #808000 50%, #40bf00 75%, #00ff00 100%)"
                    : "linear-gradient(90deg, #61d26f 0%, #d7d86b 35%, #f0a44d 62%, #d94b5b 100%)");
            if (this.mode === "qoo") {
                this.legendLow.textContent = "Bad (0)";
                this.legendHigh.textContent = "Good (100)";
            } else if (this.mode === "throughput") {
                this.legendLow.textContent = "0";
                this.legendHigh.textContent = "max";
            } else {
                this.legendLow.textContent = `${thresholds.greenMs} ms`;
                this.legendHigh.textContent = `${thresholds.redMs} ms`;
            }
        } else if (this.mode === "qoo") {
            this.legendGradient.style.background = cvd
                ? "linear-gradient(90deg, #440154 0%, #3b528b 30%, #21918c 55%, #5ec962 78%, #fde725 100%)"
                : "linear-gradient(90deg, #ff0000 0%, #bf4000 25%, #808000 50%, #40bf00 75%, #00ff00 100%)";
            this.legendLow.textContent = "Poor QoO";
            this.legendHigh.textContent = "Healthy QoO";
        } else {
            this.legendGradient.style.background = cvd
                ? "linear-gradient(90deg, #440154 0%, #3b528b 30%, #21918c 55%, #5ec962 78%, #fde725 100%)"
                : "linear-gradient(90deg, #61d26f 0%, #d7d86b 35%, #f0a44d 62%, #d94b5b 100%)";
            this.legendLow.textContent = this.mode === "throughput" ? "Low traffic" : "Low RTT";
            this.legendHigh.textContent = this.mode === "throughput" ? "High traffic" : "High RTT";
        }
        if (this.legendCaveat) {
            this.legendCaveat.style.display = heat && !value ? "" : "none";
        }
        if (this.fieldCaveat) {
            this.fieldCaveat.style.display = value ? "" : "none";
            if (value) {
                // IDW is a weighted MEAN, so the label is "estimated
                // (interpolated)", never "median".
                this.fieldCaveat.textContent = `Estimated (interpolated) ${metricLabel} field — an IDW weighted MEAN of the ${FIELD_IDW_K} nearest sites (a model, not measured between them; not a median). Smoothed to a continuous surface (a length-scale damps the per-site peaks); coarse when sites are sparse. Each site's glow radius scales with its live throughput; the RTT color follows resolveRttThresholds(). Mercator-aligned to the markers (within one cell).`;
            }
        }
        if (this.legendStops) {
            if (!heat) {
                this.legendStops.style.display = "none";
            } else {
                this.legendStops.style.display = "";
                if (value) {
                    this.legendStops.textContent = this.mode === "qoo"
                        ? "interpolated QoO (0 bad · 100 good)"
                        : this.mode === "throughput"
                            ? "interpolated throughput (0 · max)"
                            : `interpolated RTT: green ${thresholds.greenMs} · yellow ${thresholds.yellowMs} · red ${thresholds.redMs} ms`;
                } else {
                    // `.67.12`: the hexbin — color = metric, size = throughput.
                    this.legendStops.textContent = this.mode === "qoo"
                        ? "fill = QoO · halo radius = site throughput"
                        : this.mode === "throughput"
                            ? "fill = throughput · halo radius = site throughput"
                            : "fill = RTT at the configured thresholds · halo radius = site throughput";
                }
            }
        }
    }
}

document.addEventListener("DOMContentLoaded", () => {
    const page = new SiteMapPage();
    page.init();
});
