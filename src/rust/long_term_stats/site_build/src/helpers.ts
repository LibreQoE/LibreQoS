export function getValueFromForm(id: string): string {
    let input = document.getElementById(id) as  HTMLInputElement;
    if (input) {
        return input.value;
    }
    return "";
}

export function scaleNumber(n: any, decimals: number = 1): string {
    if (n >= 1000000000000) {
        return (n / 1000000000000).toFixed(decimals) + "T";
    } else if (n >= 1000000000) {
        return (n / 1000000000).toFixed(decimals) + "G";
    } else if (n >= 1000000) {
        return (n / 1000000).toFixed(decimals) + "M";
    } else if (n >= 1000) {
        return (n / 1000).toFixed(decimals) + "K";
    }
    return n;
}

export function siteIcon(type: string): string {
    switch (type) {
        case "circuit": return "<i class='fa fa-user'></i>"; break;
        case "site": return "<i class='fa fa-building'></i>"; break;
        case "ap": return "<i class='fa fa-wifi'></i>"; break;
        default: return "<i class='fa fa-question'></i>";
    }
}

export function usageColor(percent: number): string {
    if (percent > 50 && percent < 75) {
        return "goldenrod";
    } else if (percent >= 75) {
        return "#ffaaaa";
    }
    return "#aaffaa";
}

export function rttColor(n: number): string {
    if (n <= 100) {
        return "#aaffaa";
    } else if (n <= 150) {
        return "goldenrod";
    } else {
        return "#ffaaaa";
    }
}

export function makeUrl(type: string, id: string): string {
    switch (type) {
        case "site": return "site:" + id;
        case "ap": return "ap:" + id;
        case "circuit": return "circuit:" + id;
        default: return "site:" + id;
    }
}