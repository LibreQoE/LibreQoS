export function heading5Icon(icon, text) {
    let h5 = document.createElement("h5");
    h5.innerHTML = "<i class='fa fa-" + icon + "'></i> " + text;
    return h5;
}

export function enableTooltips() {
    // Tooltips everywhere! Make this idempotent to avoid leaking tooltip instances
    // on websocket reconnects or repeated dashboard setup cycles.
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) {
        return;
    }
    const Tooltip = bootstrap.Tooltip;
    const tooltipTriggerList = Array.prototype.slice.call(
        document.querySelectorAll('[data-bs-toggle="tooltip"]'),
    );
    tooltipTriggerList.forEach((el) => {
        if (!el) return;
        if (Tooltip.getOrCreateInstance) {
            Tooltip.getOrCreateInstance(el);
            return;
        }
        if (Tooltip.getInstance) {
            const existing = Tooltip.getInstance(el);
            if (existing && existing.dispose) {
                existing.dispose();
            }
        }
        new Tooltip(el);
    });
}
