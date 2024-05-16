export function regular_color_ramp(n) {
    if (n <= 100) {
        return "#aaffaa";
    } else if (n <= 150) {
        return "goldenrod";
    } else {
        return "#ffaaaa";
    }
}

export function rtt_display(rtt: number): string {
    return "<span style='color: " + regular_color_ramp(rtt) + "'>⬤</span> " + rtt.toFixed(1) + " ms";
}