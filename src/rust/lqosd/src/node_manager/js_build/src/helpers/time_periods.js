export function periodNameToSeconds(period) {
    switch (period) { // Convert to seconds
        case "1h": return 3600;
        case "6h": return 21600;
        case "12h": return 43200;
        case "24h": return 86400;
        case "7d": return 604800;
        default:
            console.log("Unknown period: " + period);
            return 0;
    }
}