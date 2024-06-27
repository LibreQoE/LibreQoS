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