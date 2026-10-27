// `.64.4` (R1.3): unit tests for the SHIPPED Executive heatmap label builders.
//
// The builders live in the import-free `executive_heat_labels.mjs`, so this test
// needs no browser/WebSocket stubs (unlike `executive_utils.js`). It is wired
// into `js_build/test-node-manager.sh`.
import assert from "node:assert/strict";
import test from "node:test";

import {
    heatRowAriaLabel,
    heatRowsCaption,
    typeMonoMarkup,
} from "./executive_heat_labels.mjs";

test("typeMonoMarkup: title carries the type and the monogram is not aria-hidden", () => {
    const mono = typeMonoMarkup("Circuit");
    assert.match(mono, /class="exec-type-mono exec-type-mono--circuit"/);
    assert.match(mono, /title="Circuit"/);
    assert.ok(mono.includes(">C<"), `expected the C monogram: ${mono}`);
    assert.ok(
        !mono.includes("aria-hidden"),
        "the #12-required behaviour is NO aria-hidden on the monogram",
    );
});

test("typeMonoMarkup: unknown/Global/empty drop the visible chip", () => {
    assert.equal(typeMonoMarkup("Global"), "");
    assert.equal(typeMonoMarkup("ASN"), typeMonoMarkup("ASN")); // sanity
    assert.notEqual(typeMonoMarkup("ASN"), ""); // ASN still renders its chip
    assert.equal(typeMonoMarkup(""), "");
    assert.equal(typeMonoMarkup(undefined), "");
    assert.equal(typeMonoMarkup(null), "");
});

test("heatRowAriaLabel: preserves the badge WORD even when the chip is dropped", () => {
    assert.equal(
        heatRowAriaLabel("Circuit 1", "Circuit", "5 %"),
        "Circuit 1 Circuit latest 5 %",
    );
    // The RTT metric word keeps the exact legacy string.
    assert.equal(
        heatRowAriaLabel("Site 2", "Site", "12 ms", "RTT"),
        "Site 2 Site latest RTT 12 ms",
    );
    // An unknown badge (Global) renders NO chip, but the aria-label keeps the word.
    const globalRow = heatRowAriaLabel("Overall QoO", "Global", "0 / 0");
    assert.ok(globalRow.includes("Global"), globalRow);
    assert.equal(typeMonoMarkup("Global"), "");
});

test("heatRowsCaption: enumerates the exact rendered kind-set", () => {
    const kinds = (...entityKinds) => entityKinds.map((entity_kind) => ({entity_kind}));

    assert.equal(heatRowsCaption(kinds("Circuit")), "circuits");
    assert.equal(heatRowsCaption(kinds("Site")), "sites");
    assert.equal(heatRowsCaption(kinds("Asn")), "ASNs");
    assert.equal(heatRowsCaption(kinds("Site", "Circuit")), "sites & circuits");
    assert.equal(
        heatRowsCaption(kinds("Site", "Circuit", "Asn")),
        "sites, circuits & ASNs",
    );
    assert.equal(heatRowsCaption([]), "");
});

test("heatRowsCaption: an ASN-only panel can never claim a circuits caption", () => {
    const caption = heatRowsCaption([{entity_kind: "Asn"}]);
    assert.equal(caption, "ASNs");
    assert.ok(!caption.includes("circuits"), caption);
});
