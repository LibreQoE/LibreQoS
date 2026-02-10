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

// Color by RTT ms with a soft cap (default 200ms).
export function colorByRttMs(rttMs, capMs = 200) {
    const r = Math.min(capMs, Math.max(0, Number(rttMs) || 0));
    if (isColorBlindMode()) {
        return lerpViridis(r / capMs);
    } else {
        return lerpGreenToRedViaOrange(capMs - r, capMs);
    }
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
