export function getValueFromForm(id: string): string {
    let input = document.getElementById(id) as  HTMLInputElement;
    if (input) {
        return input.value;
    }
    return "";
}

export function scaleNumber(n: any): string {
    if (n > 1000000000000) {
        return (n / 1000000000000).toFixed(2) + "T";
    } else if (n > 1000000000) {
        return (n / 1000000000).toFixed(2) + "G";
    } else if (n > 1000000) {
        return (n / 1000000).toFixed(2) + "M";
    } else if (n > 1000) {
        return (n / 1000).toFixed(2) + "K";
    }
    return n;
}