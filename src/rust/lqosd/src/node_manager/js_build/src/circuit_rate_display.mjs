import {toNumber} from "./lq_js_common/helpers/scaling.js";
import {disposeTooltipsWithin} from "./lq_js_common/helpers/tooltips.js";

export function formatRatePairValue(ratePair) {
    const down = toNumber(ratePair?.down, 0).toFixed(1);
    const up = toNumber(ratePair?.up, 0).toFixed(1);
    return `${down} / ${up}`;
}

export function hasUsableRatePair(ratePair) {
    const down = toNumber(ratePair?.down, NaN);
    const up = toNumber(ratePair?.up, NaN);
    return Number.isFinite(down) && Number.isFinite(up) && down > 0 && up > 0;
}

function roundedTenths(value) {
    return Math.round(toNumber(value, 0) * 10);
}

export function ratePairsDiffer(left, right) {
    if (!hasUsableRatePair(left) || !hasUsableRatePair(right)) {
        return false;
    }
    return roundedTenths(left.down) !== roundedTenths(right.down)
        || roundedTenths(left.up) !== roundedTenths(right.up);
}

export function formatMaxRatePair(assignedRate, effectiveRate) {
    if (ratePairsDiffer(assignedRate, effectiveRate)) {
        return `${formatRatePairValue(assignedRate)} -> ${formatRatePairValue(effectiveRate)} Mbps`;
    }
    return `${formatRatePairValue(assignedRate)} Mbps`;
}

export function applyMaxRateDisplay(maxRateEl, assignedRate, effectiveRate, initTooltipsWithin = null) {
    if (!maxRateEl) {
        return;
    }
    maxRateEl.textContent = formatMaxRatePair(assignedRate, effectiveRate);
    disposeTooltipsWithin(maxRateEl);

    if (!ratePairsDiffer(assignedRate, effectiveRate)) {
        maxRateEl.removeAttribute("title");
        maxRateEl.removeAttribute("aria-label");
        maxRateEl.removeAttribute("data-bs-toggle");
        maxRateEl.removeAttribute("data-bs-placement");
        maxRateEl.removeAttribute("data-bs-trigger");
        maxRateEl.removeAttribute("tabindex");
        maxRateEl.removeAttribute("data-bs-original-title");
        return;
    }

    maxRateEl.setAttribute("data-bs-toggle", "tooltip");
    maxRateEl.setAttribute("data-bs-placement", "top");
    maxRateEl.setAttribute("data-bs-trigger", "hover focus");
    maxRateEl.setAttribute("tabindex", "0");
    maxRateEl.setAttribute(
        "title",
        `Assigned max ${formatRatePairValue(assignedRate)} Mbps; effective queue max ${formatRatePairValue(effectiveRate)} Mbps`,
    );
    maxRateEl.setAttribute(
        "aria-label",
        `Assigned max ${formatRatePairValue(assignedRate)} Mbps. Effective queue max ${formatRatePairValue(effectiveRate)} Mbps.`,
    );

    if (typeof initTooltipsWithin === "function") {
        initTooltipsWithin(maxRateEl);
    }
}
