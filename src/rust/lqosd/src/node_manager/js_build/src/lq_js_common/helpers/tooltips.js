function tooltipElementsWithin(rootEl = document) {
    if (!rootEl) {
        return [];
    }

    const elements = [];
    if (rootEl.matches && rootEl.matches('[data-bs-toggle="tooltip"]')) {
        elements.push(rootEl);
    }

    if (rootEl.querySelectorAll) {
        elements.push(...rootEl.querySelectorAll('[data-bs-toggle="tooltip"]'));
    }

    return elements;
}

export function enableTooltipsWithin(rootEl = document) {
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) {
        return;
    }

    const Tooltip = bootstrap.Tooltip;
    tooltipElementsWithin(rootEl).forEach((el) => {
        if (!el) {
            return;
        }
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

export function disposeTooltipsWithin(rootEl = document) {
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip || !bootstrap.Tooltip.getInstance) {
        return;
    }

    tooltipElementsWithin(rootEl).forEach((el) => {
        const existing = bootstrap.Tooltip.getInstance(el);
        if (existing && existing.dispose) {
            existing.dispose();
        }
    });
}
