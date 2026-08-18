import assert from "node:assert/strict";
import test from "node:test";

import {
    applyMaxRateDisplay,
    formatMaxRatePair,
    hasUsableRatePair,
    ratePairsDiffer,
} from "./circuit_rate_display.mjs";

class FakeElement {
    constructor(parentElement = null) {
        this.parentElement = parentElement;
        this.attributes = {};
        this.textContent = "";
    }

    setAttribute(name, value) {
        this.attributes[name] = String(value);
    }

    removeAttribute(name) {
        delete this.attributes[name];
    }

    matches(selector) {
        return selector === '[data-bs-toggle="tooltip"]'
            && this.attributes["data-bs-toggle"] === "tooltip";
    }

    querySelectorAll() {
        return [];
    }
}

test("max rate display stays compact when effective rate matches assigned rate", () => {
    const assigned = {down: 115, up: 115};
    const effective = {down: 115, up: 115};

    assert.equal(formatMaxRatePair(assigned, effective), "115.0 / 115.0 Mbps");
    assert.equal(ratePairsDiffer(assigned, effective), false);
});

test("max rate display shows assigned to effective only when rates differ", () => {
    const assigned = {down: 115, up: 115};
    const effective = {down: 100, up: 50};

    assert.equal(formatMaxRatePair(assigned, effective), "115.0 / 115.0 -> 100.0 / 50.0 Mbps");
    assert.equal(ratePairsDiffer(assigned, effective), true);
});

test("unavailable effective rate falls back to assigned rate", () => {
    const assigned = {down: 115, up: 115};

    assert.equal(formatMaxRatePair(assigned, null), "115.0 / 115.0 Mbps");
    assert.equal(hasUsableRatePair(null), false);
});

test("rate comparison ignores differences smaller than display precision", () => {
    const assigned = {down: 115, up: 115};
    const effective = {down: 115.04, up: 115};

    assert.equal(ratePairsDiffer(assigned, effective), false);
    assert.equal(formatMaxRatePair(assigned, effective), "115.0 / 115.0 Mbps");
});

test("rate comparison changes when displayed precision changes", () => {
    const assigned = {down: 115, up: 115};
    const effective = {down: 115.06, up: 115};

    assert.equal(ratePairsDiffer(assigned, effective), true);
    assert.equal(formatMaxRatePair(assigned, effective), "115.0 / 115.0 -> 115.1 / 115.0 Mbps");
});

test("zero or invalid rates are treated as unavailable", () => {
    assert.equal(hasUsableRatePair({down: 0, up: 115}), false);
    assert.equal(hasUsableRatePair({down: 115, up: Number.NaN}), false);
});

test("max rate display exposes assigned and effective meaning accessibly", () => {
    const parent = new FakeElement();
    const maxRateEl = new FakeElement(parent);
    let tooltipRoot = null;

    applyMaxRateDisplay(
        maxRateEl,
        {down: 115, up: 115},
        {down: 100, up: 50},
        (root) => {
            tooltipRoot = root;
        },
    );

    assert.equal(maxRateEl.textContent, "115.0 / 115.0 -> 100.0 / 50.0 Mbps");
    assert.equal(maxRateEl.attributes["data-bs-toggle"], "tooltip");
    assert.equal(maxRateEl.attributes["data-bs-trigger"], "hover focus");
    assert.equal(maxRateEl.attributes.tabindex, "0");
    assert.equal(
        maxRateEl.attributes.title,
        "Assigned max 115.0 / 115.0 Mbps; effective queue max 100.0 / 50.0 Mbps",
    );
    assert.equal(
        maxRateEl.attributes["aria-label"],
        "Assigned max 115.0 / 115.0 Mbps. Effective queue max 100.0 / 50.0 Mbps.",
    );
    assert.equal(tooltipRoot, maxRateEl);
});

test("max rate display removes tooltip state when rates match", () => {
    const maxRateEl = new FakeElement();
    maxRateEl.setAttribute("title", "old");
    maxRateEl.setAttribute("aria-label", "old");
    maxRateEl.setAttribute("data-bs-toggle", "tooltip");
    maxRateEl.setAttribute("data-bs-trigger", "hover focus");
    maxRateEl.setAttribute("tabindex", "0");
    maxRateEl.setAttribute("data-bs-original-title", "old");

    applyMaxRateDisplay(maxRateEl, {down: 115, up: 115}, {down: 115, up: 115});

    assert.equal(maxRateEl.textContent, "115.0 / 115.0 Mbps");
    assert.equal(maxRateEl.attributes.title, undefined);
    assert.equal(maxRateEl.attributes["aria-label"], undefined);
    assert.equal(maxRateEl.attributes["data-bs-toggle"], undefined);
    assert.equal(maxRateEl.attributes["data-bs-trigger"], undefined);
    assert.equal(maxRateEl.attributes.tabindex, undefined);
    assert.equal(maxRateEl.attributes["data-bs-original-title"], undefined);
});

test("max rate display disposes stale Bootstrap tooltip before removing tooltip attributes", () => {
    const maxRateEl = new FakeElement();
    maxRateEl.setAttribute("data-bs-toggle", "tooltip");
    let disposed = false;
    globalThis.bootstrap = {
        Tooltip: {
            getInstance(el) {
                assert.equal(el, maxRateEl);
                return {
                    dispose() {
                        disposed = true;
                    },
                };
            },
        },
    };

    try {
        applyMaxRateDisplay(maxRateEl, {down: 115, up: 115}, {down: 115, up: 115});
    } finally {
        delete globalThis.bootstrap;
    }

    assert.equal(disposed, true);
});
