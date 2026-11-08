import assert from "node:assert/strict";
import test from "node:test";

import {
    WORKER_OCCUPANCY_LABEL,
    WORKER_OCCUPANCY_WARN_PPM,
    busyPercent,
    busyPpmFromRollup,
    fleetOccupancy,
    normalizePhaseStack,
    normalizeWorkerRows,
    occupancyColor,
} from "./worker_occupancy_model.mjs";

// The bridge's integer tsc->nanos (1 GHz fixture: cycles == nanos).
const tscToNanos1G = (cycles) => Math.trunc((cycles * 1_000_000_000) / 1_000_000_000);

test("the label is the card's exact poll-excluded wording", () => {
    assert.equal(WORKER_OCCUPANCY_LABEL, "Worker occupancy (poll-excluded)");
    assert.equal(WORKER_OCCUPANCY_WARN_PPM, 850_000);
});

test("the displayed busy_ppm equals the rollup re-derivation within 1 ppm", () => {
    // Acceptance (card §4): displayed busy_ppm ==
    // tsc_to_nanos(busy_cycles) * 1e6 / interval_nanos.
    const fixture = [
        {busy_cycles: 850_000_000, interval_nanos: 1_000_000_000, busy_ppm: 850_000},
        {busy_cycles: 123_456_789, interval_nanos: 1_000_000_000, busy_ppm: 123_456},
        {busy_cycles: 0, interval_nanos: 1_000_000_000, busy_ppm: 0},
        // Counter reset / no interval: the wire stays 0, never fabricated.
        {busy_cycles: 999, interval_nanos: 0, busy_ppm: 0},
    ];
    for (const row of fixture) {
        const derived = busyPpmFromRollup(row.busy_cycles, row.interval_nanos, tscToNanos1G);
        assert.ok(
            Math.abs(derived - row.busy_ppm) <= 1,
            `derived ${derived} vs wire ${row.busy_ppm}`,
        );
    }
});

test("an idle worker reads ~0 and the 85% arm colors it", () => {
    assert.equal(busyPercent(0), 0);
    assert.equal(occupancyColor(0), "ok");
    assert.equal(occupancyColor(849_999), "ok");
    assert.equal(occupancyColor(850_000), "warning");
    assert.equal(busyPercent(940_000).toFixed(1), "94.0");
});

test("fleet max/mean/p95 over the available rows (nearest-rank p95)", () => {
    // 20 workers: 0..19 * 50k -> 0, 50k, ..., 950k.
    const workers = Array.from({length: 20}, (_, i) => ({queue_id: i, busy_ppm: i * 50_000}));
    const fleet = fleetOccupancy(workers);
    assert.equal(fleet.count, 20);
    assert.equal(fleet.max_ppm, 950_000);
    assert.equal(fleet.mean_ppm, 475_000);
    // nearest-rank p95 of 20 = sorted[ceil(19)-1] = sorted[18] = 900,000.
    assert.equal(fleet.p95_ppm, 900_000);
    assert.equal(fleet.p95_pct, 90);
    assert.equal(fleet.warning, true);
    // p95 is consistent with the mean/max bounds.
    assert.ok(fleet.mean_ppm <= fleet.p95_ppm && fleet.p95_ppm <= fleet.max_ppm);
    // Empty / pre-change payload: explicit zeros, no throw.
    assert.deepEqual(fleetOccupancy(undefined), {
        count: 0, max_ppm: 0, mean_ppm: 0, p95_ppm: 0,
        max_pct: 0, mean_pct: 0, p95_pct: 0, warning: false,
    });
});

test("the phase stack groups to 6 segments and normalizes over the included phases", () => {
    // rx, dissect, select, charge, leaf, tx, begin_cycle, drop_overdue, handoff, empty_rx
    const cycles = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    const stack = normalizePhaseStack(cycles);
    assert.deepEqual(stack.map((s) => s.name), ["RX", "classify", "enqueue", "dequeue", "TX", "handoff"]);
    assert.deepEqual(stack.map((s) => s.cycles), [10, 50, 90, 150, 60, 90]);
    const total = 450; // empty_rx (100) EXCLUDED — the poll-excluded label.
    assert.ok(Math.abs(stack.reduce((sum, s) => sum + s.fraction, 0) - 1) < 1e-12);
    for (const segment of stack) {
        assert.ok(Math.abs(segment.fraction - segment.cycles / total) < 1e-12);
    }
    // A missing/empty phase array is honest-empty (no NaN).
    assert.ok(normalizePhaseStack(undefined).every((s) => s.fraction === 0));
});

test("a pre-change frame decodes to empty rows, never fabricated values", () => {
    assert.deepEqual(normalizeWorkerRows(undefined), []);
    const rows = normalizeWorkerRows([{queue_id: 3}]);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].busyPpm, 0);
    assert.equal(rows[0].busyPct, 0);
    assert.equal(rows[0].phases.length, 6);
    assert.ok(rows[0].phases.every((phase) => phase.fraction === 0));
});
