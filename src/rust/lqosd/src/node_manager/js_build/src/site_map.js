import { colorByQoqScore, colorByRttMs } from "./helpers/color_scales";
import { isColorBlindMode } from "./helpers/colorblind";
import { isDarkMode } from "./helpers/dark_mode";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client, subscribeWS } from "./pubsub/ws";

const wsClient = get_ws_client();

const INITIAL_REQUEST_TIMEOUT_MS = 2500;
const HISTORY_WINDOW_MS = 30_000;
const COUNTRIES_GEOJSON_PATH = "vendor/countries.geojson";
const ADMIN1_BOUNDARIES_GEOJSON_PATH = "vendor/site_map_admin1_boundaries.geojson";
const COASTLINES_GEOJSON_PATH = "vendor/site_map_coastlines.geojson";
const LAKES_GEOJSON_PATH = "vendor/site_map_lakes.geojson";
const RIVERS_GEOJSON_PATH = "vendor/site_map_rivers.geojson";
const PHYSICAL_REGIONS_GEOJSON_PATH = "vendor/site_map_physical_regions.geojson";
const PHYSICAL_REGIONS_10M_GEOJSON_PATH = "vendor/site_map_physical_regions_10m.geojson";
const MARINE_AREAS_GEOJSON_PATH = "vendor/site_map_marine_areas.geojson";
const MAJOR_ROADS_10M_GEOJSON_PATH = "vendor/site_map_major_roads_10m.geojson";
const GRIP_ROADS_GEOJSON_PATH = "vendor/site_map_grip_roads.geojson";
const INITIAL_CENTER = [-101.5, 39.8];
const INITIAL_ZOOM = 3.15;
const SITE_SOURCE_ID = "site-map-sites";
const AP_SOURCE_ID = "site-map-aps";
const COUNTRIES_SOURCE_ID = "site-map-countries";
const ADMIN1_SOURCE_ID = "site-map-admin1";
const COASTLINE_SOURCE_ID = "site-map-coastlines";
const LAKES_SOURCE_ID = "site-map-lakes";
const RIVERS_SOURCE_ID = "site-map-rivers";
const PHYSICAL_REGIONS_SOURCE_ID = "site-map-physical-regions";
const PHYSICAL_REGIONS_10M_SOURCE_ID = "site-map-physical-regions-10m";
const MARINE_AREAS_SOURCE_ID = "site-map-marine-areas";
const MAJOR_ROADS_10M_SOURCE_ID = "site-map-major-roads-10m";
const GRIP_ROADS_SOURCE_ID = "site-map-grip-roads";
const SITE_CLUSTER_LAYER_ID = "site-map-site-clusters";
const SITE_POINTS_LAYER_ID = "site-map-site-points";
const AP_CLUSTER_LAYER_ID = "site-map-ap-clusters";
const AP_POINTS_LAYER_ID = "site-map-ap-points";
const COUNTRY_FILL_LAYER_ID = "site-map-country-fill";
const COUNTRY_LINE_LAYER_ID = "site-map-country-line";
const ADMIN1_LINE_LAYER_ID = "site-map-admin1-line";
const COASTLINE_LAYER_ID = "site-map-coastline-line";
const LAKES_FILL_LAYER_ID = "site-map-lakes-fill";
const RIVERS_LINE_LAYER_ID = "site-map-rivers-line";
const PHYSICAL_REGIONS_FILL_LAYER_ID = "site-map-physical-regions-fill";
const PHYSICAL_REGIONS_LINE_LAYER_ID = "site-map-physical-regions-line";
const PHYSICAL_REGIONS_10M_FILL_LAYER_ID = "site-map-physical-regions-10m-fill";
const PHYSICAL_REGIONS_10M_LINE_LAYER_ID = "site-map-physical-regions-10m-line";
const MARINE_AREAS_FILL_LAYER_ID = "site-map-marine-areas-fill";
const MAJOR_ROADS_10M_LAYER_ID = "site-map-major-roads-10m-line";
const GRIP_ROADS_LAYER_ID = "site-map-grip-roads-line";

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

function stableNodeKey(index, node) {
    return node.id || `${index}:${node.name}`;
}

function asNodeType(node) {
    return String(node?.type || node?.node_type || "").toLowerCase();
}

function countryPalette() {
    if (isDarkMode()) {
        return {
            background: "#09111d",
            land: "#132339",
            water: "#102945",
            marine: "rgba(16, 34, 56, 0.42)",
            terrain: "rgba(94, 118, 148, 0.12)",
            roads: "rgba(147, 180, 226, 0.34)",
            coast: "rgba(170, 203, 255, 0.56)",
            admin1: "rgba(125, 155, 198, 0.22)",
            rivers: "rgba(122, 174, 240, 0.36)",
            borders: "rgba(160, 194, 255, 0.82)",
            siteStroke: "rgba(244, 248, 255, 0.68)",
            apStroke: "rgba(244, 248, 255, 0.54)",
            popupBg: "rgba(11, 18, 30, 0.96)",
            popupBorder: "rgba(148, 163, 184, 0.3)",
        };
    }
    return {
        background: "#d8e5f1",
        land: "#eef4fb",
        water: "#ccdeee",
        marine: "rgba(194, 214, 234, 0.46)",
        terrain: "rgba(118, 136, 158, 0.08)",
        roads: "rgba(109, 135, 171, 0.28)",
        coast: "rgba(71, 101, 141, 0.44)",
        admin1: "rgba(77, 102, 135, 0.18)",
        rivers: "rgba(83, 128, 185, 0.30)",
        borders: "rgba(61, 90, 130, 0.72)",
        siteStroke: "rgba(15, 23, 42, 0.36)",
        apStroke: "rgba(15, 23, 42, 0.28)",
        popupBg: "rgba(255, 255, 255, 0.96)",
        popupBorder: "rgba(15, 23, 42, 0.12)",
    };
}

function buildMapStyle() {
    const palette = countryPalette();
    return {
        version: 8,
        sources: {},
        layers: [
            {
                id: "site-map-background",
                type: "background",
                paint: {
                    "background-color": palette.background,
                },
            },
        ],
    };
}

function qooClusterColorExpression() {
    return [
        "case",
        [">", ["get", "qooCount"], 0],
        [
            "step",
            ["/", ["get", "qooSum"], ["get", "qooCount"]],
            "#d94b5b",
            45, "#f5b84f",
            75, "#6ecc84",
            90, "#4ca4ff",
        ],
        "#8893a5",
    ];
}

function rttClusterColorExpression() {
    return [
        "case",
        [">", ["get", "rttCount"], 0],
        [
            "step",
            ["/", ["get", "rttSum"], ["get", "rttCount"]],
            "#61d26f",
            40, "#d7d86b",
            90, "#f0a44d",
            160, "#d94b5b",
        ],
        "#8893a5",
    ];
}

function clusterRadiusExpression() {
    return [
        "step",
        ["get", "throughputSum"],
        18,
        5_000_000, 22,
        25_000_000, 28,
        100_000_000, 36,
        500_000_000, 46,
        1_000_000_000, 56,
    ];
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
        this.initMap();
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

    initMap() {
        this.map = new window.maplibregl.Map({
            container: this.canvas,
            style: buildMapStyle(),
            center: INITIAL_CENTER,
            zoom: INITIAL_ZOOM,
            attributionControl: false,
            customAttribution: "Natural Earth, GRIP / GLOBIO (CC0)",
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

        this.map.on("load", () => {
            this.installSourcesAndLayers();
            this.installInteractions();
            this.applyTheme();
            this.renderFromHistory();
        });
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
        const palette = countryPalette();
        this.map.setPaintProperty("site-map-background", "background-color", palette.background);
        if (this.map.getLayer(COUNTRY_FILL_LAYER_ID)) {
            this.map.setPaintProperty(COUNTRY_FILL_LAYER_ID, "fill-color", palette.land);
        }
        if (this.map.getLayer(MARINE_AREAS_FILL_LAYER_ID)) {
            this.map.setPaintProperty(MARINE_AREAS_FILL_LAYER_ID, "fill-color", palette.marine);
        }
        if (this.map.getLayer(PHYSICAL_REGIONS_FILL_LAYER_ID)) {
            this.map.setPaintProperty(PHYSICAL_REGIONS_FILL_LAYER_ID, "fill-color", palette.terrain);
        }
        if (this.map.getLayer(PHYSICAL_REGIONS_LINE_LAYER_ID)) {
            this.map.setPaintProperty(PHYSICAL_REGIONS_LINE_LAYER_ID, "line-color", palette.terrain);
        }
        if (this.map.getLayer(PHYSICAL_REGIONS_10M_FILL_LAYER_ID)) {
            this.map.setPaintProperty(PHYSICAL_REGIONS_10M_FILL_LAYER_ID, "fill-color", palette.terrain);
        }
        if (this.map.getLayer(PHYSICAL_REGIONS_10M_LINE_LAYER_ID)) {
            this.map.setPaintProperty(PHYSICAL_REGIONS_10M_LINE_LAYER_ID, "line-color", palette.terrain);
        }
        if (this.map.getLayer(MAJOR_ROADS_10M_LAYER_ID)) {
            this.map.setPaintProperty(MAJOR_ROADS_10M_LAYER_ID, "line-color", palette.roads);
        }
        if (this.map.getLayer(GRIP_ROADS_LAYER_ID)) {
            this.map.setPaintProperty(GRIP_ROADS_LAYER_ID, "line-color", palette.roads);
        }
        if (this.map.getLayer(LAKES_FILL_LAYER_ID)) {
            this.map.setPaintProperty(LAKES_FILL_LAYER_ID, "fill-color", palette.water);
        }
        if (this.map.getLayer(RIVERS_LINE_LAYER_ID)) {
            this.map.setPaintProperty(RIVERS_LINE_LAYER_ID, "line-color", palette.rivers);
        }
        if (this.map.getLayer(ADMIN1_LINE_LAYER_ID)) {
            this.map.setPaintProperty(ADMIN1_LINE_LAYER_ID, "line-color", palette.admin1);
        }
        if (this.map.getLayer(COUNTRY_LINE_LAYER_ID)) {
            this.map.setPaintProperty(COUNTRY_LINE_LAYER_ID, "line-color", palette.borders);
        }
        if (this.map.getLayer(COASTLINE_LAYER_ID)) {
            this.map.setPaintProperty(COASTLINE_LAYER_ID, "line-color", palette.coast);
        }
        if (this.map.getLayer(SITE_POINTS_LAYER_ID)) {
            this.map.setPaintProperty(SITE_POINTS_LAYER_ID, "circle-stroke-color", palette.siteStroke);
        }
        if (this.map.getLayer(AP_POINTS_LAYER_ID)) {
            this.map.setPaintProperty(AP_POINTS_LAYER_ID, "circle-stroke-color", palette.apStroke);
        }
    }

    installSourcesAndLayers() {
        this.map.addSource(COUNTRIES_SOURCE_ID, {
            type: "geojson",
            data: COUNTRIES_GEOJSON_PATH,
        });
        this.map.addSource(MARINE_AREAS_SOURCE_ID, {
            type: "geojson",
            data: MARINE_AREAS_GEOJSON_PATH,
        });
        this.map.addSource(PHYSICAL_REGIONS_SOURCE_ID, {
            type: "geojson",
            data: PHYSICAL_REGIONS_GEOJSON_PATH,
        });
        this.map.addSource(PHYSICAL_REGIONS_10M_SOURCE_ID, {
            type: "geojson",
            data: PHYSICAL_REGIONS_10M_GEOJSON_PATH,
        });
        this.map.addSource(MAJOR_ROADS_10M_SOURCE_ID, {
            type: "geojson",
            data: MAJOR_ROADS_10M_GEOJSON_PATH,
        });
        this.map.addSource(GRIP_ROADS_SOURCE_ID, {
            type: "geojson",
            data: GRIP_ROADS_GEOJSON_PATH,
        });
        this.map.addSource(LAKES_SOURCE_ID, {
            type: "geojson",
            data: LAKES_GEOJSON_PATH,
        });
        this.map.addSource(RIVERS_SOURCE_ID, {
            type: "geojson",
            data: RIVERS_GEOJSON_PATH,
        });
        this.map.addSource(ADMIN1_SOURCE_ID, {
            type: "geojson",
            data: ADMIN1_BOUNDARIES_GEOJSON_PATH,
        });
        this.map.addSource(COASTLINE_SOURCE_ID, {
            type: "geojson",
            data: COASTLINES_GEOJSON_PATH,
        });
        this.map.addLayer({
            id: MARINE_AREAS_FILL_LAYER_ID,
            type: "fill",
            source: MARINE_AREAS_SOURCE_ID,
            paint: {
                "fill-color": countryPalette().marine,
                "fill-opacity": 0.9,
            },
        });
        this.map.addLayer({
            id: COUNTRY_FILL_LAYER_ID,
            type: "fill",
            source: COUNTRIES_SOURCE_ID,
            paint: {
                "fill-color": countryPalette().land,
                "fill-opacity": isDarkMode() ? 0.8 : 0.76,
            },
        });
        this.map.addLayer({
            id: PHYSICAL_REGIONS_FILL_LAYER_ID,
            type: "fill",
            source: PHYSICAL_REGIONS_SOURCE_ID,
            paint: {
                "fill-color": countryPalette().terrain,
                "fill-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    0,
                    [
                        "match", ["get", "featurecla"],
                        "mountain range", isDarkMode() ? 0.22 : 0.15,
                        "mountain", isDarkMode() ? 0.18 : 0.12,
                        "plateau", isDarkMode() ? 0.12 : 0.08,
                        "basin", isDarkMode() ? 0.09 : 0.06,
                        "plain", isDarkMode() ? 0.06 : 0.04,
                        "desert", isDarkMode() ? 0.08 : 0.05,
                        isDarkMode() ? 0.08 : 0.05,
                    ],
                    4.5,
                    [
                        "match", ["get", "featurecla"],
                        "mountain range", isDarkMode() ? 0.18 : 0.11,
                        "mountain", isDarkMode() ? 0.15 : 0.09,
                        "plateau", isDarkMode() ? 0.08 : 0.05,
                        "basin", isDarkMode() ? 0.06 : 0.04,
                        "plain", isDarkMode() ? 0.04 : 0.02,
                        "desert", isDarkMode() ? 0.05 : 0.03,
                        isDarkMode() ? 0.05 : 0.03,
                    ],
                    6,
                    0,
                ],
            },
        });
        this.map.addLayer({
            id: PHYSICAL_REGIONS_LINE_LAYER_ID,
            type: "line",
            source: PHYSICAL_REGIONS_SOURCE_ID,
            paint: {
                "line-color": countryPalette().terrain,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    2, 0.18,
                    5, 0.3,
                    8, 0.46,
                ],
                "line-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    0, 0.05,
                    4.5, 0.08,
                    6, 0.03,
                    7, 0,
                ],
            },
        });
        this.map.addLayer({
            id: PHYSICAL_REGIONS_10M_FILL_LAYER_ID,
            type: "fill",
            source: PHYSICAL_REGIONS_10M_SOURCE_ID,
            minzoom: 4.5,
            paint: {
                "fill-color": countryPalette().terrain,
                "fill-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    4.5,
                    0,
                    5.5,
                    [
                        "match", ["get", "featurecla"],
                        "mountain range", isDarkMode() ? 0.24 : 0.16,
                        "mountain", isDarkMode() ? 0.2 : 0.13,
                        "plateau", isDarkMode() ? 0.12 : 0.08,
                        "basin", isDarkMode() ? 0.09 : 0.06,
                        "plain", isDarkMode() ? 0.05 : 0.03,
                        "desert", isDarkMode() ? 0.07 : 0.05,
                        isDarkMode() ? 0.07 : 0.05,
                    ],
                    8,
                    [
                        "match", ["get", "featurecla"],
                        "mountain range", isDarkMode() ? 0.28 : 0.19,
                        "mountain", isDarkMode() ? 0.24 : 0.16,
                        "plateau", isDarkMode() ? 0.15 : 0.1,
                        "basin", isDarkMode() ? 0.11 : 0.08,
                        "plain", isDarkMode() ? 0.07 : 0.05,
                        "desert", isDarkMode() ? 0.09 : 0.06,
                        isDarkMode() ? 0.09 : 0.06,
                    ],
                ],
            },
        });
        this.map.addLayer({
            id: PHYSICAL_REGIONS_10M_LINE_LAYER_ID,
            type: "line",
            source: PHYSICAL_REGIONS_10M_SOURCE_ID,
            minzoom: 4.5,
            paint: {
                "line-color": countryPalette().terrain,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    4.5, 0.18,
                    6, 0.34,
                    8, 0.5,
                ],
                "line-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    4.5, 0,
                    5.5, 0.05,
                    8, 0.1,
                ],
            },
        });
        this.map.addLayer({
            id: MAJOR_ROADS_10M_LAYER_ID,
            type: "line",
            source: MAJOR_ROADS_10M_SOURCE_ID,
            minzoom: 5.5,
            paint: {
                "line-color": countryPalette().roads,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    5.5, 0.3,
                    7, 0.55,
                    9, 0.95,
                ],
                "line-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    5.5, 0,
                    6.5, isDarkMode() ? 0.18 : 0.14,
                    9, isDarkMode() ? 0.28 : 0.22,
                ],
            },
        });
        this.map.addLayer({
            id: GRIP_ROADS_LAYER_ID,
            type: "line",
            source: GRIP_ROADS_SOURCE_ID,
            minzoom: 5.5,
            paint: {
                "line-color": countryPalette().roads,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    5.5, 0.4,
                    7.5, 0.8,
                    10, 1.35,
                ],
                "line-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    5.5, 0,
                    6.25, isDarkMode() ? 0.18 : 0.14,
                    8, isDarkMode() ? 0.3 : 0.24,
                    10, isDarkMode() ? 0.42 : 0.34,
                ],
            },
        });
        this.map.setPaintProperty(MAJOR_ROADS_10M_LAYER_ID, "line-opacity", [
            "interpolate", ["linear"], ["zoom"],
            0, 0,
            4.5, isDarkMode() ? 0.1 : 0.08,
            5.5, isDarkMode() ? 0.14 : 0.1,
            6.25, 0,
        ]);
        this.map.addLayer({
            id: LAKES_FILL_LAYER_ID,
            type: "fill",
            source: LAKES_SOURCE_ID,
            paint: {
                "fill-color": countryPalette().water,
                "fill-opacity": 0.92,
            },
        });
        this.map.addLayer({
            id: RIVERS_LINE_LAYER_ID,
            type: "line",
            source: RIVERS_SOURCE_ID,
            paint: {
                "line-color": countryPalette().rivers,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    2, 0.22,
                    5, 0.4,
                    8, 0.62,
                ],
                "line-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    2, 0.42,
                    5, 0.34,
                    8, 0.28,
                ],
            },
        });
        this.map.addLayer({
            id: ADMIN1_LINE_LAYER_ID,
            type: "line",
            source: ADMIN1_SOURCE_ID,
            paint: {
                "line-color": countryPalette().admin1,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    2, 0.34,
                    5, 0.58,
                    8, 0.95,
                ],
                "line-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    0, 0.3,
                    3, 0.46,
                    5.5, 0.6,
                    7, 0.52,
                    9, 0.42,
                ],
            },
        });
        this.map.addLayer({
            id: COUNTRY_LINE_LAYER_ID,
            type: "line",
            source: COUNTRIES_SOURCE_ID,
            paint: {
                "line-color": countryPalette().borders,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    2, 0.5,
                    5, 0.8,
                    8, 1.2,
                ],
                "line-opacity": 0.9,
            },
        });
        this.map.addLayer({
            id: COASTLINE_LAYER_ID,
            type: "line",
            source: COASTLINE_SOURCE_ID,
            paint: {
                "line-color": countryPalette().coast,
                "line-width": [
                    "interpolate", ["linear"], ["zoom"],
                    2, 0.45,
                    5, 0.8,
                    8, 1.25,
                ],
                "line-opacity": 0.82,
            },
        });

        this.map.addSource(SITE_SOURCE_ID, {
            type: "geojson",
            data: { type: "FeatureCollection", features: [] },
            cluster: true,
            clusterRadius: 42,
            clusterMaxZoom: 7,
            clusterProperties: {
                throughputSum: ["+", ["get", "throughputCombined"]],
                qooSum: ["+", ["coalesce", ["get", "qooWorst"], 0]],
                qooCount: ["+", ["case", ["has", "qooWorst"], 1, 0]],
                rttSum: ["+", ["coalesce", ["get", "rttWorst"], 0]],
                rttCount: ["+", ["case", ["has", "rttWorst"], 1, 0]],
            },
        });
        this.map.addSource(AP_SOURCE_ID, {
            type: "geojson",
            data: { type: "FeatureCollection", features: [] },
            cluster: true,
            clusterRadius: 40,
            clusterMaxZoom: 11,
            clusterProperties: {
                throughputSum: ["+", ["get", "throughputCombined"]],
                qooSum: ["+", ["coalesce", ["get", "qooWorst"], 0]],
                qooCount: ["+", ["case", ["has", "qooWorst"], 1, 0]],
                rttSum: ["+", ["coalesce", ["get", "rttWorst"], 0]],
                rttCount: ["+", ["case", ["has", "rttWorst"], 1, 0]],
            },
        });

        this.map.addLayer({
            id: SITE_CLUSTER_LAYER_ID,
            type: "circle",
            source: SITE_SOURCE_ID,
            filter: ["has", "point_count"],
            paint: {
                "circle-color": qooClusterColorExpression(),
                "circle-radius": clusterRadiusExpression(),
                "circle-opacity": 0.88,
                "circle-stroke-color": countryPalette().siteStroke,
                "circle-stroke-width": 1.1,
            },
        });
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
                    0, 0.92,
                    5, 0.62,
                    8, 0.18,
                ],
                "circle-stroke-color": countryPalette().siteStroke,
                "circle-stroke-width": 1.0,
                "circle-blur": 0.08,
            },
        });

        this.map.addLayer({
            id: AP_CLUSTER_LAYER_ID,
            type: "circle",
            source: AP_SOURCE_ID,
            filter: ["has", "point_count"],
            paint: {
                "circle-color": qooClusterColorExpression(),
                "circle-radius": clusterRadiusExpression(),
                "circle-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    0, 0.18,
                    4, 0.44,
                    6, 0.82,
                    8, 0.9,
                ],
                "circle-stroke-color": countryPalette().apStroke,
                "circle-stroke-width": 1.0,
            },
        });
        this.map.addLayer({
            id: AP_POINTS_LAYER_ID,
            type: "circle",
            source: AP_SOURCE_ID,
            filter: ["!", ["has", "point_count"]],
            paint: {
                "circle-color": ["get", "metricColor"],
                "circle-radius": ["get", "markerRadius"],
                "circle-opacity": [
                    "interpolate", ["linear"], ["zoom"],
                    0, 0.06,
                    4, 0.18,
                    6, 0.76,
                    8, 0.96,
                ],
                "circle-stroke-color": countryPalette().apStroke,
                "circle-stroke-width": 1.0,
                "circle-blur": 0.05,
            },
        });
    }

    installInteractions() {
        const pointLayers = [SITE_POINTS_LAYER_ID, AP_POINTS_LAYER_ID];
        const clusterLayers = [SITE_CLUSTER_LAYER_ID, AP_CLUSTER_LAYER_ID];

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

        clusterLayers.forEach((layerId) => {
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
                const props = feature.properties || {};
                const count = toNumber(props.point_count, 0);
                const typeLabel = layerId === SITE_CLUSTER_LAYER_ID ? "Site" : "AP";
                this.popup
                    .setLngLat(event.lngLat)
                    .setHTML(`
                        <div class="small">
                            <div class="fw-semibold">${typeLabel} cluster</div>
                            <div class="text-muted">${count} grouped nodes at this zoom level.</div>
                        </div>`)
                    .addTo(this.map);
            });
            this.map.on("click", layerId, async (event) => {
                const feature = event.features?.[0];
                if (!feature) return;
                const sourceId = layerId === SITE_CLUSTER_LAYER_ID ? SITE_SOURCE_ID : AP_SOURCE_ID;
                const clusterId = feature.properties?.cluster_id;
                const source = this.map.getSource(sourceId);
                if (!source || clusterId === undefined || clusterId === null) return;
                source.getClusterExpansionZoom(clusterId, (err, zoom) => {
                    if (err) return;
                    this.map.easeTo({
                        center: feature.geometry.coordinates,
                        zoom,
                        duration: 400,
                    });
                });
            });
        });
    }

    requestInitialTree() {
        this.setStatus("Waiting for data", "spinner");
        listenOnceWithTimeout("NetworkTree", INITIAL_REQUEST_TIMEOUT_MS, (msg) => {
            this.processTreeMessage(msg);
            if (!this.subscription) {
                this.subscription = subscribeWS(["NetworkTree"], (liveMsg) => {
                    if (liveMsg.event === "NetworkTree") {
                        this.processTreeMessage(liveMsg);
                    }
                });
            }
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
        this.renderFromHistory();
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
                latitude: Number.isFinite(node.latitude) ? node.latitude : null,
                longitude: Number.isFinite(node.longitude) ? node.longitude : null,
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

        const siteFeatures = features.filter((feature) => feature.properties.nodeType === "site");
        const apFeatures = features.filter((feature) => feature.properties.nodeType === "ap");
        return {
            siteFeatures,
            apFeatures,
            unmappedSites,
            unmappedAps,
            maxBitsPerSecond,
        };
    }

    renderFromHistory() {
        if (!this.map || !this.map.isStyleLoaded()) {
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
        if (!siteSource || !apSource) {
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

        const sparseApCoverage = aggregate.apFeatures.length < 24;
        const siteOpacity = sparseApCoverage
            ? ["interpolate", ["linear"], ["zoom"], 0, 0.92, 5, 0.7, 8, 0.42]
            : ["interpolate", ["linear"], ["zoom"], 0, 0.92, 5, 0.54, 7, 0.12, 8, 0.02];
        const siteClusterOpacity = sparseApCoverage
            ? ["interpolate", ["linear"], ["zoom"], 0, 0.9, 5, 0.76, 8, 0.38]
            : ["interpolate", ["linear"], ["zoom"], 0, 0.9, 5, 0.6, 7, 0.2, 8, 0.06];

        this.map.setPaintProperty(SITE_POINTS_LAYER_ID, "circle-opacity", siteOpacity);
        this.map.setPaintProperty(SITE_CLUSTER_LAYER_ID, "circle-opacity", siteClusterOpacity);

        const clusterColor = this.mode === "qoo" ? qooClusterColorExpression() : rttClusterColorExpression();
        this.map.setPaintProperty(SITE_CLUSTER_LAYER_ID, "circle-color", clusterColor);
        this.map.setPaintProperty(AP_CLUSTER_LAYER_ID, "circle-color", clusterColor);
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
