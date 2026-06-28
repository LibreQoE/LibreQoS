export const MAPLIBRE_SCRIPT_SRC = "vendor/maplibre-gl.js";
export const MAPLIBRE_STYLESHEET_HREF = "vendor/maplibre-gl.css";

let mapLibrePromise = null;

export function ensureMapLibreAssets() {
    if (typeof window !== "undefined" && window.maplibregl?.Map) {
        ensureMapLibreStylesheet();
        return Promise.resolve();
    }
    if (mapLibrePromise) {
        return mapLibrePromise;
    }
    ensureMapLibreStylesheet();
    mapLibrePromise = new Promise((resolve, reject) => {
        const existing = document.querySelector(`script[src="${MAPLIBRE_SCRIPT_SRC}"]`);
        if (existing) {
            if (window.maplibregl?.Map) {
                resolve();
                return;
            }
            existing.addEventListener("load", () => resolve(), { once: true });
            existing.addEventListener("error", () => reject(new Error("Unable to load MapLibre")), { once: true });
            return;
        }
        const script = document.createElement("script");
        script.src = MAPLIBRE_SCRIPT_SRC;
        script.onload = () => resolve();
        script.onerror = () => reject(new Error("Unable to load MapLibre"));
        document.head.appendChild(script);
    }).catch((err) => {
        mapLibrePromise = null;
        throw err;
    });
    return mapLibrePromise;
}

export function ensureMapLibreStylesheet() {
    if (document.querySelector(`link[href="${MAPLIBRE_STYLESHEET_HREF}"]`)) {
        return;
    }
    const link = document.createElement("link");
    link.rel = "stylesheet";
    link.href = MAPLIBRE_STYLESHEET_HREF;
    document.head.appendChild(link);
}

export function resetMapLibreAssetLoaderForTest() {
    mapLibrePromise = null;
}
