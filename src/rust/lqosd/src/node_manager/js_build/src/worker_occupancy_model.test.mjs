import assert from "node:assert/strict";
import test from "node:test";

import {clearCatalogs, EN, ES, installCatalogs, installKeyEchoCatalogs} from "../i18n/test_catalogs.mjs";
import {
    WORKER_OCCUPANCY_WARN_PPM,
    busyPercent,
    busyPpmFromRollup,
    fleetOccupancy,
    normalizePhaseStack,
    normalizeWorkerRows,
    occupancyColor,
    retainLastGoodSections,
    stackedOccupancySegments,
    coreRowKind,
    PHASE_COLORS,
} from "./worker_occupancy_model.mjs";

// The bridge's integer tsc->nanos (1 GHz fixture: cycles == nanos).
const tscToNanos1G = (cycles) => Math.trunc((cycles * 1_000_000_000) / 1_000_000_000);

test.beforeEach(() => installCatalogs());
test.afterEach(clearCatalogs);

test("the warning line is the autobalance arm", () => {
    // The card's poll-excluded wording lives in the catalog
    // (`cpu_workers.fleet_badge`), rendered by the page; this model keeps only
    // the threshold.
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
    assert.deepEqual(stack.map((s) => s.name), ["RX", "enqueue", "release", "dequeue", "TX", "handoff"]);
    assert.deepEqual(stack.map((s) => s.label), [
        "RX (incl. dissect/classify)",
        "enqueue (CAKE/FIFO) + AQM",
        "release/charge",
        "dequeue",
        "TX",
        "handoff",
    ]);
    assert.deepEqual(stack.map((s) => s.cycles), [10, 70, 40, 150, 60, 90]);
    const total = 420; // empty_rx (100) EXCLUDED — the poll-excluded label.
    assert.ok(Math.abs(stack.reduce((sum, s) => sum + s.fraction, 0) - 1) < 1e-12);
    for (const segment of stack) {
        assert.ok(Math.abs(segment.fraction - segment.cycles / total) < 1e-12);
    }
    // A missing/empty phase array is honest-empty (no NaN).
    assert.ok(normalizePhaseStack(undefined).every((s) => s.fraction === 0));
});

test("the phase labels are the group codes' catalog keys, in both locales", () => {
    // An empty catalog makes every label render as the key it asked for: that is
    // the code-to-key mapping, observed through the real render path.
    installKeyEchoCatalogs();
    const keys = normalizePhaseStack([]).map((group) => group.label);
    assert.deepEqual(keys, [
        "cpu_workers.phase.rx",
        "cpu_workers.phase.enqueue",
        "cpu_workers.phase.release",
        "cpu_workers.phase.dequeue",
        "cpu_workers.phase.tx",
        "cpu_workers.phase.handoff",
    ]);
    for (const key of keys) {
        assert.ok(EN.messages[key], `en.json is missing "${key}"`);
        assert.ok(ES.messages[key], `es.json is missing "${key}"`);
    }
});

test("the phase labels follow the served locale, and the group codes do not", () => {
    installCatalogs(ES);
    const stack = normalizePhaseStack([1, 1, 0, 1, 1, 1, 1, 1, 1, 1]);
    assert.deepEqual(stack.map((group) => group.name), ["RX", "enqueue", "release", "dequeue", "TX", "handoff"]);
    assert.deepEqual(stack.map((group) => group.label), [
        ES.messages["cpu_workers.phase.rx"],
        ES.messages["cpu_workers.phase.enqueue"],
        ES.messages["cpu_workers.phase.release"],
        ES.messages["cpu_workers.phase.dequeue"],
        ES.messages["cpu_workers.phase.tx"],
        ES.messages["cpu_workers.phase.handoff"],
    ]);
    assert.equal(stack[1].label, "encolado (CAKE/FIFO) + AQM");
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

test("the stacked core bar splits the busy fraction and leaves an idle/poll remainder", () => {
    const phases = normalizePhaseStack([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]);
    const stack = stackedOccupancySegments(850_000, phases);
    assert.ok(Math.abs(stack.busyPct - 85) < 1e-9);
    assert.ok(Math.abs(stack.idlePct - 15) < 1e-9);
    const busySum = stack.segments.reduce((sum, seg) => sum + seg.pct, 0);
    assert.ok(Math.abs(busySum - 85) < 1e-9, `segments sum ${busySum} != 85`);
    assert.ok(Math.abs(busySum + stack.idlePct - 100) < 1e-9);
    // An idle worker is all remainder; no phases => zero busy segments.
    const idle = stackedOccupancySegments(0, phases);
    assert.ok(idle.segments.every((seg) => seg.pct === 0));
    assert.equal(idle.idlePct, 100);
});

test("the palette map: classify is purple (the 85% warning unchanged)", () => {
    assert.equal(PHASE_COLORS.enqueue, "purple");
    assert.equal(PHASE_COLORS.RX, "success");
    assert.equal(PHASE_COLORS.release, "secondary");
    assert.equal(PHASE_COLORS.dequeue, "danger");
    assert.equal(PHASE_COLORS.TX, "primary");
    assert.equal(PHASE_COLORS.handoff, "info");
    assert.equal(occupancyColor(850_000), "warning");
});

test("a transient workers_unavailable retains the last good sections", () => {
    const goodWorkers = {available: true, rows: [{workerId: 1}]};
    const goodUtil = {available: true, rows: [{queue_id: 1, busy_ppm: 850_000}]};
    const last = {workers: goodWorkers, utilization: goodUtil};
    // A still-good payload replaces the last good for its sections.
    const good = {workers: {available: true, rows: []}, utilization: {available: true, rows: []}};
    assert.equal(retainLastGoodSections(last, good).stale, false);
    // A transient workers_unavailable keeps the last good workers section.
    const bad = {workers: {available: false, reason: "workers_unavailable"}, utilization: goodUtil};
    const retained = retainLastGoodSections(last, bad);
    assert.equal(retained.stale, true);
    assert.deepEqual(retained.data.workers, goodWorkers);
    assert.deepEqual(retained.data.utilization, goodUtil);
    // A utilization_unavailable keeps the last good utilization.
    const badUtil = {workers: goodWorkers, utilization: {available: false, reason: "no_worker_utilization_rows"}};
    const kept = retainLastGoodSections(last, badUtil);
    assert.equal(kept.stale, true);
    assert.deepEqual(kept.data.utilization, goodUtil);
    // No prior good payload: the next passes through (the initial load).
    assert.equal(retainLastGoodSections(null, bad).stale, false);
});

test("a worker lcore is never the OS/LB usage fallback while utilization pends", () => {
    assert.equal(coreRowKind(true, true), "worker");
    assert.equal(coreRowKind(true, false), "worker-pending");
    assert.equal(coreRowKind(false, false), "os");
});

test("the LIVE row value is the recent window; cumulative is reference only", () => {
    const rows = normalizeWorkerRows([
        {queue_id: 0, busy_ppm: 492_000, busy_ppm_recent: 780_000},
        {queue_id: 1, busy_ppm: 492_000},
        {queue_id: 2, busy_ppm: 492_000, busy_ppm_recent: 0},
    ]);
    assert.equal(rows[0].busyPct, 78, "recent wins over the converged cumulative");
    assert.equal(rows[0].busyPpmCumulative, 492_000, "cumulative kept for reference");
    assert.equal(rows[0].busyPpmRecent, 780_000);
    assert.equal(rows[1].busyPct, 49.2, "an old producer falls back to cumulative");
    assert.equal(rows[1].busyPpmRecent, null);
    assert.equal(rows[2].busyPct, 0, "a genuine recent 0 is NOT the cumulative");

    // The fleet badge reads the same LIVE value (not the converged mean).
    const fleet = fleetOccupancy([
        {queue_id: 0, busy_ppm: 492_000, busy_ppm_recent: 780_000},
        {queue_id: 1, busy_ppm: 492_000, busy_ppm_recent: 520_000},
    ]);
    assert.equal(fleet.max_pct, 78);
    assert.equal(fleet.mean_pct, 65);
});
