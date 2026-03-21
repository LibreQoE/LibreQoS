import { colorByQoqScore, colorByRttMs } from "./helpers/color_scales";
import { isColorBlindMode } from "./helpers/colorblind";
import { isDarkMode } from "./helpers/dark_mode";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client, subscribeWS } from "./pubsub/ws";

const wsClient = get_ws_client();

const INITIAL_REQUEST_TIMEOUT_MS = 2500;
const HISTORY_WINDOW_MS = 30_000;

const TILE_BBOX_URL = "https://insight.libreqos.com/tiles/api/bbox";
// NOTE: This key is intentionally non-secret. The remote OSM cache uses it as a lightweight gate.
// It must match both the bbox Authorization token and the tile `key=` query param.
const OSM_CACHE_KEY = "LibreQoSRocks";
const INSIGHT_TILE_PROTOCOL = "insight";
// Insight OSM cache tiles: `z/y/x` (no `.png` suffix). See `src/lts2/rust/osm_cache/src/http.rs`.
// The Insight tile server returns `503 + Retry-After` while it fetches missing tiles, so we route
// requests through a custom MapLibre protocol handler that retries before surfacing errors.
const TILE_URL_TEMPLATE = `${INSIGHT_TILE_PROTOCOL}://insight.libreqos.com/tiles/{z}/{y}/{x}?key=${encodeURIComponent(OSM_CACHE_KEY)}`;
const TILE_ATTRIBUTION = "© OpenStreetMap contributors";
const TILE_MAX_ZOOM = 17;

const FALLBACK_CENTER = [-101.5, 39.8];
const FALLBACK_ZOOM = 3.15;

const OSM_RASTER_SOURCE_ID = "site-map-osm";
const OSM_RASTER_LAYER_ID = "site-map-osm-tiles";
const SITE_SOURCE_ID = "site-map-sites";
const AP_SOURCE_ID = "site-map-aps";
const SITE_LINK_SOURCE_ID = "site-map-site-links";
const SITE_POINTS_LAYER_ID = "site-map-site-points";
const AP_POINTS_LAYER_ID = "site-map-ap-points";
const SITE_LINK_LAYER_ID = "site-map-site-links-line";

const INSIGHT_TILE_MAX_PARALLEL_FETCHES = 6;
let insightTileFetchActive = 0;
const insightTileFetchWaiters = [];

async function withInsightTileFetchSlot(fn) {
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

function averageOrNull(sum, count) {
    return count > 0 ? (sum / count) : null;
}

function hasMeaningfulLatLon(lat, lon) {
    if (!Number.isFinite(lat) || !Number.isFinite(lon)) {
        return false;
    }
    // Many integrations default to (0,0) when coordinates are unknown.
    return !(lat === 0 && lon === 0);
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

function buildOsmRasterStyle() {
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
            const deadlineMs = Date.now() + 90_000;
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

                if (resp.status === 503 || resp.status === 429) {
                    const retryAfter = Number(resp.headers.get("retry-after"));
                    const baseDelayMs = Number.isFinite(retryAfter) ? (retryAfter * 1000) : 1000;
                    const delayMs = Math.min(Math.max(baseDelayMs, 250), 15_000) + Math.round(Math.random() * 250);
                    await sleepMs(delayMs);
                    if (signal?.aborted) {
                        throw new Error("AbortError");
                    }
                    continue;
                }

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
            if (!Number.isFinite(latN) || !Number.isFinite(lonN)) {
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
        this.siteLabelMarkers = new Map();
        this.lastBboxAttemptAt = 0;

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
        this.detailsGrid = document.getElementById("siteMapDetailsGrid");
        this.legendGradient = document.getElementById("siteMapLegendGradient");
        this.legendLow = document.getElementById("siteMapLegendLow");
        this.legendHigh = document.getElementById("siteMapLegendHigh");
    }

    init() {
        this.bindControls();
        this.refreshLegend();
        this.requestInitialTree();
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
            this.renderDetails(null);
        });
        window.addEventListener("colorBlindModeChanged", () => {
            this.refreshLegend();
            this.renderFromHistory();
        });
    }

    initMap(center = FALLBACK_CENTER, zoom = FALLBACK_ZOOM) {
        installInsightTileProtocolOnce();
        if (typeof window.maplibregl?.setMaxParallelImageRequests === "function") {
            window.maplibregl.setMaxParallelImageRequests(6);
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
            });
        }
        if (!this.map.getSource(AP_SOURCE_ID)) {
            this.map.addSource(AP_SOURCE_ID, {
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
                    "line-width": [
                        "interpolate", ["linear"], ["zoom"],
                        2, 0.6,
                        6, 1.2,
                        10, 2.0,
                    ],
                    "line-opacity": [
                        "interpolate", ["linear"], ["zoom"],
                        2, 0.25,
                        6, 0.45,
                        10, 0.6,
                    ],
                },
            });
        }

        if (!this.map.getLayer(SITE_POINTS_LAYER_ID)) {
            this.map.addLayer({
                id: SITE_POINTS_LAYER_ID,
                type: "circle",
                source: SITE_SOURCE_ID,
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
    }

    installInteractions() {
        const pointLayers = [SITE_POINTS_LAYER_ID, AP_POINTS_LAYER_ID];

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
                this.selectedFeature = feature.properties;
                this.renderDetails(feature.properties);
            });
        });
    }

    requestInitialTree() {
        this.setStatus("Waiting for data", "spinner");
        if (!this.subscription) {
            this.subscription = subscribeWS(["NetworkTree"], (liveMsg) => {
                if (liveMsg.event === "NetworkTree") {
                    this.processTreeMessage(liveMsg);
                }
            });
        }
        listenOnceWithTimeout("NetworkTree", INITIAL_REQUEST_TIMEOUT_MS, (msg) => {
            this.processTreeMessage(msg);
        }, () => {
            this.setStatus("No data received yet", "warning");
        });
        wsClient.send({ NetworkTree: {} });
    }

    processTreeMessage(msg) {
        const data = Array.isArray(msg?.data) ? msg.data : [];
        this.history.push({ timestamp: Date.now(), data });
        const cutoff = Date.now() - HISTORY_WINDOW_MS;
        this.history = this.history.filter((entry) => entry.timestamp >= cutoff);
        this.latestSnapshot = data;
        this.lastUpdateAt = Date.now();
        this.setStatus("Live", "success");
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

        this.mapInitPromise = (async () => {
            try {
                if (siteLatLonPairs.length === 1) {
                    const [lat, lon] = siteLatLonPairs[0];
                    siteLatLonPairs.push([lat + 0.0001, lon + 0.0001]);
                }
                const center = await requestOsmCenterFromBbox(siteLatLonPairs, 4000);
                this.hasFitOnce = true;
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
        const byIndex = new Map();

        aggregate.forEach((value) => {
            const node = value.latestNode;
            const nodeType = asNodeType(node);
            const avgDown = value.throughputSamples > 0 ? (value.throughputDown / value.throughputSamples) : 0;
            const avgUp = value.throughputSamples > 0 ? (value.throughputUp / value.throughputSamples) : 0;
            const throughputCombined = avgDown + avgUp;
            maxBitsPerSecond = Math.max(maxBitsPerSecond, throughputCombined);

            const qooDown = averageOrNull(value.qooDownSum, value.qooDownCount);
            const qooUp = averageOrNull(value.qooUpSum, value.qooUpCount);
            const rttDownMs = averageOrNull(value.rttDownSum, value.rttDownCount);
            const rttUpMs = averageOrNull(value.rttUpSum, value.rttUpCount);

            const normalized = {
                key: value.key,
                index: value.latestIndex,
                name: node.name,
                id: node.id || null,
                type: nodeType,
                immediateParent: node.immediate_parent,
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
                inheritedCoords: false,
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
                    node.inheritedCoords = true;
                }
            } else if (node.immediateParent !== null && node.immediateParent !== undefined) {
                const parent = byIndex.get(node.immediateParent);
                if (parent) {
                    node.parentName = parent.name;
                }
            }
        });

        byIndex.forEach((node) => {
            if (node.latitude === null || node.longitude === null) {
                const listTarget = node.type === "site" ? unmappedSites : unmappedAps;
                listTarget.push(node);
                return;
            }

            const metricValue = this.mode === "qoo" ? node.qooWorst : node.rttWorst;
            const metricColor = this.mode === "qoo"
                ? colorByQoqScore(metricValue)
                : colorByRttMs(metricValue);
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
                    nodeType: node.type,
                    parentName: node.parentName || "",
                    inheritedCoords: node.inheritedCoords ? 1 : 0,
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
            const utilizationRatio = maxBitsPerSecond > 0 ? Math.max(0, Math.min(1, node.throughputCombined / maxBitsPerSecond)) : 0;
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
            maxBitsPerSecond,
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
        this.updateSources(aggregate);
        this.updateSelection();
        this.renderUnmapped(aggregate.unmappedSites, aggregate.unmappedAps);
        if (!this.hasFitOnce) {
            this.fitToData(aggregate.siteFeatures, aggregate.apFeatures);
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

        this.syncSiteLabels(aggregate.siteFeatures);
    }

    syncSiteLabels(siteFeatures) {
        if (!this.map) {
            return;
        }

        const wanted = new Set();
        siteFeatures.forEach((feature) => {
            const key = feature?.properties?.key;
            const name = feature?.properties?.name;
            const metricColor = feature?.properties?.metricColor;
            const coordinates = feature?.geometry?.coordinates;
            if (!key || !name || !Array.isArray(coordinates) || coordinates.length < 2) {
                return;
            }
            wanted.add(key);

            const existing = this.siteLabelMarkers.get(key);
            if (!existing) {
                const el = document.createElement("div");
                el.className = "site-map-site-label";
                el.innerHTML = `<span class="site-map-site-label-dot"></span><span class="site-map-site-label-text"></span>`;
                const dot = el.querySelector(".site-map-site-label-dot");
                const text = el.querySelector(".site-map-site-label-text");
                if (text) {
                    text.textContent = name;
                }
                if (dot && metricColor) {
                    dot.style.backgroundColor = metricColor;
                    dot.style.opacity = "0.82";
                }

                const marker = new window.maplibregl.Marker({
                    element: el,
                    anchor: "bottom",
                    offset: [0, -14],
                })
                    .setLngLat(coordinates)
                    .addTo(this.map);

                this.siteLabelMarkers.set(key, { marker, dot, text });
                return;
            }

            existing.marker.setLngLat(coordinates);
            if (existing.text) {
                existing.text.textContent = name;
            }
            if (existing.dot && metricColor) {
                existing.dot.style.backgroundColor = metricColor;
            }
        });

        Array.from(this.siteLabelMarkers.entries()).forEach(([key, value]) => {
            if (wanted.has(key)) {
                return;
            }
            value.marker.remove();
            this.siteLabelMarkers.delete(key);
        });
    }

    fitToData(siteFeatures, apFeatures) {
        const features = [...siteFeatures, ...apFeatures];
        if (!features.length) {
            return;
        }
        const bounds = new window.maplibregl.LngLatBounds();
        features.forEach((feature) => bounds.extend(feature.geometry.coordinates));
        this.map.fitBounds(bounds, {
            padding: 70,
            maxZoom: 7.5,
            duration: 0,
        });
        this.hasFitOnce = true;
    }

    pointPopupHtml(props) {
        return `
            <div class="small">
                <div class="fw-semibold">${escapeHtml(props.name)}</div>
                <div class="text-muted mb-2">${escapeHtml(String(props.nodeType || "").toUpperCase())}</div>
                <div><strong>Throughput:</strong> ${escapeHtml(formatBitsPerSecond(props.throughputCombined))}</div>
                <div><strong>${this.mode === "qoo" ? "QoO" : "RTT"}:</strong> ${escapeHtml(this.mode === "qoo" ? formatPercent(Math.min(toNumber(props.qooDown, NaN), toNumber(props.qooUp, NaN))) : formatMs(Math.max(toNumber(props.rttDownMs, NaN), toNumber(props.rttUpMs, NaN))))}</div>
                ${props.inheritedCoords ? `<div class="text-muted mt-1">Using parent site coordinates.</div>` : ""}
            </div>`;
    }

    updateSelection() {
        if (!this.selectedFeature || !this.latestRender) {
            return;
        }
        const current = [...this.latestRender.siteFeatures, ...this.latestRender.apFeatures]
            .find((feature) => feature.properties.key === this.selectedFeature.key);
        if (!current) {
            this.selectedFeature = null;
            this.renderDetails(null);
            return;
        }
        this.selectedFeature = current.properties;
        this.renderDetails(current.properties);
    }

    renderDetails(props) {
        if (!props) {
            this.detailsPanel.style.display = "none";
            return;
        }
        this.detailsPanel.style.display = "block";
        this.detailsTitle.textContent = props.name;
        this.detailsSubtitle.textContent = `${String(props.nodeType || "").toUpperCase()}${props.parentName ? ` · parent ${props.parentName}` : ""}${props.inheritedCoords ? " · using parent site coordinates" : ""}`;
        this.detailsGrid.innerHTML = [
            this.metricCard("Combined throughput", formatBitsPerSecond(props.throughputCombined)),
            this.metricCard("Download throughput", formatBitsPerSecond(props.throughputDown)),
            this.metricCard("Upload throughput", formatBitsPerSecond(props.throughputUp)),
            this.metricCard("QoO download", formatPercent(props.qooDown)),
            this.metricCard("QoO upload", formatPercent(props.qooUp)),
            this.metricCard("RTT download", formatMs(props.rttDownMs)),
            this.metricCard("RTT upload", formatMs(props.rttUpMs)),
            this.metricCard("Coordinate source", props.inheritedCoords ? "Inherited from site" : "Explicit"),
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
            .map((node) => `<div class="site-map-list-item">${escapeHtml(node.name)}${node.parentName ? `<div class="text-muted small">${escapeHtml(node.parentName)}</div>` : ""}</div>`)
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
                : "linear-gradient(90deg, #d94b5b 0%, #f5b84f 42%, #6ecc84 75%, #4ca4ff 100%)";
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
