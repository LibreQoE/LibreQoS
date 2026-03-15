import { colorByRttMs } from "./helpers/color_scales";
import { isDarkMode } from "./helpers/dark_mode";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client } from "./pubsub/ws";

const wsClient = get_ws_client();

const AUTO_REFRESH_MS = 15000;
const RESPONSE_TIMEOUT_MS = 2000;
const MIN_POINTS = 3;
const MIN_TOTAL_BYTES = 1_000_000;
const CLUSTER_PREVIEW_LIMIT = 3;
const COUNTRIES_GEOJSON_PATH = "vendor/countries.geojson";
const FLOW_MAP_PROFILE = false;
const DEFAULT_POINT_OF_VIEW = {
    lat: 18,
    lng: -32,
    altitude: 2.15,
};
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
    };
}

function trafficColor(bytes, maxBytes) {
    const dark = isDarkMode();
    const ratio = maxBytes > 0 ? Math.max(0, Math.min(1, bytes / maxBytes)) : 0;
    if (dark) {
        if (ratio < 0.2) return "#28456c";
        if (ratio < 0.4) return "#2c64b6";
        if (ratio < 0.65) return "#3ea6ff";
        if (ratio < 0.85) return "#56d4ff";
        return "#7cffb2";
    }
    if (ratio < 0.2) return "#b8c8d8";
    if (ratio < 0.4) return "#7da6bf";
    if (ratio < 0.65) return "#61a0a8";
    if (ratio < 0.85) return "#467fe0";
    return "#2563eb";
}

function hexToRgba(color, alpha) {
    if (typeof color !== "string" || !color.startsWith("#")) {
        return color;
    }
    const hex = color.slice(1);
    if (hex.length !== 6) {
        return color;
    }
    const r = parseInt(hex.slice(0, 2), 16);
    const g = parseInt(hex.slice(2, 4), 16);
    const b = parseInt(hex.slice(4, 6), 16);
    return `rgba(${r}, ${g}, ${b}, ${alpha.toFixed(3)})`;
}

function markerDisplayColor(color, bytes, maxBytes, mode, selected = false) {
    if (selected || mode === "traffic") {
        return color;
    }
    const ratio = maxBytes > 0 ? Math.max(0, Math.min(1, bytes / maxBytes)) : 0;
    const alpha = 0.74 + (Math.pow(ratio, 0.7) * 0.24);
    return hexToRgba(color, alpha);
}

function markerSizePx(bytes, maxBytes, count = 1, mode = "rtt") {
    const ratio = maxBytes > 0 ? Math.max(0, Math.min(1, bytes / maxBytes)) : 0;
    const trafficWeight = Math.pow(ratio, mode === "traffic" ? 0.85 : 0.62);
    const clusterBoost = count > 1 ? Math.min(18, Math.log2(count + 1) * 4.4) : 0;
    return Math.round((mode === "traffic" ? 13 : 15) + (trafficWeight * (mode === "traffic" ? 18 : 24)) + clusterBoost);
}

function markerDotRadius(bytes, maxBytes, count = 1, selected = false, mode = "rtt") {
    const px = markerSizePx(bytes, maxBytes, count, mode);
    const base = Math.max(0.11, Math.min(0.28, px / 92));
    return selected ? base * 1.18 : base;
}

function markerAltitude(bytes, maxBytes, count = 1) {
    const ratio = maxBytes > 0 ? Math.max(0, Math.min(1, bytes / maxBytes)) : 0;
    const clusterBoost = count > 1 ? Math.min(0.0012, Math.log2(count + 1) * 0.0002) : 0;
    return 0.0054 + (ratio * 0.0011) + clusterBoost;
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

function dotProduct(a, b) {
    return (a.x * b.x) + (a.y * b.y) + (a.z * b.z);
}

function formatRtt(rttNanos) {
    const rttMs = toNumber(rttNanos, 0) / 1_000_000;
    if (rttMs <= 0) {
        return "Unknown";
    }
    return `${rttMs.toFixed(rttMs >= 100 ? 0 : 1)} ms`;
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

function updateLegend(colorMode, summary) {
    const theme = getThemePalette();
    const gradient = document.getElementById("flowMapLegendGradient");
    const title = document.getElementById("flowMapLegendTitle");
    const body = document.getElementById("flowMapLegendBody");
    const min = document.getElementById("flowMapLegendMin");
    const max = document.getElementById("flowMapLegendMax");
    const status = document.getElementById("flowMapLegendStatus");

    if (gradient) {
        gradient.style.background = colorMode === "traffic" ? theme.trafficGradient : theme.legendGradient;
    }
    if (title) {
        title.textContent = colorMode === "traffic" ? "Traffic Gradient" : "Latency Gradient";
    }
    if (body) {
        body.textContent = colorMode === "traffic"
            ? "Marker color shows relative traffic intensity. Marker diameter shows recent traffic volume."
            : "Marker color shows RTT. Marker diameter shows recent traffic volume.";
    }
    if (min) {
        min.textContent = colorMode === "traffic" ? "Lighter" : "Fast";
    }
    if (max) {
        max.textContent = colorMode === "traffic" ? "Heavier" : "Slower";
    }
    if (status) {
        const total = summary.totalBytes > 0 ? scaleNumber(summary.totalBytes) : "0";
        status.textContent = `${summary.pointCount} markers, ${total}B recent volume`;
    }
}

function renderDetailsBody(meta) {
    return [
        ["Type", meta.isCluster ? "Cluster" : "Endpoint"],
        [meta.colorLabel, meta.colorValue],
        ["Traffic", meta.bytes],
        ["Latitude", meta.lat.toFixed(2)],
        ["Longitude", meta.lng.toFixed(2)],
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

function metaMatches(selectedMeta, candidateMeta) {
    if (!selectedMeta || !candidateMeta) {
        return false;
    }
    if (selectedMeta.isCluster || candidateMeta.isCluster) {
        return candidateMeta.label === selectedMeta.label;
    }
    return candidateMeta.label === selectedMeta.label
        && Math.abs(candidateMeta.lat - selectedMeta.lat) < 0.0001
        && Math.abs(candidateMeta.lng - selectedMeta.lng) < 0.0001;
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

        this.rawPayload = [];
        this.countryFeatures = [];
        this.renderedPoints = [];
        this.renderedClusterCount = 0;
        this.pointIndex = new Map();
        this.renderedSummary = {
            pointCount: 0,
            totalBytes: 0,
            maxBytes: 0,
            lastUpdatedLabel: "Waiting for data",
        };
        this.autoRotate = false;
        this.autoRefresh = true;
        this.colorMode = "rtt";
        this.maxPoints = 1000;
        this.userInteracting = false;
        this.interactionTimer = null;
        this.reclusterTimer = null;
        this.themeObserver = null;
        this.lastKnownAltitude = DEFAULT_POINT_OF_VIEW.altitude;
        this.activeClusterRadius = clusterRadiusPx(DEFAULT_POINT_OF_VIEW.altitude);
        this.selectedMeta = null;

        this.initGlobe();
        this.observeThemeChanges();
        void this.loadCountries();
    }

    initGlobe() {
        this.globe = window.Globe()(this.dom)
            .width(this.dom.clientWidth || 960)
            .height(this.dom.clientHeight || 640)
            .backgroundColor("rgba(0,0,0,0)")
            .showAtmosphere(true)
            .polygonsTransitionDuration(0)
            .polygonAltitude("altitude")
            .polygonCapColor("capColor")
            .polygonSideColor("sideColor")
            .polygonStrokeColor("strokeColor")
            .labelLat("lat")
            .labelLng("lng")
            .labelAltitude("altitude")
            .labelText("labelText")
            .labelSize("labelSize")
            .labelColor("labelColor")
            .labelResolution(2)
            .labelIncludeDot("includeDot")
            .labelDotRadius("dotRadius")
            .labelDotOrientation(() => "bottom")
            .labelLabel((point) => this.buildTooltipHtml(point?.meta))
            .onLabelClick((point) => this.pinDetails(point?.meta || null))
            .labelsTransitionDuration(0)
            .pointsData([])
            .polygonsData([])
            .labelsData([]);

        this.controls = this.globe.controls();
        this.controls.enablePan = false;
        this.controls.enableDamping = false;
        this.controls.rotateSpeed = 0.75;
        this.controls.zoomSpeed = 1.5;
        this.controls.minDistance = 120;
        this.controls.maxDistance = 420;
        this.controls.autoRotate = this.autoRotate;
        this.controls.autoRotateSpeed = 0.45;

        this.globe.pointOfView(DEFAULT_POINT_OF_VIEW, 0);
        this.captureViewState();
        this.applyTheme();
        this.attachControlListeners();
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

    applyCountryLayer() {
        if (FLOW_MAP_DEBUG.disableBorders) {
            this.globe.polygonsData([]);
            return;
        }
        const theme = getThemePalette();
        const polygons = this.countryFeatures.map((feature) => ({
            ...feature,
            capColor: theme.landFill,
            sideColor: theme.landSide,
            strokeColor: theme.countryBorder,
            altitude: theme.countryAltitude,
        }));
        this.globe.polygonsData(polygons);
    }

    attachControlListeners() {
        if (!this.controls?.addEventListener) {
            return;
        }
        this.controls.addEventListener("start", () => {
            this.userInteracting = true;
            clearScheduledRefresh();
            this.scheduleInteractionRelease();
        });
        this.controls.addEventListener("change", () => {
            this.captureViewState();
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

    observeThemeChanges() {
        const observer = new MutationObserver(() => {
            this.applyTheme();
            if (this.rawPayload.length > 0) {
                this.renderData(this.rawPayload);
            }
            updateLegend(this.colorMode, this.renderedSummary);
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

    buildTooltipHtml(meta) {
        if (!meta) {
            return "";
        }
        const theme = getThemePalette();
        if (meta.isCluster) {
            const preview = meta.members.length > 0
                ? `<div style="margin-top:6px; opacity:0.84">${meta.members.map((member) => {
                    const metric = this.colorMode === "traffic"
                        ? `${escapeHtml(member.bytes)}`
                        : `${escapeHtml(member.rtt)}`;
                    return `${escapeHtml(member.label)} (${metric})`;
                }).join("<br>")}</div>`
                : "";
            const overflow = meta.overflowCount > 0
                ? `<div style="margin-top:4px; opacity:0.78">+${meta.overflowCount} more endpoints</div>`
                : "";
            return `
                <div style="background:${theme.tooltipBg};color:${theme.tooltipText};border:1px solid ${theme.tooltipBorder};padding:10px 12px;border-radius:12px;max-width:300px">
                    <strong>${escapeHtml(meta.label)}</strong><br>
                    ${escapeHtml(meta.colorLabel)}: ${escapeHtml(meta.colorValue)}<br>
                    Traffic: ${escapeHtml(meta.bytes)}<br>
                    Center: ${meta.lat.toFixed(2)}, ${meta.lng.toFixed(2)}
                    ${preview}
                    ${overflow}
                </div>
            `;
        }

        return `
            <div style="background:${theme.tooltipBg};color:${theme.tooltipText};border:1px solid ${theme.tooltipBorder};padding:10px 12px;border-radius:12px;max-width:260px">
                <strong>${escapeHtml(meta.label)}</strong><br>
                ${escapeHtml(meta.colorLabel)}: ${escapeHtml(meta.colorValue)}<br>
                Traffic: ${escapeHtml(meta.bytes)}<br>
                Coordinates: ${meta.lat.toFixed(2)}, ${meta.lng.toFixed(2)}
            </div>
        `;
    }

    preparePoints(payload) {
        const source = Array.isArray(payload) ? [...payload] : [];
        source.sort((a, b) => toNumber(b?.[3], 0) - toNumber(a?.[3], 0));
        const limited = this.maxPoints > 0 ? source.slice(0, this.maxPoints) : source;
        const basePoints = limited.map((row, index) => {
            const lat = toNumber(row?.[0], 0);
            const lng = toNumber(row?.[1], 0);
            const bytes = toNumber(row?.[3], 0);
            const rttNanos = toNumber(row?.[4], 0);
            const rttMs = rttNanos / 1_000_000;
            return {
                id: `endpoint:${index}:${lat.toFixed(3)}:${lng.toFixed(3)}:${String(row?.[2] || "")}`,
                lat,
                lng,
                rawBytes: bytes,
                rttMs,
                endpointLabel: row?.[2] ? String(row[2]) : `Endpoint ${lat.toFixed(2)}, ${lng.toFixed(2)}`,
            };
        });

        const clustered = this.clusterVisiblePoints(basePoints);

        const maxBytes = clustered.reduce((acc, point) => Math.max(acc, point.rawBytes), 0);
        const theme = getThemePalette();
        return clustered.map((point) => {
            const baseColor = this.colorMode === "traffic"
                ? trafficColor(point.rawBytes, maxBytes)
                : colorByRttMs(point.rttMs);
            const meta = {
                id: point.id,
                label: point.endpointLabel,
                bytes: `${scaleNumber(point.rawBytes)}B`,
                lat: point.lat,
                lng: point.lng,
                colorLabel: this.colorMode === "traffic" ? "Relative traffic" : "RTT",
                colorValue: this.colorMode === "traffic" ? `${scaleNumber(point.rawBytes)}B` : formatRtt(point.rttMs * 1_000_000),
                isCluster: point.isCluster,
                count: point.count,
                members: point.members,
                overflowCount: point.overflowCount,
            };
            const selected = metaMatches(this.selectedMeta, meta);
            const dotRadius = markerDotRadius(point.rawBytes, maxBytes, point.count, selected, this.colorMode);
            return {
                id: point.id,
                lat: point.lat,
                lng: point.lng,
                altitude: markerAltitude(point.rawBytes, maxBytes, point.count),
                labelText: "",
                labelSize: 0.01,
                labelColor: selected ? theme.selectedColor : markerDisplayColor(baseColor, point.rawBytes, maxBytes, this.colorMode, selected),
                includeDot: true,
                dotRadius,
                rawBytes: point.rawBytes,
                meta,
                isCluster: point.isCluster,
            };
        });
    }

    clusterVisiblePoints(basePoints) {
        if (basePoints.length === 0) {
            return [];
        }

        const totalTimer = profileStart("clusterVisiblePoints");
        const radiusPx = this.currentClusterSize();
        const visible = [];
        const hidden = [];
        const width = this.dom.clientWidth || 960;
        const height = this.dom.clientHeight || 640;
        const pointOfView = this.globe?.pointOfView?.() || DEFAULT_POINT_OF_VIEW;
        const focusDirection = latLngToUnitVector(
            toNumber(pointOfView.lat, DEFAULT_POINT_OF_VIEW.lat),
            toNumber(pointOfView.lng, DEFAULT_POINT_OF_VIEW.lng),
        );
        const projectTimer = profileStart("screen projection");

        basePoints.forEach((point) => {
            const pointDirection = latLngToUnitVector(point.lat, point.lng);
            if (dotProduct(pointDirection, focusDirection) <= 0.06) {
                hidden.push(point);
                return;
            }

            const screen = this.globe.getScreenCoords(point.lat, point.lng, 0.001);
            if (!screen || !Number.isFinite(screen.x) || !Number.isFinite(screen.y)) {
                hidden.push(point);
                return;
            }
            if (screen.x < -32 || screen.x > width + 32 || screen.y < -32 || screen.y > height + 32) {
                hidden.push(point);
                return;
            }
            visible.push({ ...point, screenX: screen.x, screenY: screen.y });
        });
        profileEnd(projectTimer, `(raw=${basePoints.length}, visible=${visible.length}, hidden=${hidden.length})`);

        const clusterTimer = profileStart("screen-grid clustering");
        const clustered = [];
        const buckets = new Map();

        visible.forEach((point) => {
            const cellX = Math.round(point.screenX / radiusPx);
            const cellY = Math.round(point.screenY / radiusPx);
            const key = `${cellX}:${cellY}`;
            const bucket = buckets.get(key) || [];
            bucket.push(point);
            buckets.set(key, bucket);
        });

        buckets.forEach((members) => {
            if (members.length === 1) {
                clustered.push({
                    ...members[0],
                    isCluster: false,
                    count: 1,
                    members: [],
                    overflowCount: 0,
                });
                return;
            }

            let latWeighted = 0;
            let lngWeighted = 0;
            let rawBytes = 0;
            let rttWeighted = 0;
            let strongestLabel = members[0].endpointLabel;
            let strongestBytes = members[0].rawBytes;
            const previewMembers = [];

            members.forEach((member) => {
                const weight = Math.max(member.rawBytes, 1);
                latWeighted += member.lat * weight;
                lngWeighted += member.lng * weight;
                rawBytes += member.rawBytes;
                rttWeighted += member.rttMs * weight;
                if (member.rawBytes >= strongestBytes) {
                    strongestBytes = member.rawBytes;
                    strongestLabel = member.endpointLabel;
                }
                if (previewMembers.length < CLUSTER_PREVIEW_LIMIT) {
                    previewMembers.push({
                        label: member.endpointLabel,
                        bytes: `${scaleNumber(member.rawBytes)}B`,
                        rtt: formatRtt(member.rttMs * 1_000_000),
                    });
                }
            });

            const weight = Math.max(rawBytes, 1);
            clustered.push({
                id: `cluster:${members[0].id}`,
                lat: latWeighted / weight,
                lng: lngWeighted / weight,
                rawBytes,
                rttMs: rttWeighted / weight,
                endpointLabel: `${members.length} nearby endpoints`,
                isCluster: true,
                count: members.length,
                members: previewMembers,
                overflowCount: Math.max(0, members.length - previewMembers.length),
                strongestLabel,
            });
        });
        profileEnd(clusterTimer, `(buckets=${buckets.size}, output=${clustered.length})`);

        profileEnd(totalTimer, `(final=${clustered.length})`);
        return clustered;
    }

    renderData(payload) {
        const renderTimer = profileStart("renderData");
        this.rawPayload = Array.isArray(payload) ? payload : [];
        const points = this.preparePoints(this.rawPayload);
        this.renderedPoints = points;
        this.renderedClusterCount = points.filter((point) => point.isCluster).length;
        this.pointIndex = new Map(points.map((point) => [point.id, point]));
        this.globe.pointsData([]);
        this.globe.labelsData(points);
        this.renderedSummary = {
            pointCount: points.length,
            totalBytes: points.reduce((acc, point) => acc + toNumber(point.rawBytes, 0), 0),
            maxBytes: this.rawPayload.reduce((acc, row) => Math.max(acc, toNumber(row?.[3], 0)), 0),
            lastUpdatedLabel: this.renderedSummary.lastUpdatedLabel,
        };
        this.refreshPinnedSelection(points);
        profileEnd(renderTimer, `(points=${points.length}, clusters=${this.renderedClusterCount})`);
    }

    updateSummary(pointCount, totalBytes, timestamp) {
        this.renderedSummary.pointCount = pointCount;
        this.renderedSummary.totalBytes = totalBytes;
        this.renderedSummary.lastUpdatedLabel = formatAge(timestamp);
    }

    setColorMode(mode) {
        this.colorMode = mode === "traffic" ? "traffic" : "rtt";
        this.renderData(this.rawPayload);
    }

    setMaxPoints(limit) {
        const parsed = parseInt(limit, 10);
        this.maxPoints = Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
        this.renderData(this.rawPayload);
    }

    setAutoRotate(enabled) {
        this.autoRotate = !!enabled;
        if (this.controls) {
            this.controls.autoRotate = this.autoRotate;
        }
    }

    resetView() {
        this.globe.pointOfView(DEFAULT_POINT_OF_VIEW, 700);
        this.lastKnownAltitude = DEFAULT_POINT_OF_VIEW.altitude;
        if (this.rawPayload.length > 0) {
            this.renderData(this.rawPayload);
        }
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
        if (this.interactionTimer) {
            clearTimeout(this.interactionTimer);
        }
        if (this.reclusterTimer) {
            clearTimeout(this.reclusterTimer);
        }
        if (this.themeObserver) {
            this.themeObserver.disconnect();
        }
    }

    pinDetails(meta) {
        this.selectedMeta = meta || null;
        if (this.rawPayload.length > 0) {
            this.renderData(this.rawPayload);
        }
        this.renderPinnedDetails();
    }

    clearPinnedDetails() {
        this.selectedMeta = null;
        if (this.rawPayload.length > 0) {
            this.renderData(this.rawPayload);
        }
        this.renderPinnedDetails();
    }

    refreshPinnedSelection(points) {
        if (!this.selectedMeta) {
            return;
        }
        const selected = points.find((point) => metaMatches(this.selectedMeta, point.meta));
        if (selected?.meta) {
            this.selectedMeta = selected.meta;
        } else {
            this.selectedMeta = null;
        }
        this.renderPinnedDetails();
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

        if (!this.selectedMeta) {
            panel.classList.remove("active");
            body.innerHTML = "";
            members.style.display = "none";
            members.textContent = "";
            return;
        }

        panel.classList.add("active");
        title.textContent = this.selectedMeta.label;
        subtitle.textContent = this.selectedMeta.isCluster ? "Pinned cluster details" : "Pinned endpoint details";
        body.innerHTML = renderDetailsBody(this.selectedMeta).map(([label, value]) => `
            <div class="flow-map-details-row">
                <span>${escapeHtml(label)}</span>
                <strong>${escapeHtml(value)}</strong>
            </div>
        `).join("");

        if (this.selectedMeta.isCluster && Array.isArray(this.selectedMeta.members) && this.selectedMeta.members.length > 0) {
            const overflow = this.selectedMeta.overflowCount > 0 ? `<div>+${this.selectedMeta.overflowCount} more endpoints</div>` : "";
            members.innerHTML = this.selectedMeta.members.map((member) => {
                if (member && typeof member === "object") {
                    const metric = this.colorMode === "traffic"
                        ? member.bytes
                        : member.rtt;
                    return `<div>${escapeHtml(member.label)}${metric ? ` (${escapeHtml(metric)})` : ""}</div>`;
                }
                return `<div>${escapeHtml(member)}</div>`;
            }).join("") + overflow;
            members.style.display = "block";
        } else {
            members.style.display = "none";
            members.textContent = "";
        }
    }
}

const map = new FlowMapGlobe("flowMap");
const overlay = makeOverlay(map.dom, "flowMapOverlay");
let updateTimer = null;
let pendingRequest = null;
let lastUpdateAt = 0;

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

function refreshHud() {
    map.updateSummary(map.renderedSummary.pointCount, map.renderedSummary.totalBytes, lastUpdateAt);
    updateLegend(map.colorMode, map.renderedSummary);
    updateHud(map.renderedSummary);
}

function scheduleNextRefresh() {
    if (!map.autoRefresh || document.hidden || updateTimer || pendingRequest || map.userInteracting) {
        return;
    }
    updateTimer = setTimeout(() => {
        updateTimer = null;
        refreshNow();
    }, AUTO_REFRESH_MS);
}

function clearScheduledRefresh() {
    if (updateTimer) {
        clearTimeout(updateTimer);
        updateTimer = null;
    }
}

function refreshNow() {
    if (pendingRequest) {
        return;
    }
    clearScheduledRefresh();
    syncRefreshButton(true);
    pendingRequest = listenOnceWithTimeout("FlowMap", RESPONSE_TIMEOUT_MS, (msg) => {
        pendingRequest = null;
        syncRefreshButton(false);
        const data = msg?.data || [];
        const totalBytes = data.reduce((acc, row) => acc + toNumber(row?.[3], 0), 0);
        lastUpdateAt = Date.now();

        if (data.length < MIN_POINTS || totalBytes < MIN_TOTAL_BYTES) {
            overlay.show("Insufficient data", "Not enough recent flow traffic to render the map yet.");
            map.renderData([]);
            map.updateSummary(0, totalBytes, lastUpdateAt);
        } else {
            overlay.hide();
            map.renderData(data);
            map.updateSummary(map.renderedSummary.pointCount, totalBytes, lastUpdateAt);
        }

        refreshHud();
        scheduleNextRefresh();
    }, () => {
        pendingRequest = null;
        syncRefreshButton(false);
        overlay.show("Waiting for data", "No FlowMap websocket response received yet.");
        map.renderData([]);
        map.updateSummary(0, 0, 0);
        refreshHud();
        scheduleNextRefresh();
    });
    wsClient.send({ FlowMap: {} });
}

function stopUpdates() {
    clearScheduledRefresh();
    if (pendingRequest) {
        pendingRequest.cancel();
        pendingRequest = null;
        syncRefreshButton(false);
    }
}

function resumeUpdates() {
    if (!pendingRequest && map.autoRefresh) {
        scheduleNextRefresh();
    }
}

function initControls() {
    const refreshBtn = document.getElementById("flowMapRefreshBtn");
    const resetBtn = document.getElementById("flowMapResetBtn");
    const rotateBtn = document.getElementById("flowMapRotateBtn");
    const autoRefresh = document.getElementById("flowMapAutoRefresh");
    const modeLatency = document.getElementById("flowMapModeLatency");
    const modeThroughput = document.getElementById("flowMapModeThroughput");
    const density = document.getElementById("flowMapDensity");
    const detailsClose = document.getElementById("flowMapDetailsClose");

    if (refreshBtn) {
        refreshBtn.addEventListener("click", () => refreshNow());
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
    if (modeLatency && modeThroughput) {
        const syncModeButtons = () => {
            const latencyActive = map.colorMode !== "traffic";
            modeLatency.classList.toggle("btn-primary", latencyActive);
            modeLatency.classList.toggle("btn-outline-secondary", !latencyActive);
            modeLatency.setAttribute("aria-pressed", latencyActive ? "true" : "false");
            modeThroughput.classList.toggle("btn-primary", !latencyActive);
            modeThroughput.classList.toggle("btn-outline-secondary", latencyActive);
            modeThroughput.setAttribute("aria-pressed", latencyActive ? "false" : "true");
        };
        syncModeButtons();
        modeLatency.addEventListener("click", () => {
            map.setColorMode("rtt");
            syncModeButtons();
            refreshHud();
        });
        modeThroughput.addEventListener("click", () => {
            map.setColorMode("traffic");
            syncModeButtons();
            refreshHud();
        });
    }
    if (density) {
        density.value = String(map.maxPoints);
        density.addEventListener("change", () => {
            map.setMaxPoints(density.value);
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
refreshHud();
refreshNow();
