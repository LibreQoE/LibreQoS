import {isColorBlindMode} from "./colorblind";
import {lerpColor, lerpGreenToRedViaOrange} from "./scaling";

// Viridis color scale interpolation (0..1)
function lerpViridis(t) {
    const stops = [
        [68, 1, 84],    // #440154
        [59, 82, 139],  // #3B528B
        [33, 145, 140], // #21918C
        [94, 201, 98],  // #5EC962
        [253, 231, 37], // #FDE725
    ];
    if (t <= 0) return "#440154";
    if (t >= 1) return "#FDE725";
    let idx = t * (stops.length - 1);
    let i = Math.floor(idx);
    let frac = idx - i;
    let c0 = stops[i], c1 = stops[i + 1];
    let r = Math.round(c0[0] + frac * (c1[0] - c0[0]));
    let g = Math.round(c0[1] + frac * (c1[1] - c0[1]));
    let b = Math.round(c0[2] + frac * (c1[2] - c0[2]));
    return "#" + ((1 << 24) + (r << 16) + (g << 8) + b).toString(16).slice(1);
}

// Color by retransmit percentage (0..10).
// Normal mode: green->red by lower is better.
// Color blind mode: viridis gradient (0..1) where higher moves toward yellow.
export function colorByRetransmitPct(pct0to10) {
    const p = Math.min(10, Math.max(0, pct0to10));
    if (isColorBlindMode()) {
        return lerpViridis(p / 10.0);
    } else {
        return lerpGreenToRedViaOrange(10 - p, 10);
    }
}

function resolveRttThresholds(scale) {
    const defaults = { greenMs: 0, yellowMs: 100, redMs: 200 };

    // Back-compat: if a number is provided, treat it as a cap (red point).
    if (typeof scale === "number" && Number.isFinite(scale) && scale > 0) {
        const red = Math.round(scale);
        const yellow = Math.round(red / 2);
        return { greenMs: 0, yellowMs: yellow, redMs: red };
    }

    // Explicit thresholds object.
    const fromObj = (obj) => {
        if (!obj || typeof obj !== "object") return null;
        const g = Number(obj.greenMs ?? obj.green_ms ?? obj.green ?? defaults.greenMs);
        const y = Number(obj.yellowMs ?? obj.yellow_ms ?? obj.yellow ?? defaults.yellowMs);
        const r = Number(obj.redMs ?? obj.red_ms ?? obj.red ?? defaults.redMs);
        if (!Number.isFinite(g) || !Number.isFinite(y) || !Number.isFinite(r)) return null;
        return { greenMs: Math.max(0, Math.round(g)), yellowMs: Math.max(0, Math.round(y)), redMs: Math.max(0, Math.round(r)) };
    };

    const fromScale = fromObj(scale);
    const fromWindow = fromObj(window.rttThresholds || window.rtt_thresholds || window.config?.rtt_thresholds);
    const t = fromScale || fromWindow || defaults;

    // Normalize ordering.
    const greenMs = t.greenMs;
    const yellowMs = Math.max(t.yellowMs, greenMs);
    const redMs = Math.max(t.redMs, yellowMs, 1);
    return { greenMs, yellowMs, redMs };
}

function clamp01(x) {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    return x;
}

// Color by RTT ms using a 3-point ramp (green/yellow/red).
export function colorByRttMs(rttMs, scale = null) {
    const t = resolveRttThresholds(scale);
    const raw = Number(rttMs);
    const r = Number.isFinite(raw) ? raw : 0;

    // Clamp into [green..red] for continuous scales.
    const minV = t.greenMs;
    const maxV = t.redMs;
    const clamped = Math.min(maxV, Math.max(minV, r));
    const denom = Math.max(1, maxV - minV);
    const frac = (clamped - minV) / denom;

    if (isColorBlindMode()) {
        return lerpViridis(frac);
    }

    // Non-colorblind: piecewise interpolate green->yellow->red.
    if (t.yellowMs <= t.greenMs) {
        // Degenerate: no green->yellow region; go yellow->red.
        const w = clamp01((clamped - t.yellowMs) / Math.max(1, t.redMs - t.yellowMs));
        return lerpColor([255, 255, 0], [255, 0, 0], w);
    }
    if (t.redMs <= t.yellowMs) {
        // Degenerate: no yellow->red region; go green->yellow.
        const w = clamp01((clamped - t.greenMs) / Math.max(1, t.yellowMs - t.greenMs));
        return lerpColor([0, 255, 0], [255, 255, 0], w);
    }

    if (clamped <= t.yellowMs) {
        const w = clamp01((clamped - t.greenMs) / Math.max(1, t.yellowMs - t.greenMs));
        return lerpColor([0, 255, 0], [255, 255, 0], w);
    }
    const w = clamp01((clamped - t.yellowMs) / Math.max(1, t.redMs - t.yellowMs));
    return lerpColor([255, 255, 0], [255, 0, 0], w);
}

// Color by QoQ score (0..100, higher is better).
export function colorByQoqScore(score0to100) {
    // Distinguish "no data" from a real 0 score (which is legitimately bad).
    if (score0to100 === null || score0to100 === undefined) {
        return "var(--bs-border-color)";
    }
    const raw = Number(score0to100);
    if (!Number.isFinite(raw)) {
        return "var(--bs-border-color)";
    }
    const s = Math.min(100, Math.max(0, raw));
    if (isColorBlindMode()) {
        return lerpViridis(s / 100.0);
    } else {
        return lerpColor([255, 0, 0], [0, 255, 0], s / 100.0);
    }
}
