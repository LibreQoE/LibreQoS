import { colorByQoqScore, colorByRttMs } from "./helpers/color_scales";
import { isColorBlindMode } from "./helpers/colorblind";
import { isDarkMode } from "./helpers/dark_mode";
import { isRedacted } from "./helpers/redact";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client, subscribeWS } from "./pubsub/ws";

const wsClient = get_ws_client();

const INITIAL_REQUEST_TIMEOUT_MS = 2500;
const HISTORY_WINDOW_MS = 30_000;

const TILE_BBOX_URL = "https://insight.libreqos.com/tiles/api/bbox";
// NOTE: This key is intentionally non-secret. The remote OSM cache uses it as a lightweight gate.
// It must match both the bbox Authorization token and the tile `key=` query param.
const OSM_CACHE_KEY = "LibreQoSRocks";
const WEB_MERCATOR_MAX_LAT = 85.05112878;
const INSIGHT_TILE_PROTOCOL = "insight";
// Insight OSM cache tiles: `z/y/x.png` (Cloudflare cache-friendly). See `src/lts2/rust/osm_cache/src/http.rs`.
// The Insight tile server returns `503 + Retry-After` while it fetches missing tiles, so we route
// requests through a custom MapLibre protocol handler that retries before surfacing errors.
const TILE_URL_TEMPLATE = `${INSIGHT_TILE_PROTOCOL}://insight.libreqos.com/tiles/{z}/{y}/{x}.png?key=${encodeURIComponent(OSM_CACHE_KEY)}`;
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

function metricColorForMode(mode, metricValue) {
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
    return colorByRttMs(numeric);
}

function configuredMaxThroughput(node) {
    return node?.configured_max_throughput || node?.max_throughput || [0, 0];
}

function effectiveMaxThroughput(node) {
    return node?.effective_max_throughput || configuredMaxThroughput(node);
}

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
    if (isDarkMode()) {
        return {
            siteStroke: "rgba(244, 248, 255, 0.68)",
            apStroke: "rgba(244, 248, 255, 0.54)",
            link: "rgba(15, 23, 42, 0.35)",
        };
    }
    return {
        siteStroke: "rgba(15, 23, 42, 0.36)",
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
        this.mode = "qoo";
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
        this.sizeDotSmall = document.getElementById("siteMapSizeDotSmall");
        this.sizeDotMedium = document.getElementById("siteMapSizeDotMedium");
        this.sizeDotLarge = document.getElementById("siteMapSizeDotLarge");
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
        const setMode = (mode) => {
            this.mode = mode;
            document.getElementById("siteMapModeQoo")?.classList.toggle("btn-primary", mode === "qoo");
            document.getElementById("siteMapModeQoo")?.classList.toggle("btn-outline-secondary", mode !== "qoo");
            document.getElementById("siteMapModeRtt")?.classList.toggle("btn-primary", mode === "rtt");
            document.getElementById("siteMapModeRtt")?.classList.toggle("btn-outline-secondary", mode !== "rtt");
            this.refreshLegend();
            this.renderFromHistory();
        };
        document.getElementById("siteMapModeQoo")?.addEventListener("click", () => setMode("qoo"));
        document.getElementById("siteMapModeRtt")?.addEventListener("click", () => setMode("rtt"));
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
            try {
                this.installSourcesAndLayers();
            } catch (err) {
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
                    "circle-radius": ["get", "markerRadius"],
                    "circle-opacity": [
                        "interpolate", ["linear"], ["zoom"],
                        0, 0.86,
                        6, 0.76,
                        10, 0.62,
                    ],
                    "circle-stroke-color": markerPalette().siteStroke,
                    "circle-stroke-width": 1.15,
                    "circle-blur": 0.06,
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
        const pointLayers = [SITE_POINTS_LAYER_ID, AP_POINTS_LAYER_ID, FANOUT_POINT_LAYER_ID];

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
                console.error("Site map bbox request failed; map not initialized.", err);
                this.setStatus("Waiting for Insight map location", "warning");
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
            const maxMbps = effectiveMaxThroughput(node);
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

            const metricValue = this.mode === "qoo" ? node.qooWorst : node.rttWorst;
            const metricColor = metricColorForMode(this.mode, metricValue);
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
            || !this.map.getSource(SITE_LINK_SOURCE_ID)) {
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
        const siteSource = this.map.getSource(SITE_SOURCE_ID);
        const apSource = this.map.getSource(AP_SOURCE_ID);
        const linkSource = this.map.getSource(SITE_LINK_SOURCE_ID);
        if (!siteSource || !apSource || !linkSource) {
            return;
        }
        siteSource.setData({
            type: "FeatureCollection",
            features: aggregate.siteFeatures,
        });
        apSource.setData({
            type: "FeatureCollection",
            features: aggregate.apFeatures,
        });
        linkSource.setData({
            type: "FeatureCollection",
            features: aggregate.siteLinkFeatures ?? [],
        });

        const sparseApCoverage = aggregate.apFeatures.length < 24;
        const siteOpacity = sparseApCoverage
            ? ["interpolate", ["linear"], ["zoom"], 0, 0.86, 6, 0.76, 10, 0.62]
            : ["interpolate", ["linear"], ["zoom"], 0, 0.86, 6, 0.72, 8, 0.28, 10, 0.12];
        const apOpacity = sparseApCoverage
            ? ["interpolate", ["linear"], ["zoom"], 0, 0.02, 6, 0.22, 8, 0.78, 10, 0.92]
            : ["interpolate", ["linear"], ["zoom"], 0, 0.0, 6, 0.18, 8, 0.82, 10, 0.96];

        this.map.setPaintProperty(SITE_POINTS_LAYER_ID, "circle-opacity", siteOpacity);
        this.map.setPaintProperty(AP_POINTS_LAYER_ID, "circle-opacity", apOpacity);

        this.syncFanoutOverlays();
        this.scheduleTextOverlaySync();
        this.syncMarkerSizeLegend(aggregate.maxBitsPerSecond);
    }

    syncMarkerSizeLegend(maxBitsPerSecond) {
        const dots = [this.sizeDotSmall, this.sizeDotMedium, this.sizeDotLarge];
        if (dots.some((dot) => !dot)) {
            return;
        }

        const max = Math.max(1, toNumber(maxBitsPerSecond, 1));
        // Match the actual marker sizing function by sampling at a few ratios.
        const ratios = [0, 0.1, 1.0];
        const diameters = ratios.map((ratio) => throughputRadiusPx(max * ratio, max) * 2);

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
            const metricValue = this.mode === "qoo" ? node.qooWorst : node.rttWorst;
            const metricColor = metricColorForMode(this.mode, metricValue);

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

    siteLabelPriority(feature, selectedKey) {
        const props = feature?.properties || {};
        const throughputCombined = toNumber(props.throughputCombined, 0);
        const attachedCount = toNumber(props.attachedInheritedApCount, 0);
        const isSelected = props.key === selectedKey;
        return (isSelected ? 1e18 : 0) + throughputCombined + (attachedCount * 5_000_000);
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
        if (!this.map || this.map.getZoom() < 0) {
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
        if (!this.map || !this.latestRender) {
            this.clearMarkerSet(this.siteLabelMarkers);
            return;
        }

        const zoom = this.map.getZoom();
        const selectedSiteFeature = this.selectedFeature?.nodeType === "site"
            ? this.latestRender.siteFeatures?.find((feature) => feature.properties?.key === this.selectedFeature.key)
            : null;

        const candidates = [];
        if (zoom >= SITE_LABEL_MIN_ZOOM) {
            const visibleFeatures = this.map.queryRenderedFeatures({ layers: [SITE_POINTS_LAYER_ID] }) || [];
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
                <div><strong>${this.mode === "qoo" ? "QoO" : "RTT"}:</strong> ${escapeHtml(this.mode === "qoo" ? formatPercent(Math.min(toNumber(props.qooDown, NaN), toNumber(props.qooUp, NaN))) : formatMs(Math.max(toNumber(props.rttDownMs, NaN), toNumber(props.rttUpMs, NaN))))}</div>
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
        this.detailsTitle.textContent = displayNodeName(props.name, props.nodeType);
        const parentName = displayParentName(props.parentName, props.parentType);
        this.detailsSubtitle.textContent = `${String(props.nodeType || "").toUpperCase()}${parentName ? ` · parent ${parentName}` : ""}${props.inheritedCoords ? " · using parent site coordinates" : ""}`;
        const attachedAps = props.nodeType === "site" ? this.attachedApsForSite(props.key) : [];
        if (this.attachedList) {
            if (attachedAps.length === 0) {
                this.attachedList.innerHTML = "";
            } else {
                const preview = attachedAps
                    .slice(0, 6)
                    .map((node) => `<div class="site-map-list-item">${escapeHtml(displayNodeName(node.name, node.type))}</div>`)
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
            .map((node) => `<div class="site-map-list-item">${escapeHtml(displayNodeName(node.name, node.type))}${node.parentName ? `<div class="text-muted small">${escapeHtml(displayParentName(node.parentName, node.parentType))}</div>` : ""}</div>`)
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
        if (this.mode === "qoo") {
            this.legendGradient.style.background = isColorBlindMode()
                ? "linear-gradient(90deg, #440154 0%, #3b528b 30%, #21918c 55%, #5ec962 78%, #fde725 100%)"
                : "linear-gradient(90deg, #ff0000 0%, #bf4000 25%, #808000 50%, #40bf00 75%, #00ff00 100%)";
            this.legendLow.textContent = "Poor QoO";
            this.legendHigh.textContent = "Healthy QoO";
        } else {
            this.legendGradient.style.background = isColorBlindMode()
                ? "linear-gradient(90deg, #440154 0%, #3b528b 30%, #21918c 55%, #5ec962 78%, #fde725 100%)"
                : "linear-gradient(90deg, #61d26f 0%, #d7d86b 35%, #f0a44d 62%, #d94b5b 100%)";
            this.legendLow.textContent = "Low RTT";
            this.legendHigh.textContent = "High RTT";
        }
    }
}

document.addEventListener("DOMContentLoaded", () => {
    const page = new SiteMapPage();
    page.init();
});
