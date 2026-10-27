import assert from "node:assert/strict";
import test from "node:test";

import {
    clampSchedulerPercent,
    schedulerExpectedStepSeconds,
    schedulerProgressPercent,
    schedulerStepEndPercent,
    schedulerTimingParts,
} from "./scheduler_progress.mjs";

const T0 = 1_800_000_000; // fixed unix seconds

test("time-weighted step end boundaries match the mirrored weights", () => {
    assert.equal(schedulerStepEndPercent({step_index: 2, step_count: 5}), 72);
    assert.equal(schedulerStepEndPercent({step_index: 5, step_count: 5}), 100);
    assert.equal(schedulerStepEndPercent({step_index: 2, step_count: 4}), 75);
});

test("no creep without a timestamp: the published percent is authoritative", () => {
    const p = {active: true, phase: "running_integration", percent: 2, step_index: 2, step_count: 5};
    assert.equal(schedulerProgressPercent(p, T0 * 1000), 2);
});

test("creep is bounded: monotone, strictly below the step end, never 100", () => {
    const p = {
        active: true,
        phase: "running_integration",
        percent: 2,
        step_index: 2,
        step_count: 5,
        updated_unix: T0,
    };
    assert.equal(schedulerProgressPercent(p, T0 * 1000), 2);
    const mid = schedulerProgressPercent(p, T0 * 1000 + 7000);
    assert.ok(mid > 2 && mid < 72, `mid creep ${mid} must be inside (2,72)`);
    // A huge elapsed must NOT reach the boundary (72) or 100.
    const saturated = schedulerProgressPercent(p, T0 * 1000 + 10_000_000);
    assert.equal(saturated, 71);
    assert.ok(saturated < 100);
});

test("the terminal publish is never crossed (active:false stays exact)", () => {
    const terminal = {active: false, phase: "ready", percent: 100, step_index: 5, step_count: 5, updated_unix: T0};
    assert.equal(schedulerProgressPercent(terminal, T0 * 1000 + 999_999), 100);
});

test("an explicit-override wait phase never creeps", () => {
    const wait = {
        active: true,
        phase: "waiting_for_topology_runtime",
        percent: 80,
        step_index: 4,
        step_count: 5,
        updated_unix: T0,
    };
    assert.equal(schedulerProgressPercent(wait, T0 * 1000 + 60_000), 80);
});

test("reload_requested never creeps (the pre-step is momentary)", () => {
    const pre = {
        active: true,
        phase: "reload_requested",
        percent: 0,
        step_index: 1,
        step_count: 5,
        updated_unix: T0,
    };
    assert.equal(schedulerProgressPercent(pre, T0 * 1000 + 60_000), 0);
});

test("expected step seconds: the integration phases are the long pole", () => {
    assert.equal(schedulerExpectedStepSeconds({phase: "running_integration"}), 14);
    assert.equal(schedulerExpectedStepSeconds({phase: "partial_integration"}), 14);
    assert.equal(schedulerExpectedStepSeconds({phase: "applying_overrides"}), 1.5);
});

test("timing parts expose elapsed/expected for the #4 readout", () => {
    const parts = schedulerTimingParts({phase: "running_integration", updated_unix: T0}, T0 * 1000 + 5000);
    assert.equal(parts.elapsedMs, 5000);
    assert.equal(parts.expectedMs, 14000);
    assert.equal(schedulerTimingParts({updated_unix: 0}, T0 * 1000), null);
});

test("clamp guards the range", () => {
    assert.equal(clampSchedulerPercent(-5), 0);
    assert.equal(clampSchedulerPercent(250), 100);
    assert.equal(clampSchedulerPercent(NaN), 0);
});
