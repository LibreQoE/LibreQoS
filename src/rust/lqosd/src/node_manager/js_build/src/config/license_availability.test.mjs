// The availability alert is the License & Services page's licence funnel: the
// unlicensed notice names the shaping cap and links to lts_trial.html. Every
// branch of `license_availability.mjs` is asserted here, so a regression that
// drops the funnel link, retargets it, or leaks markup between states fails.
//
// The alert element is a plain object that models the DOM contract the helper
// relies on: writing `innerHTML` or `textContent` replaces all children, so
// each write clears the other's previous value.

import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import {formatMappedCircuitLimit, renderAvailabilityAlert} from "./license_availability.mjs";
import {clearCatalogs, EN, installCatalogs} from "../../i18n/test_catalogs.mjs";

// LtsStatus discriminants the server sends (lts2_sys/shared_types.rs):
// Invalid is the unlicensed state, Full is a licensed one.
const STATE_INVALID = 0;
const STATE_FULL = 5;

/** A stand-in for `#licenseAvailabilityAlert`, hidden as the markup ships it. */
function fakeAlert() {
    const classes = new Set(["d-none"]);
    let html = "";
    let text = "";
    return {
        classList: {
            add: (name) => classes.add(name),
            remove: (name) => classes.delete(name),
            contains: (name) => classes.has(name),
        },
        get innerHTML() {
            return html;
        },
        set innerHTML(value) {
            html = String(value);
            text = "";
        },
        get textContent() {
            return text;
        },
        set textContent(value) {
            text = String(value);
            html = "";
        },
    };
}

// The unlicensed shape `current_capabilities` reports: no entitlement, so the
// plain branches cannot fire beside it.
const UNLICENSED = {
    license_state: STATE_INVALID,
    authority_label: "Unlicensed",
    control_service_reachable: false,
    bootstrap_intent: false,
    bootstrap_suppressed: false,
    can_use_support_tickets: false,
    can_use_chatbot: false,
    mapped_circuit_limit: 1000,
};

test.beforeEach(() => {
    installCatalogs();
});

test.afterEach(() => {
    clearCatalogs();
});

test("the limit row formats a number and names an absent cap", () => {
    assert.equal(formatMappedCircuitLimit(2500), "2,500");
    assert.equal(formatMappedCircuitLimit(null), EN.messages["config.lts.status.unlimited"]);
    assert.equal(formatMappedCircuitLimit(undefined), EN.messages["config.lts.status.unlimited"]);
});

test("unlicensed with a numeric limit renders the cap notice with the funnel link", () => {
    const alert = fakeAlert();
    renderAvailabilityAlert(alert, {...UNLICENSED, mapped_circuit_limit: 2500});
    assert.ok(alert.innerHTML.includes("2,500"), "the notice must name the formatted limit");
    assert.ok(
        alert.innerHTML.includes("href=\"lts_trial.html\""),
        "the notice must link to the licence channel entry page",
    );
    assert.equal(alert.classList.contains("d-none"), false, "the notice must be visible");

    // Branch order: a payload that would also match the control-unavailable
    // branch still renders the cap notice, so the funnel cannot be masked.
    const masked = fakeAlert();
    renderAvailabilityAlert(masked, {...UNLICENSED, mapped_circuit_limit: 2500, can_use_support_tickets: true});
    assert.ok(masked.innerHTML.includes("href=\"lts_trial.html\""));
    assert.equal(masked.textContent, "", "the cap branch must not fall through to a sentence");
});

test("the cap decision reads license_state, never the authority label", () => {
    // The label is display text: any wording beside an Invalid state still funnels.
    const relabeled = fakeAlert();
    renderAvailabilityAlert(relabeled, {...UNLICENSED, authority_label: "Not the English token"});
    assert.ok(relabeled.innerHTML.includes("href=\"lts_trial.html\""));

    // And the English token alone cannot reach the branch from a licensed state.
    const licensed = fakeAlert();
    renderAvailabilityAlert(licensed, {...UNLICENSED, license_state: STATE_FULL});
    assert.equal(licensed.innerHTML, "", "a licensed state renders no cap notice");
    assert.equal(licensed.classList.contains("d-none"), true);
});

test("the default 1,000 limit renders the cap notice", () => {
    // A restated fixture, not a Rust-side fallback witness: lts2_sys's
    // load_config-failure payload reports the Invalid state with the
    // default 1,000-circuit limit, and this test proves only the JS-side
    // half — an unlicensed payload with that default limit still funnels
    // to the licence channel.
    const alert = fakeAlert();
    renderAvailabilityAlert(alert, {...UNLICENSED});
    assert.ok(alert.innerHTML.includes("1,000"), "the notice must name the default limit");
    assert.ok(alert.innerHTML.includes("href=\"lts_trial.html\""));
    assert.equal(alert.classList.contains("d-none"), false);
});

test("unlicensed without a numeric limit falls through to the plain branches", () => {
    const alert = fakeAlert();
    renderAvailabilityAlert(alert, {...UNLICENSED, mapped_circuit_limit: 2500});
    assert.ok(alert.innerHTML.includes("href=\"lts_trial.html\""));

    // A null or undefined cap means the plain branches take over, and the
    // funnel markup of the previous render must not survive the handover.
    for (const absent of [null, undefined]) {
        renderAvailabilityAlert(alert, {...UNLICENSED, mapped_circuit_limit: absent, can_use_support_tickets: true});
        assert.equal(alert.textContent, EN.messages["config.lts.alert.control_unavailable"]);
        assert.equal(alert.innerHTML, "", "the plain branch must leave no funnel markup behind");
        assert.equal(alert.classList.contains("d-none"), false);
    }

    renderAvailabilityAlert(alert, {...UNLICENSED, mapped_circuit_limit: null, bootstrap_suppressed: true});
    assert.equal(alert.textContent, EN.messages["config.lts.alert.bootstrap_suppressed"]);

    renderAvailabilityAlert(alert, {
        ...UNLICENSED,
        mapped_circuit_limit: null,
        bootstrap_intent: true,
        authority_label: "Bootstrap pending",
    });
    assert.equal(alert.textContent, EN.messages["config.lts.alert.bootstrap_pending"]);

    // No branch applies: the alert empties and hides again.
    renderAvailabilityAlert(alert, {...UNLICENSED, mapped_circuit_limit: null});
    assert.equal(alert.textContent, "");
    assert.equal(alert.innerHTML, "");
    assert.equal(alert.classList.contains("d-none"), true);
});

// The bundle runs DOM setup at import, so the wiring can only be checked
// from source text, like the page contracts. The fetchCapabilities error
// path fetches the same id on its own, so the lookup and the call are pinned
// as one adjacency inside renderAvailability.
test("the License & Services page wires the alert element to the helper", async () => {
    const page = await readFile(new URL("../config_lts.js", import.meta.url), "utf8");
    const start = page.indexOf("function renderAvailability(");
    assert.ok(start >= 0, "the page must keep the renderAvailability function");
    const body = page.slice(start, page.indexOf("function fetchCapabilities", start));
    assert.match(
        body,
        /const alert = document\.getElementById\("licenseAvailabilityAlert"\);[\s\S]*?renderAvailabilityAlert\(alert, capabilities\);/,
        "renderAvailability must fetch the #licenseAvailabilityAlert element and pass it to the helper",
    );
});
