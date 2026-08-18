import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {colorByRttMs} from "../helpers/color_scales";
import {isDarkMode} from "../helpers/dark_mode";
import {ensureMapLibreAssets} from "./world_map_assets.mjs";
import {applyWorldMapFeatureUpdate, WORLD_MAP_ENCODING_DESCRIPTION} from "./world_map_dashlet_state.mjs";
import {emptyFeatureCollection, rowsToEndpointFeatures} from "./world_map_model.mjs";

const WORLD_CENTER = [0, 18];
const WORLD_ZOOM = 0.85;
const ENDPOINT_SOURCE_ID = "dashboard-world-endpoints";
const ENDPOINT_LAYER_ID = "dashboard-world-endpoints-circle";
const ENDPOINT_HEAT_LAYER_ID = "dashboard-world-endpoints-heat";
const COASTLINE_SOURCE_ID = "dashboard-world-coastlines";
const COASTLINE_LAYER_ID = "dashboard-world-coastlines-line";
const BACKGROUND_LAYER_ID = "dashboard-world-background";

function mapThemePalette() {
    if (isDarkMode()) {
        return {
            background: "#101820",
            coastline: "#94a3b8",
            endpointStroke: "#111827",
        };
    }
    return {
        background: "#edf2f7",
        coastline: "#64748b",
        endpointStroke: "#ffffff",
    };
}

function worldMapStyle() {
    const palette = mapThemePalette();
    return {
        version: 8,
        sources: {
            [COASTLINE_SOURCE_ID]: {
                type: "geojson",
                data: "vendor/site_map_coastlines.geojson",
            },
            [ENDPOINT_SOURCE_ID]: {
                type: "geojson",
                data: emptyFeatureCollection(),
            },
        },
        layers: [
            {
                id: BACKGROUND_LAYER_ID,
                type: "background",
                paint: {
                    "background-color": palette.background,
                },
            },
            {
                id: COASTLINE_LAYER_ID,
                type: "line",
                source: COASTLINE_SOURCE_ID,
                paint: {
                    "line-color": palette.coastline,
                    "line-opacity": 0.74,
                    "line-width": [
                        "interpolate", ["linear"], ["zoom"],
                        0, 0.55,
                        3, 1.2,
                    ],
                },
            },
            {
                id: ENDPOINT_HEAT_LAYER_ID,
                type: "heatmap",
                source: ENDPOINT_SOURCE_ID,
                maxzoom: 5,
                paint: {
                    "heatmap-weight": ["interpolate", ["linear"], ["get", "weight"], 0, 0, 1, 1],
                    "heatmap-intensity": ["interpolate", ["linear"], ["zoom"], 0, 0.5, 4, 1.8],
                    "heatmap-radius": ["interpolate", ["linear"], ["zoom"], 0, 7, 4, 20],
                    "heatmap-opacity": ["interpolate", ["linear"], ["zoom"], 0, 0.68, 5, 0],
                    "heatmap-color": [
                        "interpolate", ["linear"], ["heatmap-density"],
                        0, "rgba(0,0,0,0)",
                        0.25, "rgba(79, 172, 254, 0.45)",
                        0.55, "rgba(124, 255, 178, 0.55)",
                        0.8, "rgba(253, 221, 96, 0.7)",
                        1, "rgba(255, 110, 118, 0.8)",
                    ],
                },
            },
            {
                id: ENDPOINT_LAYER_ID,
                type: "circle",
                source: ENDPOINT_SOURCE_ID,
                paint: {
                    "circle-color": ["get", "color"],
                    "circle-radius": ["get", "radius"],
                    "circle-opacity": 0.76,
                    "circle-stroke-color": palette.endpointStroke,
                    "circle-stroke-width": 0.75,
                },
            },
        ],
    };
}

export class WorldMapGraph {
    constructor(id) {
        this.dom = document.getElementById(id);
        this.map = null;
        this.themeObserver = null;
        this.resizeHandler = () => this.resize();
        this.pendingFeatures = emptyFeatureCollection();
        this.ready = false;
        this.disposed = false;
        if (!this.dom) {
            throw new Error(`WorldMapGraph: missing DOM element '${id}'`);
        }
        this.dom.classList.add("lqos-dashboard-map");
        this.dom.textContent = "";
        ensureMapLibreAssets()
            .then(() => this.initMap())
            .catch((err) => {
                if (this.disposed) return;
                this.dom.textContent = err?.message || "Unable to load map";
            });
    }

    initMap() {
        if (this.disposed || this.map || !window.maplibregl?.Map) {
            return;
        }
        this.map = new window.maplibregl.Map({
            container: this.dom,
            style: worldMapStyle(),
            center: WORLD_CENTER,
            zoom: WORLD_ZOOM,
            minZoom: 0,
            maxZoom: 5,
            attributionControl: false,
            interactive: true,
            renderWorldCopies: false,
        });
        this.map.scrollZoom.disable();
        this.map.dragRotate.disable();
        this.map.touchZoomRotate.disableRotation();
        this.map.addControl(new window.maplibregl.NavigationControl({ showCompass: false }), "bottom-right");
        const renderWhenReady = () => {
            if (this.disposed || !this.map || !this.map.getSource(ENDPOINT_SOURCE_ID)) {
                return;
            }
            this.ready = true;
            this.applyCanvasAccessibility();
            this.applyTheme();
            this.renderFeatures();
            this.map.resize();
        };
        this.map.on("load", renderWhenReady);
        window.addEventListener("resize", this.resizeHandler);
        this.observeThemeChanges();
        renderWhenReady();
    }

    applyCanvasAccessibility() {
        const canvas = this.dom.querySelector(".maplibregl-canvas");
        if (!canvas) {
            return;
        }
        const label = this.dom.getAttribute("aria-label");
        const describedBy = this.dom.getAttribute("aria-describedby");
        if (label) {
            canvas.setAttribute("aria-label", label);
        }
        if (describedBy) {
            canvas.setAttribute("aria-describedby", describedBy);
        }
    }

    observeThemeChanges() {
        if (this.themeObserver) {
            return;
        }
        this.themeObserver = new MutationObserver(() => this.applyTheme());
        this.themeObserver.observe(document.documentElement, {
            attributes: true,
            attributeFilter: ["data-bs-theme"],
        });
    }

    applyTheme() {
        if (!this.map) {
            return;
        }
        const palette = mapThemePalette();
        if (this.map.getLayer(BACKGROUND_LAYER_ID)) {
            this.map.setPaintProperty(BACKGROUND_LAYER_ID, "background-color", palette.background);
        }
        if (this.map.getLayer(COASTLINE_LAYER_ID)) {
            this.map.setPaintProperty(COASTLINE_LAYER_ID, "line-color", palette.coastline);
        }
        if (this.map.getLayer(ENDPOINT_LAYER_ID)) {
            this.map.setPaintProperty(ENDPOINT_LAYER_ID, "circle-stroke-color", palette.endpointStroke);
        }
    }

    updateFeatures(features) {
        this.pendingFeatures = {
            type: "FeatureCollection",
            features,
        };
        this.renderFeatures();
    }

    renderFeatures() {
        if (!this.ready || !this.map) {
            return;
        }
        const source = this.map.getSource(ENDPOINT_SOURCE_ID);
        if (source?.setData) {
            source.setData(this.pendingFeatures);
        }
    }

    resize() {
        if (this.map) {
            this.map.resize();
        }
    }

    dispose() {
        this.disposed = true;
        if (this.themeObserver) {
            this.themeObserver.disconnect();
            this.themeObserver = null;
        }
        window.removeEventListener("resize", this.resizeHandler);
        if (this.map) {
            this.map.remove();
            this.map = null;
        }
    }
}

export class ShaperWorldMapDashlet extends BaseDashlet {
    constructor(slot, title){
        super(slot);
        this._emptyId = this.id + "_empty";
        this._descriptionId = this.id + "_description";
        this._statusId = this.id + "_status";
        this._title = title;
    }
    canBeSlowedDown(){ return true; }
    title(){ return this._title; }
    tooltip(){ return `<h5>${this.title()}</h5><p>${WORLD_MAP_ENCODING_DESCRIPTION}</p>`; }
    subscribeTo(){ return ["EndpointLatLon"]; }
    buildContainer(){
        let b = super.buildContainer();
        const graph = this.graphDiv();
        graph.setAttribute("aria-label", this.title());
        graph.setAttribute("aria-describedby", `${this._descriptionId} ${this._statusId}`);
        const description = document.createElement("div");
        description.id = this._descriptionId;
        description.classList.add("visually-hidden");
        description.textContent = `World map showing recent endpoint locations. ${WORLD_MAP_ENCODING_DESCRIPTION}`;
        const status = document.createElement("div");
        status.id = this._statusId;
        status.classList.add("visually-hidden");
        status.setAttribute("role", "status");
        status.setAttribute("aria-live", "polite");
        b.appendChild(graph);
        b.appendChild(description);
        b.appendChild(status);
        return b;
    }
    setup(){
        this.traceRender("setup-start");
        this.graph = new WorldMapGraph(this.graphDivId());
        this.traceRender("setup-complete", { graphId: this.graphDivId() });
    }
    onTabActivated(){
        requestAnimationFrame(() => this.graph?.resize());
    }
    _setStatus(msg){
        const status = document.getElementById(this._statusId);
        if (status) status.textContent = msg;
    }
    _showEmpty(show, msg = "No recent endpoint location data"){
        const card = document.getElementById(this.id);
        if (!card) return;
        let empty = document.getElementById(this._emptyId);
        if (!empty) {
            empty = document.createElement('div');
            empty.id = this._emptyId;
            empty.classList.add('text-center','text-muted','small');
            empty.style.padding = '12px';
            card.appendChild(empty);
        }
        empty.textContent = msg;
        const graph = document.getElementById(this.graphDivId());
        if (show) {
            empty.style.display = '';
            if (graph) graph.style.display = 'none';
            this._setStatus("No recent endpoint location data.");
        } else {
            empty.style.display = 'none';
            if (graph) graph.style.display = '';
            this.graph?.resize();
        }
    }
    onMessage(msg){
        if (msg.event !== "EndpointLatLon") return;
        const rows = msg.data || [];
        const features = rowsToEndpointFeatures(rows, colorByRttMs);
        applyWorldMapFeatureUpdate(this, features, msg.event);
    }
}

export class ShaperWorldMapDown extends ShaperWorldMapDashlet {
    constructor(slot){ super(slot, "Shaper World Map (Download)"); }
}
