// 74t171: pure render math for the per-worker occupancy view (the wire rows:
// queue_id, busy_ppm, idle_ppm, laps_per_s, busy_laps, backlog_mean,
// cycles_per_busy_lap, phase_cycles). No DOM, no fetch — unit-testable.
//
// The canonical definition (lqos-74t.159): busy_ppm = busy_nanos/interval_nanos
// x 1e6 where busy excludes empty-RX idle polling. The label MUST say so.

export const WORKER_OCCUPANCY_LABEL = "Worker occupancy (poll-excluded)";
// The autobalance arm: an 85% busy worker is the warning line.
export const WORKER_OCCUPANCY_WARN_PPM = 850_000;

// The stable bridge phase order (stats.rs PHASE_NAMES).
export const PHASE_NAMES = [
    "rx", "dissect", "select", "charge", "leaf", "tx",
    "begin_cycle", "drop_overdue", "handoff", "empty_rx",
];

// The card's stacked groups (empty_rx is poll/idle — excluded by the label).
export const PHASE_STACK_GROUPS = [
    {name: "RX", indices: [0]},
    {name: "classify", indices: [1, 2]},
    {name: "enqueue", indices: [3, 4]},
    {name: "dequeue", indices: [6, 7]},
    {name: "TX", indices: [5]},
    {name: "handoff", indices: [8]},
];

function toNumber(value, fallback) {
    const n = Number(value);
    return Number.isFinite(n) ? n : fallback;
}

function clamp(value, lo, hi) {
    return Math.min(hi, Math.max(lo, value));
}

/// busy_ppm -> percent (0..100), the displayed number.
export function busyPercent(busyPpm) {
    return clamp(toNumber(busyPpm, 0), 0, 1_000_000) / 10_000;
}

/// The 85% warning color class for a worker (autobalance parity).
export function occupancyColor(busyPpm) {
    return toNumber(busyPpm, 0) >= WORKER_OCCUPANCY_WARN_PPM
        ? "warning"
        : "ok";
}

/// Independent re-derivation of the wire busy_ppm from cumulative rollup
/// counters (the acceptance cross-check; tscToNanos is injected so the test
/// pins the same integer math the bridge uses).
export function busyPpmFromRollup(busyCycles, intervalNanos, tscToNanos) {
    const interval = toNumber(intervalNanos, 0);
    if (interval <= 0) {
        return 0;
    }
    const busyNanos = toNumber(tscToNanos(busyCycles), 0);
    return Math.min(1_000_000, Math.trunc((busyNanos * 1_000_000) / interval));
}

/// Fleet aggregates over the available worker rows: max/mean/p95 busy_ppm.
/// p95 = nearest-rank on the sorted values (ceil(0.95*n)-1), 0 for an empty set.
export function fleetOccupancy(workers) {
    const rows = Array.isArray(workers) ? workers : [];
    const values = rows
        .map((row) => clamp(toNumber(row?.busy_ppm, 0), 0, 1_000_000))
        .filter((value) => Number.isFinite(value));
    if (values.length === 0) {
        return {
            count: 0, max_ppm: 0, mean_ppm: 0, p95_ppm: 0,
            max_pct: 0, mean_pct: 0, p95_pct: 0, warning: false,
        };
    }
    const sorted = [...values].sort((a, b) => a - b);
    const sum = values.reduce((total, value) => total + value, 0);
    const max = sorted[sorted.length - 1];
    const mean = Math.round(sum / values.length);
    const rank = Math.max(0, Math.ceil(0.95 * sorted.length) - 1);
    const p95 = sorted[rank];
    return {
        count: values.length,
        max_ppm: max,
        mean_ppm: mean,
        p95_ppm: p95,
        max_pct: busyPercent(max),
        mean_pct: busyPercent(mean),
        p95_pct: busyPercent(p95),
        warning: max >= WORKER_OCCUPANCY_WARN_PPM,
    };
}

/// Normalizes a worker's phase_cycles into the card's 6-segment stack.
/// Fractions are over the INCLUDED phases only (empty-RX excluded); a zero
/// total yields zero fractions (honest-empty, never NaN).
export function normalizePhaseStack(phaseCycles) {
    const cycles = Array.isArray(phaseCycles) ? phaseCycles : [];
    const groups = PHASE_STACK_GROUPS.map((group) => ({
        name: group.name,
        cycles: group.indices.reduce(
            (total, index) => total + Math.max(0, toNumber(cycles[index], 0)),
            0,
        ),
    }));
    const total = groups.reduce((sum, group) => sum + group.cycles, 0);
    return groups.map((group) => ({
        ...group,
        fraction: total > 0 ? group.cycles / total : 0,
    }));
}

/// A pre-change frame (no worker section / no phase_cycles) renders as an
/// explicit unavailable state, never fabricated zeros.
export function normalizeWorkerRows(workers) {
    const rows = Array.isArray(workers) ? workers : [];
    return rows.map((row) => ({
        queueId: toNumber(row?.queue_id, null),
        busyPpm: clamp(toNumber(row?.busy_ppm, 0), 0, 1_000_000),
        busyPct: busyPercent(row?.busy_ppm),
        idlePpm: clamp(toNumber(row?.idle_ppm, 0), 0, 1_000_000),
        lapsPerS: toNumber(row?.laps_per_s, 0),
        busyLaps: toNumber(row?.busy_laps, 0),
        backlogMean: toNumber(row?.backlog_mean, 0),
        cyclesPerBusyLap: toNumber(row?.cycles_per_busy_lap, 0),
        phases: normalizePhaseStack(row?.phase_cycles),
        color: occupancyColor(row?.busy_ppm),
    }));
}
