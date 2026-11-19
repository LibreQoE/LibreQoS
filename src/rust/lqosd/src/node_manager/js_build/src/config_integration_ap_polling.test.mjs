import assert from "node:assert/strict";
import {readFile} from "node:fs/promises";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {
    AP_POLLING_UNAVAILABLE_KEYS,
    capacitySourceBadge,
    driftClass,
    driftText,
    intervalValid,
    isUnavailable,
    statusBadge,
    summaryCounts,
    unavailableKey,
    validateFactorRows,
} from "./config/ap_polling.mjs";
import {clearCatalogs, EN, ES, installCatalogs} from "../i18n/test_catalogs.mjs";

const sourceDirectory = new URL(".", import.meta.url);
const configSourcePath = fileURLToPath(new URL("config_integration.js", sourceDirectory));
const configPagePath = fileURLToPath(new URL("../../static2/config_integration.html", sourceDirectory));

test.beforeEach(() => installCatalogs());
test.afterEach(clearCatalogs);

function assertCatalogKey(key) {
    assert.equal(typeof EN.messages[key], "string", `en.json defines ${key}`);
    assert.equal(typeof ES.messages[key], "string", `es.json defines ${key}`);
}

test("every endpoint unavailable reason maps to an explicit catalog message", () => {
    for (const reason of Object.keys(AP_POLLING_UNAVAILABLE_KEYS)) {
        const key = unavailableKey([reason]);
        assert.equal(key, AP_POLLING_UNAVAILABLE_KEYS[reason]);
        assertCatalogKey(key);
    }
    assert.equal(
        unavailableKey(["a_reason_this_build_does_not_know"]),
        "config.integration.ap_polling.unavailable.unknown",
    );
    assertCatalogKey("config.integration.ap_polling.unavailable.unknown");
    assert.equal(unavailableKey([], true), AP_POLLING_UNAVAILABLE_KEYS.request_failed);
});

test("an unavailable document is explicit, never an empty zeroed table", () => {
    assert.equal(isUnavailable(null), true);
    assert.equal(isUnavailable(undefined), true);
    assert.equal(
        isUnavailable({state: "unavailable", unavailable_reasons: ["ap_polling_disabled"], devices: null, aggregate: null}),
        true,
    );
    assert.equal(
        isUnavailable({state: "partial", unavailable_reasons: ["devices_failing"], devices: [], aggregate: {}}),
        false,
    );
});

test("drift renders a sign and never a fake zero for an unknown value", () => {
    assert.equal(driftText(3), "+3");
    assert.equal(driftText(0), "0");
    assert.equal(driftText(-2), "-2");
    assert.equal(driftText(2.9), "+2");
    assert.equal(driftText(null), "—");
    assert.equal(driftText(undefined), "—");
    assert.equal(driftText("not-a-number"), "—");
    assert.equal(driftClass(2), "text-success");
    assert.equal(driftClass(-2), "text-warning");
    assert.equal(driftClass(0), "");
    assert.equal(driftClass(null), "");
});

test("the capacity-source badge falls back to unknown and its title is cataloged", () => {
    for (const source of ["phy", "per_sm_rate", "crm", "unknown"]) {
        const badge = capacitySourceBadge(source);
        assert.equal(badge.token, source);
        assert.match(badge.className, /^text-bg-/);
        assertCatalogKey(badge.titleKey);
    }
    const fallback = capacitySourceBadge("vendor_private_value");
    assert.equal(fallback.token, "unknown");
    assert.equal(fallback.titleKey, "config.integration.ap_polling.source.unknown");
});

test("the state badge labels are cataloged and unknown states stay observing", () => {
    for (const [token, labelKey] of [
        ["ok", "config.integration.ap_polling.state.ok"],
        ["failing", "config.integration.ap_polling.state.failing"],
        ["observing", "config.integration.ap_polling.state.observing"],
    ]) {
        const badge = statusBadge(token);
        assert.equal(badge.token, token);
        assert.equal(badge.labelKey, labelKey);
        assertCatalogKey(labelKey);
    }
    assert.equal(statusBadge("mystery").token, "observing");
});

test("the interval accepts only whole seconds at the config floor", () => {
    assert.equal(intervalValid(30), true);
    assert.equal(intervalValid("60"), true);
    assert.equal(intervalValid(29), false);
    assert.equal(intervalValid(30.5), false);
    assert.equal(intervalValid(""), false);
    assert.equal(intervalValid(null), false);
});

test("the factor editor folds rows into the config map", () => {
    const result = validateFactorRows([
        {model: " ePMP 4500L ", factor: "0.7"},
        {model: "PowerBeam 5AC", factor: 1},
        {model: "", factor: ""},
    ]);
    assert.equal(result.ok, true);
    assert.deepEqual(result.factors, {"ePMP 4500L": 0.7, "PowerBeam 5AC": 1});
});

test("the factor editor rejects blank models, duplicates and out-of-range values", () => {
    assert.equal(
        validateFactorRows([{model: "", factor: "0.5"}]).errorKey,
        "config.integration.ap_polling.validation.factor_model",
    );
    for (const factor of ["0", "-0.1", "1.01", "not-a-number", ""]) {
        assert.equal(
            validateFactorRows([{model: "m", factor}]).errorKey,
            "config.integration.ap_polling.validation.factor_range",
            `factor ${JSON.stringify(factor)} is rejected`,
        );
    }
    assert.equal(
        validateFactorRows([{model: "m", factor: "0.5"}, {model: " m ", factor: "0.6"}]).errorKey,
        "config.integration.ap_polling.validation.factor_duplicate",
    );
    for (const key of [
        "config.integration.ap_polling.validation.factor_model",
        "config.integration.ap_polling.validation.factor_range",
        "config.integration.ap_polling.validation.factor_duplicate",
        "config.integration.ap_polling.validation.interval",
    ]) {
        assertCatalogKey(key);
    }
});

test("the summary counts tolerate a missing aggregate", () => {
    assert.deepEqual(summaryCounts(undefined), {total: 0, ok: 0, failing: 0, observing: 0});
    assert.deepEqual(
        summaryCounts({devices_total: 3, ok: 2, failing: 1, observing: 0}),
        {total: 3, ok: 2, failing: 1, observing: 0},
    );
    assertCatalogKey("config.integration.ap_polling.status.summary");
});

test("the page renders the panel and the bundle reads the 6a endpoint", async () => {
    const [source, page] = await Promise.all([
        readFile(configSourcePath, "utf8"),
        readFile(configPagePath, "utf8"),
    ]);
    assert.match(page, /data-i18n="config\.integration\.ap_polling\.title"/);
    assert.match(page, /id="apPollingEnabled"/);
    assert.match(page, /id="apPollingInterval"/);
    assert.match(page, /id="apPollingUnavailable"/);
    assert.match(page, /id="apPollingRows"/);
    assert.match(page, /id="apPollingFactorBody"/);
    assert.match(page, /id="apPollingRefresh"/);
    assert.match(source, /fetch\("\/local-api\/ap-polling"/);
    assert.match(source, /window\.config\.ap_polling = \{/);
    assert.match(source, /enabled: document\.getElementById\("apPollingEnabled"\)\.checked/);
    assert.match(source, /intervalValid\(/);
    assert.match(source, /validateFactorRows\(/);
});
