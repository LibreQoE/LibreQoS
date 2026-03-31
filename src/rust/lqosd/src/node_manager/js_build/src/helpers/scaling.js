import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {scaleNanos} from "../lq_js_common/helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {isColorBlindMode} from "./colorblind";

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

function getRttThresholds() {
    const defaults = { greenMs: 0, yellowMs: 100, redMs: 200 };
    const obj = window.rttThresholds || window.rtt_thresholds || window.config?.rtt_thresholds;
    const g = toNumber(obj?.greenMs ?? obj?.green_ms ?? obj?.green, defaults.greenMs);
    const y = toNumber(obj?.yellowMs ?? obj?.yellow_ms ?? obj?.yellow, defaults.yellowMs);
    const r = toNumber(obj?.redMs ?? obj?.red_ms ?? obj?.red, defaults.redMs);

    const greenMs = Math.max(0, Math.round(g));
    const yellowMs = Math.max(greenMs, Math.round(y));
    const redMs = Math.max(yellowMs, Math.round(r), 1);
    return { greenMs, yellowMs, redMs };
}

function clamp01(x) {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    return x;
}

function colorByRttMs(rttMs) {
    const t = getRttThresholds();
    const raw = toNumber(rttMs, t.greenMs);
    const clamped = Math.min(t.redMs, Math.max(t.greenMs, raw));

    const frac = (clamped - t.greenMs) / Math.max(1, t.redMs - t.greenMs);
    if (isColorBlindMode()) {
        return lerpViridis(frac);
    }

    if (clamped <= t.yellowMs) {
        const w = clamp01((clamped - t.greenMs) / Math.max(1, t.yellowMs - t.greenMs));
        return lerpColor([0, 255, 0], [255, 255, 0], w);
    }
    const w = clamp01((clamped - t.yellowMs) / Math.max(1, t.redMs - t.yellowMs));
    return lerpColor([255, 255, 0], [255, 0, 0], w);
}

export function colorRamp(n) {
    n = toNumber(n, 0);
    if (n <= 100) {
        return "#aaffaa";
    } else if (n <= 150) {
        return "goldenrod";
    } else {
        return "#ffaaaa";
    }
}

export function rttCircleSpan(rtt) {
    let span = document.createElement("span");
    span.style.color = colorRamp(rtt);
    span.innerText = "⬤";
    return span;
}

export function lerpGreenToRedViaOrange(value, max) {
    value = toNumber(value, 0);
    max = toNumber(max, 0);
    let r = 0;
    let g = 0;
    let b = 0;
    if (value < max / 2) {
        r = 255;
        g = 255 - Math.floor(255 * (value / (max / 2)));
    } else {
        r = Math.floor(255 * ((max - value) / (max / 2)));
        g = 255;
    }
    return `rgb(${r}, ${g}, ${b})`;
}

export function formatThroughput(throughput, limitInMbps) {
    throughput = toNumber(throughput, 0);
    limitInMbps = toNumber(limitInMbps, 0);
    let limitBits = limitInMbps * 1000 * 1000;
    let percent = 0;
    if (limitBits > 0) {
        percent = (throughput / limitBits) * 100;
    }
    let color = lerpGreenToRedViaOrange(100-percent, 100);
    let blob = "<span class='muted' style='color: " + color + "'>■</span>";
    blob += "<span>" + scaleNumber(throughput, 1) + "bps</span>";
    return blob;
}

export function formatRtt(rtt) {
    if (rtt === undefined || rtt === null || rtt.nanoseconds === 0) {
        return "-";
    }
    rtt = toNumber(rtt, 0);
    let color = colorByRttMs(rtt);
    let blob = "<span class='muted' style='color: " + color + "'>■</span>";
    blob += "<span>" + parseFloat(rtt).toFixed(0) + "ms</span>";
    return blob;
}

export function retransmitFractionFromSample(sample) {
    const retransmits = toNumber(sample?.retransmits, 0);
    const packets = toNumber(sample?.packets, 0);
    if (packets <= 0) {
        return 0;
    }
    return retransmits / packets;
}

export function retransmitPercentFromSample(sample) {
    return retransmitFractionFromSample(sample) * 100.0;
}

export function formatRetransmitFraction(fraction) {
    const percent = toNumber(fraction, 0) * 100.0;
    const clampedPercent = Math.min(100, Math.max(0, percent));
    const color = lerpColor([0, 255, 0], [255, 0, 0], clampedPercent / 100.0);
    return "<span class='muted' style='color: " + color + "'>■</span>" + percent.toFixed(1) + "%</span>";
}

export function formatRetransmitPercent(percent) {
    percent = toNumber(percent, 0);
    const clampedPercent = Math.min(100, Math.max(0, percent));
    const color = lerpColor([0, 255, 0], [255, 0, 0], clampedPercent / 100.0);
    return "<span class='muted' style='color: " + color + "'>■</span>" + percent.toFixed(1) + "%</span>";
}

export function formatRetransmitCount(retransmits) {
    retransmits = toNumber(retransmits, 0);
    return "<span class='text-body-secondary'>" + retransmits.toFixed(0) + "</span>";
}

export function formatRetransmit(retransmits) {
    return formatRetransmitFraction(retransmits);
}

export function formatRetransmitRaw(retransmits) {
    return formatRetransmitCount(retransmits);
}

export function formatCakeStat(n) {
    n = toNumber(n, 0);
    let percent = Math.min(100, n) / 100;
    let color = lerpColor([128, 128, 0], [255, 255, 255], percent);
    let html = "<span class='muted' class='retransmits' style='color: " + color + "'>";
    html += n;
    html += "</span>";
    return html;
}

export function formatCakeStatPercent(n, packets) {
    n = toNumber(n, 0);
    packets = toNumber(packets, 0);
    if (packets === 0) {
        n = 0;
    } else {
        n = (n / packets);
    }
    let percent = Math.min(100, n) / 100;
    let color = lerpGreenToRedViaOrange(100-percent, 100);
    let html = "<span class='muted' class='retransmits' style='color: " + color + "'>";
    html += "■";
    html += "</span>";
    html += n.toFixed(2);
    html += "%";
    return html;
}

export function lerpColor(color1, color2, weight) {
    weight = toNumber(weight, 0);
    var r = Math.round(color1[0] + (color2[0] - color1[0]) * weight);
    var g = Math.round(color1[1] + (color2[1] - color1[1]) * weight);
    var b = Math.round(color1[2] + (color2[2] - color1[2]) * weight);
    return `rgb(${r}, ${g}, ${b})`;
}

export function formatPercent(percent, digits=0) {
    percent = toNumber(percent, 0);
    let color = lerpGreenToRedViaOrange(100-Math.min(100,percent), 100);
    return "<span class='muted' style='color: " + color + "'>" + percent.toFixed(digits) + "%</span>";
}

export function rttNanosAsSpan(rttNanos, precision=0) {
    rttNanos = toNumber(rttNanos, 0);
    let rttInMs = rttNanos / 1000000;
    let color = colorByRttMs(rttInMs);
    let html = "<span class='muted' style='color: " + color + "'>■</span> " + scaleNanos(rttNanos, precision);
    return html;
}

export function formatMbps(mbps) {
    mbps = toNumber(mbps, 0);
    // Format Mbps values with smart decimal display
    // Whole numbers: no decimals (e.g., "100 Mbps")
    // Fractional: show decimals with up to 2 decimal places (e.g., "2.5 Mbps", "0.25 Mbps")
    if (mbps === Math.floor(mbps)) {
        return mbps + " Mbps";
    } else {
        // Use up to 2 decimal places, removing trailing zeros
        return parseFloat(mbps.toFixed(2)) + " Mbps";
    }
}
