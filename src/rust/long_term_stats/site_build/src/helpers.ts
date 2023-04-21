export function getValueFromForm(id: string): string {
    let input = document.getElementById(id) as  HTMLInputElement;
    if (input) {
        return input.value;
    }
    return "";
}