import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {scaleNanos} from "../lq_js_common/helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";

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
    const limit = 200;
    let percent = 0;
    if (limit > 0) {
        percent = (rtt / limit) * 100;
    }
    let color = lerpGreenToRedViaOrange(100-percent, 100);
    let blob = "<span class='muted' style='color: " + color + "'>■</span>";
    blob += "<span>" + parseFloat(rtt).toFixed(0) + "ms</span>";
    return blob;
}

export function formatRetransmit(retransmits) {
    retransmits = toNumber(retransmits, 0);
    retransmits *= 100;
    let percent = Math.min(100, retransmits);
    let color = lerpColor([0, 255, 0], [255, 0, 0], percent);
    return "<span class='muted' style='color: " + color + "'>■</span>" + retransmits.toFixed(1) + "%</span>";
}

export function formatRetransmitRaw(retransmits) {
    retransmits = toNumber(retransmits, 0);
    retransmits *= 100;
    let percent = Math.min(100, retransmits);
    let color = lerpColor([0, 255, 0], [255, 0, 0], percent);
    return "<span class='muted' style='color: " + color + "'>■</span>" + retransmits.toFixed(0) + "</span>";
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
    let rttInMs = Math.min(200, rttNanos / 1000000);
    let color = lerpGreenToRedViaOrange(200 - rttInMs, 200);
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
