// The System Health timing readout has to interpolate the message it renders.
//
// Both catalogs name `{duration}` and `{remaining}`; the call site once passed
// `elapsed`, so the placeholder reached the screen literally in every language
// and no catalog check could see it — the mismatch was between the call and the
// message. These tests render the real catalogs, so a rename on either side
// fails here as visible text rather than as a `{duration}` left in the UI.

import assert from "node:assert/strict";
import {readFileSync} from "node:fs";
import {dirname, resolve} from "node:path";
import test from "node:test";
import {fileURLToPath} from "node:url";

import {schedulerDurationText, schedulerTimingText} from "./scheduler_timing.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CATALOG_DIR = resolve(HERE, "../../static2/i18n");
const EN = JSON.parse(readFileSync(resolve(CATALOG_DIR, "en.json"), "utf8"));
const ES = JSON.parse(readFileSync(resolve(CATALOG_DIR, "es.json"), "utf8"));

// The integration phase is the long pole (14 s); a 2 s old update therefore
// renders BOTH durations, which is the branch that carries `{duration}`.
const UPDATED_UNIX = 1_760_000_000;
const EXPECTED_SECONDS = 14;
const PHASE = {phase: "running_integration", updated_unix: UPDATED_UNIX};

function installCatalogs(served = null) {
    globalThis.window = served ? {LQOS_I18N_EN: EN, LQOS_I18N: served} : {LQOS_I18N_EN: EN};
}

test.afterEach(() => {
    delete globalThis.window;
});

test("the elapsed/remaining readout fills both placeholders in English", () => {
    installCatalogs();
    const text = schedulerTimingText(PHASE, (UPDATED_UNIX + 2) * 1000);
    assert.equal(text, "elapsed 2.0s · ~12.0s left");
    assert.doesNotMatch(text, /\{(?:duration|remaining)\}/);
});

test("the elapsed/remaining readout fills both placeholders in Spanish", () => {
    installCatalogs(ES);
    const text = schedulerTimingText(PHASE, (UPDATED_UNIX + 2) * 1000);
    assert.equal(text, "tiempo transcurrido: 2.0s · tiempo restante aproximado: 12.0s");
    assert.doesNotMatch(text, /\{(?:duration|remaining)\}/);
});

test("past the expected duration the readout drops the remaining half", () => {
    installCatalogs();
    const text = schedulerTimingText(PHASE, (UPDATED_UNIX + EXPECTED_SECONDS + 1) * 1000);
    assert.equal(text, "elapsed 15.0s");
    assert.doesNotMatch(text, /\{duration\}/);
});

test("progress without a timing anchor renders nothing", () => {
    installCatalogs();
    assert.equal(schedulerTimingText({phase: "idle"}), "");
});

test("a duration renders sub-second values in milliseconds and negatives as unknown", () => {
    assert.equal(schedulerDurationText(750), "750ms");
    assert.equal(schedulerDurationText(2000), "2.0s");
    assert.equal(schedulerDurationText(-1), "—");
    assert.equal(schedulerDurationText(undefined), "—");
});
