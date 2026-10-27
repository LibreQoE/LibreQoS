import { isColorBlindMode } from "./helpers/colorblind";
import { isDarkMode } from "./helpers/dark_mode";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client } from "./pubsub/ws";
import {
    R6_FILL_ALTITUDE,
    R6_FILL_MARGIN,
    buildHexLattice,
    mapEndpoints,
    buildHexGeometry,
    hexbinPointsFromRows,
    packLayerColors,
    parseColorValue,
    r6CellFillRgb,
    rampStops,
    rgbCss,
    rgbUnit,
    rttPercentileDomain,
    tweenAlpha,
    mixColors,
    shouldRefreshHud,
    latticeCacheKey,
    layerUpdatePlan,
} from "./flow_globe_hexbins.mjs";
import {
    colorAlpha,
    geometryPolygons,
    normalizePolygonRings,
    subdivideTriangleToMaxEdge,
    unwrapRing,
} from "./flow_globe_countries.mjs";

const wsClient = get_ws_client();

const AUTO_REFRESH_MS = 5000;
// `.50.4` Lever 1: on the auto path the heavier legend/stats repaint is
// throttled; only the cheap header age text ticks every second (`tickAgeLabel`).
const HUD_THROTTLE_MS = 5000;
// `.50.4` Lever 2 / `.50.6`: time-based colour tween, alpha = 1 - exp(-elapsed/tau).
// tau 1.67 s => alpha = 1 - exp(-5/1.67) = 95.0 % at the 5 s auto cadence, so the
// colour never fully settles between updates (continuous drift) — DRIVER ruling.
const TWEEN_TAU_S = 1.67;
// `.50.7` Stage 2: country-cap subdivision (each earcut triangle -> 4^level) so
// the merged cap hugs the sphere (the curvature-6 look).
const COUNTRY_CAP_MAX_EDGE_DEG = 3;
// `.50.7`: the country border sits this far above the land cap (altitude
// separation — cap < border < R6 fill < R6 border).
const COUNTRY_BORDER_ALTITUDE_STEP = 0.0005;
const RESPONSE_TIMEOUT_MS = 2000;
const MIN_POINTS = 3;
const MIN_TOTAL_BYTES = 1_000_000;
// The Flow Globe is fixed at the top-N endpoints by throughput (the DRIVER
// retired the Density dropdown): Filled, Points, the header and the inspector
// all describe the same set.
const FLOW_GLOBE_MAX_ENDPOINTS = 5000;
const CLUSTER_PREVIEW_LIMIT = 3;
const COUNTRIES_GEOJSON_PATH = "vendor/countries.geojson";
const FLOW_MAP_PROFILE = false;
const DEFAULT_POINT_OF_VIEW = {
    lat: 18,
    lng: -32,
    altitude: 2.15,
};
const MIN_INITIAL_ALTITUDE = 1.05;
const MAX_INITIAL_ALTITUDE = 2.05;
const MIN_ZOOM_SPEED = 1.8;
const MAX_ZOOM_SPEED = 3.4;
// Flow Globe honeycomb (bead lqos-74t.50, card docs/flow_globe_hexbin_card.md).
// The Filled heatmap is a hexasphere custom layer; the maths live in the pure
// `flow_globe_hexbins.mjs` (nearest-endpoint Internet RTT colour on an adaptive
// p5 + manual-top ramp, per-cell alpha). The arcs/blobs are retired (WG1).
const FLOW_MAP_DEBUG = (() => {
    const params = new URLSearchParams(window.location.search);
    const disabled = new Set(
        (params.get("flowMapDisable") || "")
            .split(",")
            .map((value) => value.trim().toLowerCase())
            .filter(Boolean)
    );
    return {
        disableBorders: disabled.has("borders"),
        disableClusterBadges: disabled.has("badges"),
    };
})();

function profileStart(label) {
    return FLOW_MAP_PROFILE ? { label, started: performance.now() } : null;
}

function profileEnd(handle, extras = "") {
    if (!handle) {
        return;
    }
    const elapsed = performance.now() - handle.started;
    console.debug(`[FlowMap profile] ${handle.label}: ${elapsed.toFixed(1)}ms${extras ? ` ${extras}` : ""}`);
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
    return {
        cancel: () => {
            wsClient.off(eventName, wrapped);
            clearTimeout(timer);
        }
    };
}

function makeOverlay(container, id) {
    container.style.position = "relative";

    const overlay = document.createElement("div");
    overlay.id = id;
    overlay.style.position = "absolute";
    overlay.style.inset = "0";
    overlay.style.display = "none";
    overlay.style.alignItems = "center";
    overlay.style.justifyContent = "center";
    overlay.style.pointerEvents = "none";
    overlay.style.zIndex = "11";
    overlay.style.padding = "16px";

    const panel = document.createElement("div");
    panel.style.background = "color-mix(in srgb, var(--lqos-surface-solid) 88%, transparent)";
    panel.style.border = "1px solid var(--lqos-border)";
    panel.style.borderRadius = "var(--lqos-radius-lg)";
    panel.style.boxShadow = "var(--lqos-shadow-md)";
    panel.style.padding = "16px 20px";
    panel.style.maxWidth = "560px";
    panel.style.textAlign = "center";
    panel.style.backdropFilter = "blur(12px)";
    panel.style.webkitBackdropFilter = "blur(12px)";

    const title = document.createElement("div");
    title.style.fontWeight = "700";
    title.style.fontSize = "1.1rem";

    const subtitle = document.createElement("div");
    subtitle.className = "text-muted";
    subtitle.style.marginTop = "6px";

    panel.appendChild(title);
    panel.appendChild(subtitle);
    overlay.appendChild(panel);
    container.appendChild(overlay);

    return {
        show: (heading, subheading) => {
            title.textContent = heading;
            subtitle.textContent = subheading || "";
            overlay.style.display = "flex";
        },
        hide: () => {
            overlay.style.display = "none";
        },
    };
}

// The globe body colour (ocean) is what a hexagon sits over in the empty state;
// the "interpolated" and "no-data" looks blend toward it (R3: no per-vertex
// alpha in the basic materials, so the dim is baked into the RGB).
function backdropRgb() {
    return parseColorValue(getThemePalette().oceanColor);
}

function getThemePalette() {
    if (isDarkMode()) {
        return {
            atmosphereColor: "#4d7fda",
            atmosphereAltitude: 0.05,
            oceanColor: "#0b1730",
            emissiveTint: "#09111d",
            emissiveIntensity: 0.18,
            landFill: "rgba(111, 157, 230, 0.18)",
            landSide: "rgba(24, 42, 79, 0.16)",
            countryBorder: "rgba(164, 196, 255, 0.74)",
            countryAltitude: 0.004,
            legendGradient: "linear-gradient(90deg, #56b6ff 0%, #7cffb2 40%, #fddd60 70%, #ff6e76 100%)",
            trafficGradient: "linear-gradient(90deg, #21456f 0%, #2b7fff 40%, #56d4ff 72%, #7cffb2 100%)",
            tooltipBg: "rgba(11, 18, 32, 0.96)",
            tooltipBorder: "rgba(148, 163, 184, 0.28)",
            tooltipText: "#edf4ff",
            selectedColor: "#ffffff",
            terminalColor: "rgba(148, 163, 184, 0.9)",
            hexBorderRgb: "226, 232, 240",
            hexNeutralRgb: "148, 163, 184",
        };
    }

    return {
        atmosphereColor: "#90aed8",
        atmosphereAltitude: 0.035,
        oceanColor: "#dce9f8",
        emissiveTint: "#ffffff",
        emissiveIntensity: 0.04,
        landFill: "rgba(93, 128, 177, 0.16)",
        landSide: "rgba(118, 145, 184, 0.12)",
        countryBorder: "rgba(51, 86, 137, 0.58)",
        countryAltitude: 0.0035,
        legendGradient: "linear-gradient(90deg, #61a0a8 0%, #6fbf73 38%, #f2c14e 70%, #d96c75 100%)",
        trafficGradient: "linear-gradient(90deg, #b6cadf 0%, #61a0a8 40%, #4c8bf5 72%, #2563eb 100%)",
        tooltipBg: "rgba(255, 255, 255, 0.97)",
        tooltipBorder: "rgba(15, 23, 42, 0.12)",
        tooltipText: "#1f2937",
        selectedColor: "#0f172a",
        terminalColor: "rgba(71, 85, 105, 0.85)",
        hexBorderRgb: "15, 23, 42",
        hexNeutralRgb: "100, 116, 139",
    };
}

function clusterRadiusPx(altitude) {
    if (!Number.isFinite(altitude)) {
        return 36;
    }
    return Math.max(22, Math.min(44, 18 + (altitude * 9.2)));
}

function shortestLngDelta(a, b) {
    let delta = Math.abs(a - b);
    if (delta > 180) {
        delta = 360 - delta;
    }
    return delta;
}

function latLngToUnitVector(lat, lng) {
    const latRad = (lat * Math.PI) / 180;
    const lngRad = (lng * Math.PI) / 180;
    const cosLat = Math.cos(latRad);
    return {
        x: cosLat * Math.sin(lngRad),
        y: Math.sin(latRad),
        z: cosLat * Math.cos(lngRad),
    };
}

function normalizeVector(x, y, z) {
    const length = Math.hypot(x, y, z) || 1;
    return {
        x: x / length,
        y: y / length,
        z: z / length,
    };
}

function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
}

function unitVectorToLatLng(vector) {
    const lat = Math.asin(clamp(vector.y, -1, 1)) * (180 / Math.PI);
    const lng = Math.atan2(vector.x, vector.z) * (180 / Math.PI);
    return { lat, lng };
}

function dotProduct(a, b) {
    return (a.x * b.x) + (a.y * b.y) + (a.z * b.z);
}

function angularDistanceDegrees(a, b) {
    return Math.acos(clamp(dotProduct(a, b), -1, 1)) * (180 / Math.PI);
}

function formatAge(timestamp) {
    if (!timestamp) {
        return "Waiting for data";
    }
    const deltaMs = Math.max(0, Date.now() - timestamp);
    if (deltaMs < 1000) {
        return "Just now";
    }
    if (deltaMs < 60_000) {
        return `${Math.round(deltaMs / 1000)}s ago`;
    }
    return `${Math.round(deltaMs / 60_000)}m ago`;
}

function updateHud(summary) {
    const sampleCount = document.getElementById("flowMapSampleCount");
    const lastUpdated = document.getElementById("flowMapLastUpdated");
    if (sampleCount) {
        sampleCount.textContent = `${summary.pointCount} endpoints`;
    }
    if (lastUpdated) {
        lastUpdated.textContent = summary.lastUpdatedLabel || "Waiting for data";
    }
}

function updateLegend(summary, domain) {
    const gradient = document.getElementById("flowMapLegendGradient");
    const title = document.getElementById("flowMapLegendTitle");
    const min = document.getElementById("flowMapLegendMin");
    const max = document.getElementById("flowMapLegendMax");
    const status = document.getElementById("flowMapLegendStatus");

    if (gradient) {
        const stops = rampStops(isColorBlindMode()).map((rgb) => rgbCss(rgb));
        gradient.style.background = `linear-gradient(90deg, ${stops.join(", ")})`;
    }
    if (title) {
        title.textContent = "Internet latency (nearest endpoint)";
    }
    if (min) {
        min.textContent = domain ? `${Math.round(domain.p5)} ms` : "Fast";
    }
    if (max) {
        max.textContent = domain ? `${Math.round(domain.p95)} ms` : "Slow";
    }
    if (status) {
        const total = summary.totalBytes > 0 ? scaleNumber(summary.totalBytes) : "0";
        status.textContent = `${summary.pointCount} endpoints, ${total}B recent volume`;
    }
}

function renderCellBody(cell) {
    const rtt = cell.rttInternetMs > 0
        ? `${cell.rttInternetMs.toFixed(cell.rttInternetMs >= 100 ? 0 : 1)} ms`
        : "Unknown";
    const kind = cell.measured
        ? "Measured"
        : (cell.covered ? "Interpolated (modeled)" : "No coverage");
    return [
        ["Internet RTT", rtt],
        ["Cell", kind],
        // `contributing` is the TRUE count (members is capped for memory).
        ["Locations", `${cell.contributing || 0}`],
        ["Source distance", cell.measured || !Number.isFinite(cell.sourceDistKm)
            ? "—"
            : `${Math.round(cell.sourceDistKm)} km`],
        ["Throughput", `${scaleNumber(cell.bytes || 0)}B`],
        ["Latitude", cell.lat.toFixed(2)],
        ["Longitude", cell.lng.toFixed(2)],
    ];
}

function escapeHtml(value) {
    return String(value)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll("\"", "&quot;")
        .replaceAll("'", "&#39;");
}

class FlowMapGlobe {
    constructor(id) {
        this.dom = document.getElementById(id);
        if (!this.dom) {
            throw new Error(`FlowMap: missing DOM element '${id}'`);
        }
        if (typeof window.Globe !== "function") {
            throw new Error("FlowMap: missing vendored globe libraries");
        }
        // `.50.3`: the per-instance geometry-cache project identity (a remount
        // creates a new instance with its own per-layer lattice).
        this.projectId = id;

        this.rawPayload = [];
        this.countryFeatures = [];
        this.renderedSummary = {
            pointCount: 0,
            totalBytes: 0,
            maxBytes: 0,
            lastUpdatedLabel: "Waiting for data",
        };
        this.autoRotate = true;
        this.autoRefresh = true;
        this.userInteracting = false;
        this.interactionTimer = null;
        this.reclusterTimer = null;
        this.themeObserver = null;
        this.lastKnownAltitude = DEFAULT_POINT_OF_VIEW.altitude;
        this.activeClusterRadius = clusterRadiusPx(DEFAULT_POINT_OF_VIEW.altitude);
        this.initialPointOfView = { ...DEFAULT_POINT_OF_VIEW };
        this.hasAutoCenteredOnce = false;
        this.hexMaxWeight = 0;
        // The R6 merged-geometry custom layer (Filled). `customLayer` holds the
        // live THREE objects + the face->cell table for hover; disposed/rebuilt
        // when the endpoint content hash or theme changes.
        this.customLayer = null;
        this.rttDomain = { p5: 0, p95: 1 };
        this.layerDomainP5 = 0;
        // Hexasphere subdivision frequency: cell count = 10*f^2 + 2 (f=32 -> 10242).
        this.hexFrequency = (() => {
            const raw = parseInt(new URLSearchParams(window.location.search).get("flowGlobeHexFreq"), 10);
            return Number.isFinite(raw) && raw >= 6 && raw <= 64 ? raw : 32;
        })();
        // The manual RTT (ms) that maps to the red end of the ramp; the blue end
        // tracks the data's p5.
        this.rttTopMs = (() => {
            const raw = parseInt(new URLSearchParams(window.location.search).get("flowGlobeRttMax"), 10);
            return Number.isFinite(raw) && raw >= 50 && raw <= 400 ? raw : 200;
        })();
        // `.50.8`: the K-th-neighbour halo knobs (URL-tunable while evaluating).
        // K = neighbour rank, c = scale; floor/cap clamp the radius (km).
        const haloParams = new URLSearchParams(window.location.search);
        this.haloK = (() => {
            const raw = parseInt(haloParams.get("flowGlobeHaloK"), 10);
            return Number.isFinite(raw) && raw >= 1 && raw <= 50 ? raw : 5;
        })();
        this.haloC = (() => {
            const raw = parseFloat(haloParams.get("flowGlobeHaloC"));
            return Number.isFinite(raw) && raw > 0 && raw <= 10 ? raw : 1.5;
        })();
        this.haloFloorKm = (() => {
            const raw = parseInt(haloParams.get("flowGlobeHaloFloor"), 10);
            return Number.isFinite(raw) && raw >= 0 && raw <= 5000 ? raw : 300;
        })();
        this.haloCapKm = (() => {
            const raw = parseInt(haloParams.get("flowGlobeHaloCap"), 10);
            return Number.isFinite(raw) && raw >= 100 && raw <= 20000 ? raw : 2500;
        })();
        this.hoverRaf = null;
        this.tweenRaf = null;
        // `.50.5`: the tick `performance.now()` the GPU-tween alpha is measured
        // from (null => set on the first frame; the glide restarts per tick).
        this.tweenStartAt = null;
        // The GPU-tween uniform shared with the injected MeshBasicMaterial.
        this.tweenUniform = { value: 1 };
        // `.50.5` Lever 3: pause the rAF when off-screen / unfocused.
        this.inViewport = true;
        this.hasFocus = true;
        this.hexSizeTimer = null;
        this.hoverEvent = null;
        this.hoverCell = null;
        this.pinnedCell = null;
        this.pinnedCellPos = null;
        this.pointerDownAt = null;
        this.tooltip = document.getElementById("flowMapTooltip");
        this.countryLayer = null;

        this.onColorBlindChanged = () => {
            this.applyLayerColors();
            refreshHud();
        };
        window.addEventListener("colorBlindModeChanged", this.onColorBlindChanged);

        // `.50.5` Lever 3: pause the rAF when the canvas is off-screen or the
        // window is unfocused; resume on return.
        this.onWindowBlur = () => {
            this.hasFocus = false;
            this.stopTween();
        };
        this.onWindowFocus = () => {
            this.hasFocus = true;
            this.resumeTween();
        };
        window.addEventListener("blur", this.onWindowBlur);
        window.addEventListener("focus", this.onWindowFocus);
        this.inViewportObserver = (typeof IntersectionObserver === "function")
            ? new IntersectionObserver((entries) => {
                const entry = entries[entries.length - 1];
                this.inViewport = entry ? entry.isIntersecting : true;
                if (this.inViewport) {
                    this.resumeTween();
                } else {
                    this.stopTween();
                }
            }, { threshold: 0 })
            : null;
        if (this.inViewportObserver) {
            this.inViewportObserver.observe(this.dom);
        }

        this.initGlobe();
        this.observeThemeChanges();
        void this.loadCountries();
    }

    initGlobe() {
        // `.50.5` Lever 2: pass the renderer config at construction (the vendored
        // kapsule form is `Globe(config)(dom)`, where `stateInit` forwards
        // `rendererConfig` to `new WebGLRenderer`; the default is
        // `{ antialias: true, alpha: true }`).
        this.globe = window.Globe({
            rendererConfig: {
                antialias: false,
                alpha: true,
                powerPreference: "high-performance",
            },
        })(this.dom)
            .width(this.dom.clientWidth || 960)
            .height(this.dom.clientHeight || 640)
            .backgroundColor("rgba(0,0,0,0)")
            .showAtmosphere(true)
            // `.50.7` Stage 2: the country layer is now a custom merged cap Mesh +
            // border LineSegments (see `applyCountryLayer`), NOT globe.gl's
            // per-polygon layer (which built ~1430 side/stroke draw calls).
            .polygonsData([]);

        // `.50.5` Lever 2: globe.gl defaults the pixel ratio to
        // `min(2, devicePixelRatio)`; cap it at 1.5 to cut the fill cost on HiDPI.
        const renderer = this.globe.renderer?.();
        if (renderer?.setPixelRatio) {
            renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.5));
        }

        this.controls = this.globe.controls();
        this.controls.enablePan = false;
        this.controls.enableDamping = false;
        this.controls.rotateSpeed = 0.75;
        this.controls.minDistance = 120;
        this.controls.maxDistance = 420;
        this.controls.autoRotate = this.autoRotate;
        this.controls.autoRotateSpeed = 0.45;

        this.globe.pointOfView(this.initialPointOfView, 0);
        this.captureViewState();
        this.syncControlSpeeds();
        this.applyTheme();
        this.attachControlListeners();
        this.attachHoverListeners();
    }

    async loadCountries() {
        try {
            const response = await fetch(COUNTRIES_GEOJSON_PATH, { cache: "force-cache" });
            if (!response.ok) {
                throw new Error(`HTTP ${response.status}`);
            }
            const geojson = await response.json();
            this.countryFeatures = Array.isArray(geojson?.features)
                ? geojson.features.filter((feature) => feature?.geometry)
                : [];
            this.applyCountryLayer();
        } catch (error) {
            console.error("FlowMap: failed to load country borders", error);
        }
    }

    // `.50.7` Stage 2: ONE merged cap Mesh + ONE merged border LineSegments built
    // from `countryFeatures` (not globe.gl's per-polygon layer, which built ~1430
    // side/stroke draw calls). Pure ring helpers live in `flow_globe_countries.mjs`.
    applyCountryLayer() {
        this.disposeCountryLayer();
        const THREE = window.THREE;
        if (!THREE || FLOW_MAP_DEBUG.disableBorders || this.countryFeatures.length === 0) {
            return;
        }
        const theme = getThemePalette();
        const project = (lat, lng, alt) => (
            typeof this.globe.getCoords === "function" ? this.globe.getCoords(lat, lng, alt) : null
        );
        const capAlt = theme.countryAltitude;
        const borderAlt = theme.countryAltitude + COUNTRY_BORDER_ALTITUDE_STEP;
        const capPositions = [];
        const borderPositions = [];
        for (const feature of this.countryFeatures) {
            for (const polygon of geometryPolygons(feature?.geometry)) {
                const rings = normalizePolygonRings(polygon);
                const outer = unwrapRing(rings[0]);
                if (!outer) {
                    continue;
                }
                const holes = [];
                for (let i = 1; i < rings.length; i += 1) {
                    const hole = unwrapRing(rings[i]);
                    if (hole) {
                        holes.push(hole);
                    }
                }
                // Border edges (outer + holes).
                for (const ring of [outer, ...holes]) {
                    for (let i = 0; i < ring.length; i += 1) {
                        const a = ring[i];
                        const b = ring[(i + 1) % ring.length];
                        const pa = project(a[1], a[0], borderAlt);
                        const pb = project(b[1], b[0], borderAlt);
                        if (pa && pb) {
                            borderPositions.push(pa.x, pa.y, pa.z, pb.x, pb.y, pb.z);
                        }
                    }
                }
                // Cap triangles: normalize winding, triangulate, subdivide, project.
                const points = [...outer, ...holes.flat()];
                const contour = outer.map(([lng, lat]) => new THREE.Vector2(lng, lat));
                const holePaths = holes.map((hole) => hole.map(([lng, lat]) => new THREE.Vector2(lng, lat)));
                const faces = THREE.ShapeUtils.triangulateShape(contour, holePaths);
                for (const face of faces) {
                    const tris = subdivideTriangleToMaxEdge(
                        [points[face[0]], points[face[1]], points[face[2]]],
                        COUNTRY_CAP_MAX_EDGE_DEG,
                    );
                    for (const [p0, p1, p2] of tris) {
                        const a = project(p0[1], p0[0], capAlt);
                        const b = project(p1[1], p1[0], capAlt);
                        const c = project(p2[1], p2[0], capAlt);
                        if (a && b && c) {
                            capPositions.push(a.x, a.y, a.z, b.x, b.y, b.z, c.x, c.y, c.z);
                        }
                    }
                }
            }
        }
        if (capPositions.length === 0 && borderPositions.length === 0) {
            return;
        }
        const capGeometry = new THREE.BufferGeometry();
        capGeometry.setAttribute("position", new THREE.BufferAttribute(Float32Array.from(capPositions), 3));
        const capMaterial = new THREE.MeshBasicMaterial({
            color: theme.landFill,
            transparent: true,
            opacity: colorAlpha(theme.landFill, 0.18),
            side: THREE.DoubleSide,
            depthWrite: false,
        });
        const capMesh = new THREE.Mesh(capGeometry, capMaterial);
        capMesh.renderOrder = 1;
        this.globe.scene().add(capMesh);

        const borderGeometry = new THREE.BufferGeometry();
        borderGeometry.setAttribute("position", new THREE.BufferAttribute(Float32Array.from(borderPositions), 3));
        const borderMaterial = new THREE.LineBasicMaterial({
            color: theme.countryBorder,
            transparent: true,
            opacity: colorAlpha(theme.countryBorder, 1),
        });
        const borderLines = new THREE.LineSegments(borderGeometry, borderMaterial);
        borderLines.renderOrder = 1;
        this.globe.scene().add(borderLines);

        this.countryLayer = { capMesh, capMaterial, borderLines, borderMaterial };
    }

    disposeCountryLayer() {
        const layer = this.countryLayer;
        if (!layer) {
            return;
        }
        const scene = this.globe?.scene?.();
        for (const object of [layer.capMesh, layer.borderLines]) {
            if (!object) {
                continue;
            }
            scene?.remove?.(object);
            object.geometry?.dispose?.();
            object.material?.dispose?.();
        }
        this.countryLayer = null;
    }

    attachControlListeners() {
        if (!this.controls?.addEventListener) {
            return;
        }
        this.controls.addEventListener("start", () => {
            this.userInteracting = true;
            this.syncControlSpeeds();
            clearScheduledRefresh();
            this.scheduleInteractionRelease();
        });
        this.controls.addEventListener("change", () => {
            if (FPS_METER_ENABLED) {
                perfControlsChanges += 1;
            }
            this.captureViewState();
            this.syncControlSpeeds();
            if (this.rawPayload.length > 0 && this.updateClusterRadius()) {
                this.scheduleRecluster();
            }
            if (this.userInteracting) {
                this.scheduleInteractionRelease();
            }
        });
        this.controls.addEventListener("end", () => {
            this.scheduleInteractionRelease();
        });
    }

    attachHoverListeners() {
        if (!this.dom?.addEventListener) {
            return;
        }
        if (!this.onPointerMoveBound) {
            this.onPointerMoveBound = (event) => this.onPointerMove(event);
            this.onPointerLeaveBound = () => this.hideCellTooltip();
            this.onPointerDownBound = (event) => this.onPointerDown(event);
            this.onPointerUpBound = (event) => this.onPointerUp(event);
        }
        this.dom.addEventListener("pointermove", this.onPointerMoveBound);
        this.dom.addEventListener("pointerleave", this.onPointerLeaveBound);
        this.dom.addEventListener("pointerdown", this.onPointerDownBound);
        this.dom.addEventListener("pointerup", this.onPointerUpBound);
    }

    detachHoverListeners() {
        if (!this.dom?.removeEventListener) {
            return;
        }
        if (this.onPointerMoveBound) {
            this.dom.removeEventListener("pointermove", this.onPointerMoveBound);
        }
        if (this.onPointerLeaveBound) {
            this.dom.removeEventListener("pointerleave", this.onPointerLeaveBound);
        }
        if (this.onPointerDownBound) {
            this.dom.removeEventListener("pointerdown", this.onPointerDownBound);
        }
        if (this.onPointerUpBound) {
            this.dom.removeEventListener("pointerup", this.onPointerUpBound);
        }
    }

    onPointerMove(event) {
        // rAF-throttle the raycast: only the latest pointer position matters.
        this.hoverEvent = event;
        if (this.hoverRaf !== null) {
            return;
        }
        this.hoverRaf = requestAnimationFrame(() => {
            this.hoverRaf = null;
            this.raycastHover(this.hoverEvent);
        });
    }

    onPointerDown(event) {
        this.pointerDownAt = { x: event.clientX, y: event.clientY, t: performance.now() };
    }

    onPointerUp(event) {
        // A CLICK, not an orbit drag: small movement + short duration. Otherwise
        // rotating the globe would pin a cell on release.
        const down = this.pointerDownAt;
        this.pointerDownAt = null;
        if (!down) {
            return;
        }
        const moved = Math.hypot(event.clientX - down.x, event.clientY - down.y);
        if (moved > 5 || performance.now() - down.t > 600) {
            return;
        }
        const cell = this.raycastCellAt(event);
        if (cell) {
            this.pinCell(cell);
        }
    }

    raycastCellAt(event) {
        const layer = this.customLayer;
        if (!layer?.mesh || !event) {
            return null;
        }
        const THREE = window.THREE;
        if (!THREE || !layer.raycaster) {
            return null;
        }
        const rect = this.dom.getBoundingClientRect();
        if (rect.width <= 0 || rect.height <= 0) {
            return null;
        }
        const ndc = new THREE.Vector2(
            ((event.clientX - rect.left) / rect.width) * 2 - 1,
            -(((event.clientY - rect.top) / rect.height) * 2 - 1),
        );
        layer.raycaster.setFromCamera(ndc, this.globe.camera());
        // Mesh only (R2): the LineSegments border never receives the ray.
        const hits = layer.raycaster.intersectObject(layer.mesh, false);
        if (hits.length === 0) {
            return null;
        }
        // `faceCell` maps triangle index -> cell (rings are 5 or 6 sided).
        const cellIndex = layer.faceCell[hits[0].faceIndex];
        return (cellIndex === undefined ? null : layer.cells[cellIndex]) || null;
    }

    raycastHover(event) {
        const cell = this.raycastCellAt(event);
        this.hoverCell = cell;
        if (!cell) {
            this.hideCellTooltip();
            return;
        }
        this.showCellTooltip(cell, event);
    }

    pinCell(cell) {
        if (!cell) {
            return;
        }
        this.pinnedCell = cell;
        this.pinnedCellPos = { lat: cell.lat, lng: cell.lng };
        this.renderPinnedDetails();
    }

    showCellTooltip(cell, event) {
        if (!this.tooltip || !cell) {
            return;
        }
        this.tooltip.innerHTML = this.cellTooltipHtml(cell);
        const rect = this.dom.getBoundingClientRect();
        const x = event.clientX - rect.left + 14;
        const y = event.clientY - rect.top + 14;
        this.tooltip.style.left = `${Math.min(x, Math.max(8, rect.width - 300))}px`;
        this.tooltip.style.top = `${Math.min(y, Math.max(8, rect.height - 140))}px`;
        this.tooltip.classList.add("active");
    }

    hideCellTooltip() {
        this.tooltip?.classList.remove("active");
    }

    cellTooltipHtml(cell) {
        const theme = getThemePalette();
        if (!cell?.covered) {
            return `<div style="font:12px/1.35 system-ui,sans-serif;color:${theme.tooltipText}">`
                + `<strong>No endpoint coverage</strong><br>Baseline lattice cell</div>`;
        }
        const rtt = cell.rttInternetMs > 0
            ? `${cell.rttInternetMs.toFixed(cell.rttInternetMs >= 100 ? 0 : 1)} ms`
            : "Unknown";
        const contributing = Math.max(cell.contributing || 0, cell.measured ? 1 : 0);
        const tag = cell.measured
            ? `Measured (${contributing} endpoint${contributing === 1 ? "" : "s"} shown)`
            : "Interpolated (modeled)";
        const distance = cell.measured ? "" : `<br>${Math.round(cell.sourceDistKm || 0)} km to the source`;
        return `<div style="font:12px/1.35 system-ui,sans-serif;color:${theme.tooltipText}">`
            + `<strong>${rtt}</strong> Internet RTT`
            + `<br>${tag}${distance}`
            + `<br>${scaleNumber(cell.bytes || 0)}B throughput</div>`;
    }

    observeThemeChanges() {
        const observer = new MutationObserver(() => {
            this.applyTheme();
            if (this.rawPayload.length > 0) {
                this.renderData(this.rawPayload);
            }
            updateLegend(this.renderedSummary, this.rttDomain);
            updateHud(this.renderedSummary);
            this.renderPinnedDetails();
        });
        observer.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-bs-theme"],
        });
        this.themeObserver = observer;
    }

    captureViewState() {
        const altitude = toNumber(this.globe?.pointOfView?.()?.altitude, NaN);
        if (Number.isFinite(altitude)) {
            this.lastKnownAltitude = altitude;
        }
    }

    syncControlSpeeds() {
        if (!this.controls) {
            return;
        }
        const altitude = Number.isFinite(this.lastKnownAltitude) ? this.lastKnownAltitude : DEFAULT_POINT_OF_VIEW.altitude;
        this.controls.zoomSpeed = Math.max(MIN_ZOOM_SPEED, Math.min(MAX_ZOOM_SPEED, 0.95 + (altitude * 0.75)));
    }

    currentClusterSize() {
        return this.activeClusterRadius;
    }

    updateClusterRadius() {
        const desired = clusterRadiusPx(this.lastKnownAltitude);
        const current = this.activeClusterRadius;
        if (!Number.isFinite(current)) {
            this.activeClusterRadius = desired;
            return true;
        }
        if (Math.abs(desired - current) < 2.2) {
            return false;
        }
        this.activeClusterRadius = desired;
        return true;
    }

    applyTheme() {
        const theme = getThemePalette();
        this.globe
            .backgroundColor("rgba(0,0,0,0)")
            .atmosphereColor(theme.atmosphereColor)
            .atmosphereAltitude(theme.atmosphereAltitude);

        const material = this.globe.globeMaterial?.();
        if (material) {
            material.color?.set?.(theme.oceanColor);
            material.emissive?.set?.(theme.emissiveTint);
            material.emissiveIntensity = theme.emissiveIntensity;
            material.needsUpdate = true;
        }

        if (this.countryFeatures.length > 0) {
            this.applyCountryLayer();
        }
    }

    renderData(payload) {
        const renderTimer = profileStart("renderData");
        this.rawPayload = Array.isArray(payload) ? payload : [];
        const endpoints = (() => {
            const all = hexbinPointsFromRows(this.rawPayload);
            if (all.length <= FLOW_GLOBE_MAX_ENDPOINTS) {
                return all;
            }
            // Top-N by throughput (stable across Filled/Points/header/inspector).
            return [...all].sort((a, b) => b.bytes - a.bytes).slice(0, FLOW_GLOBE_MAX_ENDPOINTS);
        })();
        this.hexMaxWeight = endpoints.reduce((acc, point) => Math.max(acc, point.bytes), 0);

        const layer = this.buildCustomHexLayer(endpoints);
        this.renderedSummary = {
            pointCount: endpoints.length,
            latticeCells: layer?.cells.length || 0,
            coveredCells: layer?.covered || 0,
            totalBytes: endpoints.reduce((acc, point) => acc + point.bytes, 0),
            maxBytes: this.hexMaxWeight,
            lastUpdatedLabel: this.renderedSummary.lastUpdatedLabel,
        };
        profileEnd(renderTimer, `(endpoints=${endpoints.length}, cells=${layer?.cells.length || 0}, covered=${layer?.covered || 0})`);
        if (this.pinnedCellPos) {
            this.pinnedCell = this.nearestCellTo(this.pinnedCellPos);
        }
        this.renderPinnedDetails();
    }

    // WG2.2-B (R6): ONE merged non-indexed BufferGeometry + ONE colour-only
    // LineSegments added to the globe scene; the grid/geometry maths live in the
    // pure module. `.50.3`: the LATTICE + geometry are cached PER LAYER on a
    // `latticeKey` (frequency|theme|project); a value-only refresh keeps the mesh
    // and eases the live `color` attribute toward the new target. The rAF tween
    // is the SINGLE writer of `color`.
    buildCustomHexLayer(endpoints) {
        const THREE = window.THREE;
        if (!THREE) {
            // Should not happen (vendor/three.min.js loads before globe.gl); fail
            // soft so the base globe still renders.
            console.warn("FlowMap: window.THREE unavailable; the Filled heatmap is disabled");
            this.disposeCustomLayer();
            return null;
        }
        const latticeKey = latticeCacheKey({
            frequency: this.hexFrequency,
            theme: isDarkMode() ? "dark" : "light",
            projectId: this.projectId,
        });
        const plan = this.customLayer
            ? layerUpdatePlan(this.customLayer.latticeKey, latticeKey)
            : "rebuild";
        if (plan === "rebuild") {
            return this.rebuildCustomHexLayer(endpoints, latticeKey);
        }
        return this.recolorCustomHexLayer(endpoints);
    }

    // Lattice/geometry rebuild (Hex size / theme / project change). The mesh uses
    // the FULL lattice so the vertex count is data-independent; the value pass
    // (below) sets the colours.
    rebuildCustomHexLayer(endpoints, latticeKey) {
        const THREE = window.THREE;
        const lattice = buildHexLattice({ frequency: this.hexFrequency });
        let mappedTotal = 0;
        if (endpoints.length > 0) {
            mappedTotal = mapEndpoints(lattice, endpoints, {
                maxEndpoints: FLOW_GLOBE_MAX_ENDPOINTS,
                minRadiusKm: this.haloFloorKm,
                maxRadiusKm: this.haloCapKm,
                k: this.haloK,
                c: this.haloC,
            }).total;
        }
        // `hasData` follows `mapEndpoints`' finite-lat/lng filter, so an
        // all-invalid payload does not show stale colours.
        const hasData = mappedTotal > 0;
        this.layerDomainP5 = hasData ? rttPercentileDomain(lattice.cells).p5 : 0;
        const targetDomain = {
            p5: this.layerDomainP5,
            p95: Math.max(this.rttTopMs, this.layerDomainP5 + 1),
        };
        this.rttDomain = targetDomain;
        const backdrop = backdropRgb();
        const geometry = buildHexGeometry(lattice.cells, {
            project: (lat, lng, alt) => (
                typeof this.globe.getCoords === "function" ? this.globe.getCoords(lat, lng, alt) : null
            ),
            fillAltitude: R6_FILL_ALTITUDE,
            margin: R6_FILL_MARGIN,
            // DRIVER review: no on-map throughput outlines (hex edges read as
            // white clutter); throughput stays in the hover readout.
            includeBorder: false,
            colorFor: (cell) => rgbUnit(this.cellFillRgb(cell, backdrop, targetDomain)),
            alphaFor: (cell) => this.cellAlpha(cell),
        });

        // Dispose the previous layer before wiring the new one (R1).
        this.stopTween();
        this.disposeCustomLayer();

        const meshGeometry = new THREE.BufferGeometry();
        meshGeometry.setAttribute("position", new THREE.BufferAttribute(geometry.positions, 3));
        // `.50.5`: `color` holds the TWEEN TARGET; `aBaseColor` the tween start.
        // The shader mixes them with `uTweenAlpha` — per-frame CPU O(1), NO
        // per-frame colour upload during the glide. 4-component = per-vertex RGBA.
        const targetColors = geometry.colors;
        const baseColors = new Float32Array(targetColors.length);
        baseColors.set(targetColors);
        meshGeometry.setAttribute("color", new THREE.BufferAttribute(targetColors, 4));
        meshGeometry.setAttribute("aBaseColor", new THREE.BufferAttribute(baseColors, 4));
        const tweenUniform = { value: 1 };
        this.tweenUniform = tweenUniform;
        const meshMaterial = new THREE.MeshBasicMaterial({
            vertexColors: true,
            side: THREE.DoubleSide,
            transparent: true,
            opacity: 1,
            depthWrite: false,
        });
        meshMaterial.onBeforeCompile = (shader) => {
            shader.uniforms.uTweenAlpha = tweenUniform;
            shader.vertexShader = `attribute vec4 aBaseColor;\nuniform float uTweenAlpha;\n${shader.vertexShader}`;
            // Pin 3: guard the vec4 `vColor` behind USE_COLOR_ALPHA.
            const before = shader.vertexShader;
            shader.vertexShader = shader.vertexShader.replace(
                "#include <color_vertex>",
                "#include <color_vertex>\n#ifdef USE_COLOR_ALPHA\n  vColor = mix(aBaseColor, color, uTweenAlpha);\n#endif",
            );
            if (shader.vertexShader === before) {
                console.warn("FlowMap: GPU-tween injection missed the color_vertex chunk");
            }
        };
        // Pin 2: a unique program-cache key so three cannot reuse a plain
        // `MeshBasicMaterial(vertexColors)` program without our injection.
        meshMaterial.customProgramCacheKey = () => "flow-globe-gpu-tween";
        const mesh = new THREE.Mesh(meshGeometry, meshMaterial);
        mesh.renderOrder = 2;
        this.globe.scene().add(mesh);

        this.customLayer = {
            latticeKey,
            lattice,
            // MUST be geometry.emittedCells (the objects the mesh's positions,
            // `faceCell` and colour buffers were built from) — buildHexGeometry
            // SKIPS cells whose projection is not ready, so emitted ⊆ lattice.
            // The emitted objects are the SAME references `mapEndpoints` mutates.
            cells: geometry.emittedCells,
            faceCell: geometry.faceCell,
            covered: geometry.coveredCount,
            triangleCount: geometry.triangleCount,
            mesh,
            border: null,
            raycaster: new THREE.Raycaster(),
            targetColors,
            baseColors,
            targetAttribute: meshGeometry.attributes.color,
            baseAttribute: meshGeometry.attributes.aBaseColor,
            domainP5Target: this.layerDomainP5,
            domainP5Current: this.layerDomainP5,
            domainP5Start: this.layerDomainP5,
            hasData,
        };
        // Re-seed BOTH colour buffers: the vertex count changed, so there is no
        // previous frame to ease from.
        this.setColorTargets({ instant: true });
        return this.customLayer;
    }

    // Value-only refresh (same latticeKey): keep the mesh, remap the endpoints
    // INTO the cached lattice, and ease the colours to the new target.
    recolorCustomHexLayer(endpoints) {
        const layer = this.customLayer;
        if (!layer) {
            return null;
        }
        let mappedTotal = 0;
        if (endpoints.length > 0) {
            // Mutates `layer.lattice.cells` in place; `layer.cells` (the emitted
            // objects) are the same references, so they pick up the values.
            mappedTotal = mapEndpoints(layer.lattice, endpoints, {
                maxEndpoints: FLOW_GLOBE_MAX_ENDPOINTS,
                minRadiusKm: this.haloFloorKm,
                maxRadiusKm: this.haloCapKm,
                k: this.haloK,
                c: this.haloC,
            }).total;
        }
        // Follow `mapEndpoints`' finite-lat/lng filter, not the raw array length.
        layer.hasData = mappedTotal > 0;
        if (layer.hasData) {
            layer.covered = layer.cells.filter((cell) => cell.covered).length;
            layer.domainP5Target = rttPercentileDomain(layer.cells).p5;
        } else {
            // 0-endpoint pass: keep the cached mesh/cells; `hasData=false` makes
            // `setColorTargets` pack over `layer.cells` (the mesh's N cells) with
            // alpha 0 — NOT mapEndpoints' empty return — so the target length
            // equals the attribute length and the mesh fades out.
            layer.covered = 0;
            layer.domainP5Target = layer.domainP5Current;
        }
        this.setColorTargets({ instant: false });
        return layer;
    }

    // `.50.5`: the rAF loop writes only the `uTweenAlpha` uniform; the attribute
    // buffers are written here (once per tick/snap). `instant` (rebuild / Red-at /
    // colorblind) snaps base=target and alpha=1; a data tick folds the currently
    // displayed colour into `aBaseColor` (one O(vertices) pass) then uploads the
    // new target and restarts the glide at alpha=0.
    setColorTargets({ instant = false } = {}) {
        const layer = this.customLayer;
        const targetAttribute = layer?.targetAttribute;
        const baseAttribute = layer?.baseAttribute;
        if (!layer || !targetAttribute || !baseAttribute) {
            return;
        }
        const backdrop = backdropRgb();
        const domain = {
            p5: layer.domainP5Target,
            p95: Math.max(this.rttTopMs, layer.domainP5Target + 1),
        };
        const target = layer.targetColors;
        const base = layer.baseColors;
        if (instant) {
            this.packLayerTargets(layer, backdrop, domain, target);
            base.set(target);
            this.tweenUniform.value = 1;
            layer.domainP5Current = layer.domainP5Target;
            layer.domainP5Start = layer.domainP5Target;
            this.layerDomainP5 = layer.domainP5Current;
            this.syncRttDomain();
            targetAttribute.needsUpdate = true;
            baseAttribute.needsUpdate = true;
        } else {
            // Fold the on-screen colour into the base BEFORE `color` is
            // overwritten: base = mix(base, oldTarget, alphaNow). Once per tick.
            mixColors(base, target, this.tweenUniform.value);
            baseAttribute.needsUpdate = true;
            this.packLayerTargets(layer, backdrop, domain, target);
            targetAttribute.needsUpdate = true;
            layer.domainP5Start = layer.domainP5Current;
            this.tweenUniform.value = 0;
            this.tweenStartAt = null;
        }
        this.startTween();
    }

    packLayerTargets(layer, backdrop, domain, out) {
        if (layer.hasData) {
            packLayerColors(layer.cells, {
                rgbFor: (cell) => rgbUnit(this.cellFillRgb(cell, backdrop, domain)),
                alphaFor: (cell) => this.cellAlpha(cell),
                out,
            });
        } else {
            packLayerColors(layer.cells, {
                rgbFor: () => [0, 0, 0],
                alphaFor: () => 0,
                out,
            });
        }
    }

    startTween() {
        if (this.tweenRaf !== null && this.tweenRaf !== undefined) {
            return;
        }
        if (!this.canAnimate()) {
            return;
        }
        // Reset the glide clock ONLY on a fresh start (after the running-guard).
        this.tweenStartAt = null;
        this.tweenRaf = requestAnimationFrame(() => this.tweenFrame());
    }

    // `.50.5` Lever 3: the rAF runs only when the tab is visible, the canvas is
    // in the viewport, and the window is focused.
    canAnimate() {
        return !document.hidden && this.inViewport !== false && this.hasFocus !== false;
    }

    stopTween() {
        if (this.tweenRaf !== null && this.tweenRaf !== undefined) {
            cancelAnimationFrame(this.tweenRaf);
        }
        this.tweenRaf = null;
    }

    resumeTween() {
        this.startTween();
    }

    tweenFrame() {
        this.tweenRaf = null;
        const layer = this.customLayer;
        if (!layer || !layer.targetColors) {
            return;
        }
        const now = performance.now();
        if (this.tweenStartAt === null || this.tweenStartAt === undefined) {
            this.tweenStartAt = now;
        }
        const elapsed = (now - this.tweenStartAt) / 1000;
        const alpha = tweenAlpha(elapsed, TWEEN_TAU_S);
        // Pin 1: O(1) per frame — one uniform + the scalar p5/domain ease.
        this.tweenUniform.value = alpha;
        layer.domainP5Current = layer.domainP5Start
            + (layer.domainP5Target - layer.domainP5Start) * alpha;
        this.layerDomainP5 = layer.domainP5Current;
        this.rttDomain = {
            p5: layer.domainP5Current,
            p95: Math.max(this.rttTopMs, layer.domainP5Current + 1),
        };
        if (alpha >= 0.999) {
            this.tweenUniform.value = 1;
            layer.domainP5Current = layer.domainP5Target;
            layer.domainP5Start = layer.domainP5Target;
            this.layerDomainP5 = layer.domainP5Current;
            this.rttDomain = {
                p5: layer.domainP5Current,
                p95: Math.max(this.rttTopMs, layer.domainP5Current + 1),
            };
            refreshHud(false);
        } else {
            this.tweenRaf = requestAnimationFrame(() => this.tweenFrame());
        }
    }

    // Fill colour (0..255): faint grey NO-DATA baseline; the adaptive RTT ramp
    // for a measured cell; the same hue for an interpolated cell (whose strength
    // is carried by `cellAlpha`, not the RGB). A covered cell with no RTT sample
    // (0) is NO-DATA too, not the ramp's blue end.
    cellFillRgb(cell, backdrop, domain = this.rttDomain) {
        return r6CellFillRgb(cell, {
            backdrop,
            neutral: parseColorValue(getThemePalette().hexNeutralRgb),
            domain,
            colorblind: isColorBlindMode(),
        });
    }

    // Red-at slider + global colorblind toggle: an INSTANT recolour (snap
    // current = target) through the single tween writer — no direct attribute
    // write, so the tween's next frame can never revert a manual change.
    applyLayerColors() {
        this.setColorTargets({ instant: true });
    }

    // Per-cell opacity (0..1): unknown cells are nearly transparent so the globe
    // shows through; measured cells are near-opaque; INTERPOLATED cells fade with
    // the distance to their source, so a lone endpoint's large halo dissolves
    // instead of painting a flat disco.
    cellAlpha(cell) {
        if (!cell?.covered || !(cell.rttInternetMs > 0)) {
            return 0.08;
        }
        if (cell.measured) {
            return 0.95;
        }
        const fadeKm = Math.min(cell.sourceRadiusKm || 0, 1200);
        if (!(fadeKm > 0)) {
            return 0.08;
        }
        const t = Math.max(0, Math.min(1, (cell.sourceDistKm || 0) / fadeKm));
        return 0.06 + 0.64 * Math.pow(1 - t, 1.5);
    }

    disposeCustomLayer() {
        const layer = this.customLayer;
        if (!layer) {
            return;
        }
        const scene = this.globe?.scene?.();
        for (const object of [layer.mesh, layer.border]) {
            if (!object) {
                continue;
            }
            scene?.remove?.(object);
            object.geometry?.dispose?.();
            if (Array.isArray(object.material)) {
                object.material.forEach((material) => material?.dispose?.());
            } else {
                object.material?.dispose?.();
            }
        }
        this.customLayer = null;
        this.hideCellTooltip();
    }

    // The colour domain: blue = the data's adaptive p5, red = the manual
    // rttTopMs. Guard a manual top below p5 (a negative span would invert the
    // ramp): raise the top to at least p5+1 and keep the slider's min in step.
    syncRttDomain() {
        const p5 = this.layerDomainP5;
        const minTop = Math.max(50, Math.ceil(p5) + 1);
        if (this.rttSlider) {
            this.rttSlider.min = String(Math.max(50, Math.ceil(p5)));
        }
        if (this.rttTopMs < minTop) {
            this.rttTopMs = minTop;
            if (this.rttSlider) {
                this.rttSlider.value = String(this.rttTopMs);
            }
            if (this.rttSliderLabel) {
                this.rttSliderLabel.textContent = `${this.rttTopMs} ms`;
            }
        }
        this.rttDomain = { p5, p95: Math.max(this.rttTopMs, p5 + 1) };
    }

    updateSummary(pointCount, totalBytes, timestamp) {
        this.renderedSummary.pointCount = pointCount;
        this.renderedSummary.totalBytes = totalBytes;
        this.renderedSummary.lastUpdatedLabel = formatAge(timestamp);
    }

    setAutoRotate(enabled) {
        this.autoRotate = !!enabled;
        if (this.controls) {
            this.controls.autoRotate = this.autoRotate;
        }
    }

    resetView() {
        this.globe.pointOfView(this.initialPointOfView, 700);
        this.lastKnownAltitude = this.initialPointOfView.altitude;
        this.activeClusterRadius = clusterRadiusPx(this.initialPointOfView.altitude);
        if (this.rawPayload.length > 0) {
            this.renderData(this.rawPayload);
        }
    }

    computeInitialPointOfView(payload) {
        const source = Array.isArray(payload) ? payload : [];
        const valid = source
            .map((row) => ({
                lat: toNumber(row?.[0], NaN),
                lng: toNumber(row?.[1], NaN),
                bytes: Math.max(1, toNumber(row?.[3], 0)),
            }))
            .filter((point) => Number.isFinite(point.lat) && Number.isFinite(point.lng));

        if (valid.length === 0) {
            return { ...DEFAULT_POINT_OF_VIEW };
        }

        let sumX = 0;
        let sumY = 0;
        let sumZ = 0;
        let totalWeight = 0;
        const vectors = valid.map((point) => {
            const vector = latLngToUnitVector(point.lat, point.lng);
            const weight = Math.max(1, Math.sqrt(point.bytes));
            sumX += vector.x * weight;
            sumY += vector.y * weight;
            sumZ += vector.z * weight;
            totalWeight += weight;
            return { ...point, vector, weight };
        });

        if (totalWeight <= 0) {
            return { ...DEFAULT_POINT_OF_VIEW };
        }

        const centerVector = normalizeVector(sumX, sumY, sumZ);
        const center = unitVectorToLatLng(centerVector);

        let weightedSpread = 0;
        let maxSpread = 0;
        vectors.forEach((point) => {
            const distance = angularDistanceDegrees(centerVector, point.vector);
            weightedSpread += distance * point.weight;
            maxSpread = Math.max(maxSpread, distance);
        });

        const averageSpread = weightedSpread / totalWeight;
        const spreadReference = Math.max(averageSpread * 1.35, maxSpread * 0.78);
        const width = Math.max(1, this.dom.clientWidth || 960);
        const height = Math.max(1, this.dom.clientHeight || 640);
        const aspect = width / height;
        const aspectAdjustment = aspect > 1.3 ? Math.min(0.22, (aspect - 1.3) * 0.22) : 0;
        const altitude = clamp(
            1.02 + clamp((spreadReference - 6) / 85, 0, 1) * 1.08 - aspectAdjustment,
            MIN_INITIAL_ALTITUDE,
            MAX_INITIAL_ALTITUDE,
        );

        return {
            lat: center.lat,
            lng: center.lng,
            altitude,
        };
    }

    updateInitialPointOfViewFromPayload(payload) {
        if (this.hasAutoCenteredOnce) {
            return;
        }
        this.initialPointOfView = this.computeInitialPointOfView(payload);
        this.hasAutoCenteredOnce = true;
        this.globe.pointOfView(this.initialPointOfView, 0);
        this.lastKnownAltitude = this.initialPointOfView.altitude;
        this.activeClusterRadius = clusterRadiusPx(this.initialPointOfView.altitude);
        this.syncControlSpeeds();
    }

    scheduleInteractionRelease() {
        if (this.interactionTimer) {
            clearTimeout(this.interactionTimer);
        }
        this.interactionTimer = setTimeout(() => {
            this.userInteracting = false;
            this.captureViewState();
            this.updateClusterRadius();
            if (this.rawPayload.length > 0) {
                this.renderData(this.rawPayload);
                refreshHud();
            }
            this.interactionTimer = null;
            scheduleNextRefresh();
        }, 400);
    }

    scheduleRecluster() {
        if (this.reclusterTimer || this.rawPayload.length === 0) {
            return;
        }
        if (FPS_METER_ENABLED) {
            perfReclusters += 1;
        }
        this.reclusterTimer = setTimeout(() => {
            this.reclusterTimer = null;
            this.renderData(this.rawPayload);
            refreshHud();
        }, 60);
    }

    resize() {
        this.globe
            .width(this.dom.clientWidth || 960)
            .height(this.dom.clientHeight || 640);
    }

    destroy() {
        window.removeEventListener("colorBlindModeChanged", this.onColorBlindChanged);
        window.removeEventListener("blur", this.onWindowBlur);
        window.removeEventListener("focus", this.onWindowFocus);
        this.inViewportObserver?.disconnect?.();
        this.inViewportObserver = null;
        if (this.interactionTimer) {
            clearTimeout(this.interactionTimer);
        }
        if (this.reclusterTimer) {
            clearTimeout(this.reclusterTimer);
        }
        if (this.hexSizeTimer) {
            clearTimeout(this.hexSizeTimer);
            this.hexSizeTimer = null;
        }
        this.stopTween();
        if (this.hoverRaf !== null) {
            cancelAnimationFrame(this.hoverRaf);
            this.hoverRaf = null;
        }
        if (this.themeObserver) {
            this.themeObserver.disconnect();
        }
        this.detachHoverListeners();
        this.disposeCustomLayer();
        this.disposeCountryLayer();
    }

    clearPinnedDetails() {
        this.pinnedCell = null;
        this.pinnedCellPos = null;
        this.renderPinnedDetails();
    }

    // The pinned cell's grid position is stable across refreshes (the grid does
    // not depend on the endpoints), so re-resolve it by nearest centre.
    nearestCellTo(pos) {
        const cells = this.customLayer?.cells;
        if (!cells?.length || !pos) {
            return null;
        }
        let best = null;
        let bestDist = Infinity;
        for (const cell of cells) {
            const d = ((cell.lat - pos.lat) ** 2) + ((cell.lng - pos.lng) ** 2);
            if (d < bestDist) {
                bestDist = d;
                best = cell;
            }
        }
        return best;
    }

    renderPinnedDetails() {
        const panel = document.getElementById("flowMapDetails");
        const title = document.getElementById("flowMapDetailsTitle");
        const subtitle = document.getElementById("flowMapDetailsSubtitle");
        const body = document.getElementById("flowMapDetailsBody");
        const members = document.getElementById("flowMapDetailsMembers");
        if (!panel || !title || !subtitle || !body || !members) {
            return;
        }

        const empty = () => {
            panel.classList.remove("active");
            body.innerHTML = "";
            members.style.display = "none";
            members.textContent = "";
        };

        if (this.pinnedCell) {
            const cell = this.pinnedCell;
            panel.classList.add("active");
            title.textContent = cell.measured
                ? "Measured cell"
                : (cell.covered ? "Interpolated cell" : "No-coverage cell");
            subtitle.textContent = `${cell.lat.toFixed(1)}, ${cell.lng.toFixed(1)} — locations in this cell`;
            body.innerHTML = renderCellBody(cell).map(([label, value]) => `
                <div class="flow-map-details-row">
                    <span>${escapeHtml(label)}</span>
                    <strong>${escapeHtml(value)}</strong>
                </div>
            `).join("");
            const list = Array.isArray(cell.members) ? cell.members : [];
            if (list.length > 0) {
                const sorted = [...list].sort((a, b) => b.bytes - a.bytes);
                const shown = sorted.slice(0, 50);
                const total = Math.max(cell.contributing || 0, sorted.length);
                members.innerHTML = shown.map((member) => {
                    const rtt = member.rttInternetMs > 0
                        ? `${member.rttInternetMs.toFixed(member.rttInternetMs >= 100 ? 0 : 1)} ms`
                        : "—";
                    return `<div>${escapeHtml(member.country || "Unknown")} · `
                        + `${member.lat.toFixed(1)}, ${member.lng.toFixed(1)} · `
                        + `${escapeHtml(rtt)} · ${escapeHtml(scaleNumber(member.bytes))}B</div>`;
                }).join("") + (total > shown.length
                    ? `<div>top ${shown.length} of ${total}</div>`
                    : "");
                members.style.display = "block";
            } else if (cell.covered) {
                const km = Number.isFinite(cell.sourceDistKm) ? `${Math.round(cell.sourceDistKm)} km away` : "elsewhere";
                members.innerHTML = `<div>Modeled from the nearest source (${escapeHtml(km)}); no location sits in this cell.</div>`;
                members.style.display = "block";
            } else {
                members.style.display = "none";
                members.textContent = "";
            }
            return;
        }

        empty();
    }
}

const map = new FlowMapGlobe("flowMap");
const overlay = makeOverlay(map.dom, "flowMapOverlay");
let updateTimer = null;
let pendingRequest = null;
let lastUpdateAt = 0;
let lastHudAt = null;
// `.50.7` perf-instrument counters (incremented only when `?flowMapFps=1`).
const FPS_METER_ENABLED = new URLSearchParams(window.location.search).has("flowMapFps");
let perfControlsChanges = 0;
let perfReclusters = 0;

function syncRotateButton() {
    const rotateBtn = document.getElementById("flowMapRotateBtn");
    if (!rotateBtn) {
        return;
    }
    rotateBtn.classList.toggle("btn-primary", map.autoRotate);
    rotateBtn.classList.toggle("btn-outline-secondary", !map.autoRotate);
    rotateBtn.setAttribute("aria-pressed", map.autoRotate ? "true" : "false");
}

function syncRefreshButton(loading = false) {
    const refreshBtn = document.getElementById("flowMapRefreshBtn");
    if (!refreshBtn) {
        return;
    }
    refreshBtn.disabled = loading;
    refreshBtn.innerHTML = loading
        ? '<i class="fa fa-spinner fa-spin me-1"></i>Refreshing...'
        : '<i class="fa fa-rotate me-1"></i>Refresh';
}

function syncAutoRefreshToggle() {
    const autoRefresh = document.getElementById("flowMapAutoRefresh");
    if (!autoRefresh) {
        return;
    }
    autoRefresh.checked = map.autoRefresh;
}

// `.50.4` Lever 1: the heavier legend/stats repaint is throttled on the auto
// path; `force` (a manual Refresh or a user-driven recolour) always repaints.
// `lastHudAt` MUST be recorded on every repaint or the throttle never engages.
function refreshHud(force = true) {
    const now = performance.now();
    if (!shouldRefreshHud({ manual: force, lastHudMs: lastHudAt, nowMs: now, throttleMs: HUD_THROTTLE_MS })) {
        return;
    }
    lastHudAt = now;
    map.updateSummary(map.renderedSummary.pointCount, map.renderedSummary.totalBytes, lastUpdateAt);
    updateLegend(map.renderedSummary, map.rttDomain);
    updateHud(map.renderedSummary);
}

// `.50.4` Lever 1: the cheap "updated Xs ago" header text still ticks every
// auto-tick (no legend/stats recompute), so the age stays accurate under the 5 s
// throttle of the heavier repaint.
function tickAgeLabel() {
    const lastUpdated = document.getElementById("flowMapLastUpdated");
    if (lastUpdated) {
        lastUpdated.textContent = formatAge(lastUpdateAt);
    }
}

// `.50.5`/`.50.7` diagnostic: `?flowMapFps=1` shows a rolling 1 s readout so the
// >=60 fps + perf-breakdown can be MEASURED: fps, ms (mean frame interval),
// renderer.info.render.triangles/calls, chg/s (controls change-handler firings =
// symptom A), recl/s (scheduleRecluster runs), age (s since the last data refresh
// = is the 5 s cadence holding?), rot/f (the arm). Behavior-neutral without the
// param. Removal gate 2026-12-31.
function startFpsMeterIfRequested() {
    const params = new URLSearchParams(window.location.search);
    if (!params.has("flowMapFps")) {
        return;
    }
    const el = document.createElement("div");
    el.id = "flowMapFps";
    el.style.cssText = "position:fixed;top:8px;left:8px;z-index:9999;padding:2px 6px;"
        + "background:rgba(0,0,0,0.6);color:#0f0;font:12px monospace;border-radius:4px;white-space:pre;";
    el.textContent = "--";
    document.body.appendChild(el);
    let frames = 0;
    let last = performance.now();
    // `.50.7`: drop the hidden gap — the first window after an unhide would
    // otherwise include the hidden duration (fps≈0 / ms≈gap). rAF is paused while
    // hidden, so reset the clock on every visibility change.
    document.addEventListener("visibilitychange", () => {
        frames = 0;
        last = performance.now();
    });
    const tick = (now) => {
        frames += 1;
        if (now - last >= 1000) {
            const windowMs = now - last;
            const fps = frames > 0 ? (frames * 1000) / windowMs : 0;
            const ms = frames > 0 ? windowMs / frames : NaN;
            const renderer = map.globe?.renderer?.();
            let tri = "-";
            let calls = "-";
            if (renderer?.info) {
                const info = renderer.info;
                tri = info.render.triangles;
                calls = info.render.calls;
                // Three's default `autoReset=true` => `.render` is the LAST frame;
                // if false the counters accumulate (false-high) — reset so each
                // 1 s window is a bounded read.
                if (info.autoReset === false && typeof info.reset === "function") {
                    info.reset();
                }
            }
            const chg = perfControlsChanges;
            const recl = perfReclusters;
            perfControlsChanges = 0;
            perfReclusters = 0;
            const age = lastUpdateAt > 0 ? (Date.now() - lastUpdateAt) / 1000 : NaN;
            el.textContent = `fps=${Math.round(fps)}`
                + ` ms=${Number.isFinite(ms) ? ms.toFixed(1) : "-"}`
                + ` tri=${tri} calls=${calls}`
                + ` chg/s=${chg} recl/s=${recl}`
                + ` age=${Number.isFinite(age) ? `${age.toFixed(1)}s` : "-"}`
                + ` rot=${map.autoRotate ? "on" : "off"} f=${map.hexFrequency}`;
            frames = 0;
            last = now;
        }
        requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
}

function scheduleNextRefresh() {
    if (!map.autoRefresh || document.hidden || updateTimer || pendingRequest || map.userInteracting) {
        return;
    }
    updateTimer = setTimeout(() => {
        updateTimer = null;
        refreshNow(false);
    }, AUTO_REFRESH_MS);
}

function clearScheduledRefresh() {
    if (updateTimer) {
        clearTimeout(updateTimer);
        updateTimer = null;
    }
}

function refreshNow(manual = false) {
    if (pendingRequest) {
        return;
    }
    clearScheduledRefresh();
    // `.50.4` Lever 1: only a MANUAL refresh shows the "Refreshing…" spinner; the
    // 1 s auto-tick is silent (no button state, no forced HUD repaint).
    if (manual) {
        syncRefreshButton(true);
    }
    pendingRequest = listenOnceWithTimeout("FlowMap", RESPONSE_TIMEOUT_MS, (msg) => {
        pendingRequest = null;
        if (manual) {
            syncRefreshButton(false);
        }
        const data = msg?.data || [];
        const totalBytes = data.reduce((acc, row) => acc + toNumber(row?.[3], 0), 0);
        lastUpdateAt = Date.now();

        if (data.length < MIN_POINTS || totalBytes < MIN_TOTAL_BYTES) {
            overlay.show("Insufficient data", "Not enough recent flow traffic to render the map yet.");
            map.renderData([]);
            map.updateSummary(0, totalBytes, lastUpdateAt);
        } else {
            map.updateInitialPointOfViewFromPayload(data);
            overlay.hide();
            map.renderData(data);
            map.updateSummary(map.renderedSummary.pointCount, totalBytes, lastUpdateAt);
        }

        refreshHud(manual);
        tickAgeLabel();
        scheduleNextRefresh();
    }, () => {
        pendingRequest = null;
        if (manual) {
            syncRefreshButton(false);
        }
        overlay.show("Waiting for data", "No FlowMap websocket response received yet.");
        map.renderData([]);
        map.updateSummary(0, 0, 0);
        refreshHud(manual);
        scheduleNextRefresh();
    });
    wsClient.send({ FlowMap: {} });
}

function stopUpdates() {
    clearScheduledRefresh();
    map.stopTween();
    if (pendingRequest) {
        pendingRequest.cancel();
        pendingRequest = null;
        syncRefreshButton(false);
    }
}

function resumeUpdates() {
    map.resumeTween();
    if (!pendingRequest && map.autoRefresh) {
        scheduleNextRefresh();
    }
}

function initControls() {
    const refreshBtn = document.getElementById("flowMapRefreshBtn");
    const resetBtn = document.getElementById("flowMapResetBtn");
    const rotateBtn = document.getElementById("flowMapRotateBtn");
    const autoRefresh = document.getElementById("flowMapAutoRefresh");
    const detailsClose = document.getElementById("flowMapDetailsClose");

    if (refreshBtn) {
        refreshBtn.addEventListener("click", () => refreshNow(true));
    }
    if (resetBtn) {
        resetBtn.addEventListener("click", () => map.resetView());
    }
    if (rotateBtn) {
        rotateBtn.addEventListener("click", () => {
            map.setAutoRotate(!map.autoRotate);
            syncRotateButton();
        });
    }
    if (autoRefresh) {
        autoRefresh.checked = map.autoRefresh;
        autoRefresh.addEventListener("change", () => {
            map.autoRefresh = !!autoRefresh.checked;
            if (map.autoRefresh) {
                scheduleNextRefresh();
            } else {
                clearScheduledRefresh();
            }
        });
    }
    // Hexagon size slider -> hexasphere frequency (rebuild, debounced).
    const hexSize = document.getElementById("flowGlobeHexSize");
    if (hexSize) {
        hexSize.value = String(map.hexFrequency);
        hexSize.addEventListener("input", () => {
            const f = parseInt(hexSize.value, 10);
            if (!Number.isFinite(f)) {
                return;
            }
            map.hexFrequency = Math.max(6, Math.min(64, f));
            if (map.hexSizeTimer) {
                clearTimeout(map.hexSizeTimer);
            }
            map.hexSizeTimer = setTimeout(() => {
                map.hexSizeTimer = null;
                map.renderData(map.rawPayload);
                refreshHud();
            }, 160);
        });
    }
    // RTT "red at" slider -> recolour in place (no geometry rebuild).
    const rttMax = document.getElementById("flowGlobeRttMax");
    const rttMaxLabel = document.getElementById("flowGlobeRttMaxLabel");
    if (rttMax) {
        map.rttSlider = rttMax;
        map.rttSliderLabel = rttMaxLabel;
        rttMax.value = String(map.rttTopMs);
        if (rttMaxLabel) {
            rttMaxLabel.textContent = `${map.rttTopMs} ms`;
        }
        rttMax.addEventListener("input", () => {
            const v = parseInt(rttMax.value, 10);
            if (!Number.isFinite(v)) {
                return;
            }
            map.rttTopMs = Math.max(50, Math.min(400, v));
            // May raise rttTopMs above the data's p5 and keep the slider min in step.
            map.syncRttDomain();
            if (rttMaxLabel) {
                rttMaxLabel.textContent = `${map.rttTopMs} ms`;
            }
            map.applyLayerColors();
            refreshHud();
        });
    }
    if (detailsClose) {
        detailsClose.addEventListener("click", () => map.clearPinnedDetails());
    }

    syncRotateButton();
    syncRefreshButton(false);
    syncAutoRefreshToggle();
    refreshHud();
}

window.addEventListener("resize", () => {
    try {
        map.resize();
    } catch (_) {
        // Ignore resize errors during teardown.
    }
});

document.addEventListener("visibilitychange", () => {
    if (document.hidden) {
        stopUpdates();
    } else {
        resumeUpdates();
    }
});

window.addEventListener("beforeunload", () => {
    stopUpdates();
    map.destroy();
});

initControls();
overlay.show("Waiting for data", "Requesting recent flow endpoints...");
refreshHud(true);
refreshNow(true);
startFpsMeterIfRequested();
