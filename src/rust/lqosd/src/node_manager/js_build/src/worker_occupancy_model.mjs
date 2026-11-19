// 74t171: pure render math for the per-worker occupancy view (the wire rows:
// queue_id, busy_ppm, idle_ppm, laps_per_s, busy_laps, backlog_mean,
// cycles_per_busy_lap, phase_cycles). No DOM, no fetch — unit-testable.
//
// The canonical definition (lqos-74t.159): busy_ppm = busy_nanos/interval_nanos
// x 1e6 where busy excludes empty-RX idle polling. The page's badge states that
// caveat (`cpu_workers.fleet_badge`), so this module does not repeat the wording.

import {t} from "./i18n/index.js";

// The autobalance arm: an 85% busy worker is the warning line.
export const WORKER_OCCUPANCY_WARN_PPM = 850_000;

/// The phase -> Bootstrap color class (the stacked bar's stable palette).
export const PHASE_COLORS = {
    // The dominant (enqueue) keeps the high-contrast purple.
    RX: "success",
    enqueue: "purple",
    release: "secondary",
    dequeue: "danger",
    TX: "primary",
    handoff: "info",
};

// The stable bridge phase order (stats.rs PHASE_NAMES).
export const PHASE_NAMES = [
    "rx", "dissect", "select", "charge", "leaf", "tx",
    "begin_cycle", "drop_overdue", "handoff", "empty_rx",
];

// The card's stacked groups (empty_rx is poll/idle — excluded by the label).
// 74t171 phase-label fix: the names follow the CODE semantics (the bridge's
// Phase enum), not the enum's stale names: Phase::Dissect (worker_loop
// 1807/1850/1904) wraps the leaf/CAKE/FIFO ENQUEUE + AQM; the true packet
// dissect/classify is inside Phase::Rx (worker_loop:1561). Phase::Select and
// Phase::Leaf are never recorded; they contribute 0.
export const PHASE_STACK_GROUPS = [
    {name: "RX", indices: [0]},
    {name: "enqueue", indices: [1, 4]},
    {name: "release", indices: [3]},
    {name: "dequeue", indices: [6, 7]},
    {name: "TX", indices: [5]},
    {name: "handoff", indices: [8]},
];

// The group's display label. The group `name` is a stable code the page's colour
// map keys on; only the word comes from the catalog.
function phaseLabel(name) {
    switch (name) {
    case "RX":
        return t("cpu_workers.phase.rx");
    case "enqueue":
        return t("cpu_workers.phase.enqueue");
    case "release":
        return t("cpu_workers.phase.release");
    case "dequeue":
        return t("cpu_workers.phase.dequeue");
    case "TX":
        return t("cpu_workers.phase.tx");
    default:
        return t("cpu_workers.phase.handoff");
    }
}

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

/// 74t171 follow-up: the LIVE busy_ppm for a row. `busy_ppm_recent` (the
/// ~5 s rolling window) is the live load; the lifetime cumulative
/// `busy_ppm` converges across workers under autobalance (all rows render
/// the same value) and is only the fallback for an old producer.
export function recentBusyPpm(row) {
    const recent = row?.busy_ppm_recent;
    if (recent !== null && recent !== undefined) {
        const value = Number(recent);
        if (Number.isFinite(value)) {
            return clamp(value, 0, 1_000_000);
        }
    }
    const cumulative = Number(row?.busy_ppm);
    return Number.isFinite(cumulative) ? clamp(cumulative, 0, 1_000_000) : null;
}

/// Fleet aggregates over the available worker rows: max/mean/p95 busy_ppm
/// (the RECENT value when present, the cumulative fallback otherwise).
/// p95 = nearest-rank on the sorted values (ceil(0.95*n)-1), 0 for an empty set.
export function fleetOccupancy(workers) {
    const rows = Array.isArray(workers) ? workers : [];
    const values = rows
        .map((row) => recentBusyPpm(row))
        .filter((value) => value !== null && Number.isFinite(value));
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
        label: phaseLabel(group.name),
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
    return rows.map((row) => {
        const live = recentBusyPpm(row) ?? 0;
        return {
            queueId: toNumber(row?.queue_id, null),
            // 74t171 follow-up: the displayed value is the RECENT window.
            busyPpm: live,
            busyPct: busyPercent(live),
            busyPpmCumulative: clamp(toNumber(row?.busy_ppm, 0), 0, 1_000_000),
            busyPpmRecent: row?.busy_ppm_recent === null || row?.busy_ppm_recent === undefined
                ? null
                : clamp(toNumber(row.busy_ppm_recent, 0), 0, 1_000_000),
            idlePpm: clamp(toNumber(row?.idle_ppm, 0), 0, 1_000_000),
            lapsPerS: toNumber(row?.laps_per_s, 0),
            busyLaps: toNumber(row?.busy_laps, 0),
            backlogMean: toNumber(row?.backlog_mean, 0),
            cyclesPerBusyLap: toNumber(row?.cycles_per_busy_lap, 0),
            phases: normalizePhaseStack(row?.phase_cycles),
            color: occupancyColor(live),
        };
    });
}

/// 74t171 rework: the per-core STACKED bar — the busy fraction split by the
/// phase fractions, plus the neutral idle/poll remainder (the full bar = 100%).
export function stackedOccupancySegments(busyPpm, phases) {
    const busy = clamp(toNumber(busyPpm, 0), 0, 1_000_000) / 1_000_000;
    const segments = (Array.isArray(phases) ? phases : []).map((phase) => {
        const fraction = Math.max(0, toNumber(phase?.fraction, 0)) * busy;
        return {name: phase?.name ?? "?", label: phase?.label ?? phase?.name ?? "?", fraction, pct: fraction * 100};
    });
    return {
        segments,
        busyPct: busy * 100,
        idlePct: Math.max(0, (1 - busy) * 100),
    };
}

/// 74t171 work order (1): retain the last SUCCESSFUL workers/utilization
/// sections across a transient workers_unavailable — a refresh failure must
/// not flip the Cores rows to the usage fallback or wipe the layout.
export function retainLastGoodSections(last, next) {
    if (!last) {
        return {data: next, stale: false};
    }
    const workersBad = next?.workers?.available === false;
    const utilizationBad = next?.utilization?.available === false;
    if (!workersBad && !utilizationBad) {
        return {data: next, stale: false};
    }
    return {
        data: {
            ...next,
            workers: workersBad ? last.workers : next.workers,
            utilization: utilizationBad ? last.utilization : next.utilization,
        },
        stale: true,
    };
}

/// 74t171: the Cores row kind — a DPDK worker lcore is ALWAYS a Worker row
/// (a pending utilization is a "waiting" state, never the OS/LB usage
/// fallback); only a non-worker lcore is OS/LB.
export function coreRowKind(hasWorker, hasUtilization) {
    if (hasWorker) {
        return hasUtilization ? "worker" : "worker-pending";
    }
    return "os";
}
