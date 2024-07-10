export function scaleNumber(n, fixed=2) {
    if (n > 1000000000000) {
        return (n / 1000000000000).toFixed(fixed) + "T";
    } else if (n > 1000000000) {
        return (n / 1000000000).toFixed(fixed) + "G";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(fixed) + "M";
    } else if (n > 1000) {
        return (n / 1000).toFixed(fixed) + "K";
    }
    return n;
}

export function scaleNanos(n) {
    if (n == 0) return "";
    if (n > 1000000000) {
        return (n / 1000000000).toFixed(2) + "s";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(2) + "ms";
    } else if (n > 1000) {
        return (n / 1000).toFixed(2) + "µs";
    }
    return n + "ns";
}

export function colorRamp(n) {
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
    let r = 0;
    let g = 0;
    let b = 0;
    if (value < max / 2) {
        r = 255;
        g = Math.floor(255 * value / (max / 2));
    } else {
        r = Math.floor(255 * (max - value) / (max / 2));
        g = 255;
    }
    return `rgb(${r}, ${g}, ${b})`;
}

export function formatThroughput(throughput, limitInMbps) {
    let limitBits = limitInMbps * 1000 * 1000;
    let percent = 0;
    if (limitBits > 0) {
        percent = (throughput / limitBits) * 100;
    }
    let blob = "<span class='overlayThroughputWrapper'>";
    blob += "<span class='overlayThroughputBar'>";
    for (let i=0; i<100; i+=10) {
        let color = lerpGreenToRedViaOrange(100-i, 100);
        if (percent < i) {
            blob += "░";
        } else {
            blob += "<span style='color: " + color + "'>█</span>";
        }
    }
    blob += "</span>";

    blob += "<span class='overlayThroughputNumber' style='color: white; font-weight: bold;'>" + scaleNumber(throughput, 1) + "bps</span>";
    blob += "</span>";
    return blob;
}

export function formatRtt(rtt) {
    if (rtt === undefined) {
        return "-";
    }
    const limit = 200;
    let percent = 0;
    if (limit > 0) {
        percent = (rtt / limit) * 100;
    }
    let blob = "<span class='overlayThroughputWrapper'>";
    blob += "<span class='overlayThroughputBar'>";
    for (let i=0; i<100; i+=10) {
        let color = lerpGreenToRedViaOrange(100-i, 100);
        if (percent < i) {
            blob += "░";
        } else {
            blob += "<span style='color: " + color + "'>█</span>";
        }
    }
    blob += "</span>";

    blob += "<span class='overlayThroughputNumber' style='color: white; font-weight: bold;'>" + parseFloat(rtt).toFixed(0) + " ms</span>";
    blob += "</span>";
    return blob;
}

export function formatRetransmit(retransmits) {
    let percent = Math.min(100, retransmits) / 100;
    let color = lerpColor([0, 255, 0], [255, 0, 0], percent);
    let html = "<span class='retransmits' style='color: " + color + "'>";
    html += retransmits;
    html += "</span>";
    return html;
}

export function formatCakeStat(n) {
    let percent = Math.min(100, n) / 100;
    let color = lerpColor([128, 128, 0], [255, 255, 255], percent);
    let html = "<span class='retransmits' style='color: " + color + "'>";
    html += n;
    html += "</span>";
    return html;
}

export function lerpColor(color1, color2, weight) {
    var r = Math.round(color1[0] + (color2[0] - color1[0]) * weight);
    var g = Math.round(color1[1] + (color2[1] - color1[1]) * weight);
    var b = Math.round(color1[2] + (color2[2] - color1[2]) * weight);
    return `rgb(${r}, ${g}, ${b})`;
}