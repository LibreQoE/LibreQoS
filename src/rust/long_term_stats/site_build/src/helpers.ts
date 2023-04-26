export function getValueFromForm(id: string): string {
    let input = document.getElementById(id) as  HTMLInputElement;
    if (input) {
        return input.value;
    }
    return "";
}

export function scaleNumber(n: any): string {
    if (n >= 1000000000000) {
        return (n / 1000000000000).toFixed(1) + "T";
    } else if (n >= 1000000000) {
        return (n / 1000000000).toFixed(1) + "G";
    } else if (n >= 1000000) {
        return (n / 1000000).toFixed(1) + "M";
    } else if (n >= 1000) {
        return (n / 1000).toFixed(1) + "K";
    }
    return n;
}