export function scaleNumber(n: number, decimals: number = 2) {
    if (n > 1000000000000) {
        return (n / 1000000000000).toFixed(decimals) + "T";
    } else if (n > 1000000000) {
        return (n / 1000000000).toFixed(decimals) + "G";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(decimals) + "M";
    } else if (n > 1000) {
        return (n / 1000).toFixed(decimals) + "K";
    }
    return n;
}