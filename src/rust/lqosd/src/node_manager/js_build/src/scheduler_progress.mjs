// `.63.1` — the System-Health progress continuity pure logic:
//  * (1) TIME-WEIGHTED step boundaries;
//  * (#2b) a BOUNDED client-side creep between real scheduler updates;
//  * (#4) the elapsed/expected timing parts for the readout.
//
// Imported by `template.js`; unit-tested by `scheduler_progress.test.mjs`.
// `SCHEDULER_STEP_WEIGHTS` MUST stay in sync with `SCHEDULER_STEP_WEIGHTS` in
// `src/src/scheduler.py` (the server is the source of truth for the published
// percent; this mirror is only used to bound the client creep).

export const SCHEDULER_STEP_WEIGHTS = {
    5: [2, 70, 3, 15, 10],
    4: [5, 70, 15, 10],
};

// The integration subprocess is the long pole (~10-14 s); every other step is
// sub-second. Used ONLY to creep the UI between real updates.
export const SCHEDULER_INTEGRATION_PHASES = new Set([
    "running_integration",
    "partial_integration",
]);
export const SCHEDULER_INTEGRATION_EXPECTED_SECONDS = 14;
export const SCHEDULER_STEP_EXPECTED_SECONDS = 1.5;

// Phases whose published percent is an explicit override on a possibly
// indefinite wait: never creep (that would invent progress on a bounded wait).
export const SCHEDULER_NO_CREEP_PHASES = new Set([
    "waiting_for_topology_runtime",
    "reload_requested",
    "degraded",
    "validation_failed",
]);

export function clampSchedulerPercent(value) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return 0;
    return Math.max(0, Math.min(100, Math.round(parsed)));
}

export function schedulerStepWeights(stepCount) {
    return SCHEDULER_STEP_WEIGHTS[Number(stepCount)] || null;
}

// The percent the CURRENT active step would show once COMPLETED (its weighted
// end boundary) — the CAP for the bounded creep. NOT a claim it IS complete.
export function schedulerStepEndPercent(progress) {
    const stepCount = Number(progress?.step_count);
    const stepIndex = Number(progress?.step_index);
    if (!Number.isFinite(stepCount) || stepCount <= 0 || !Number.isFinite(stepIndex)) {
        return 100;
    }
    const bounded = Math.max(1, Math.min(stepCount, stepIndex));
    const weights = schedulerStepWeights(stepCount);
    if (!weights) {
        return clampSchedulerPercent((bounded / stepCount) * 100);
    }
    let sum = 0;
    for (let i = 0; i < Math.min(bounded, weights.length); i += 1) {
        sum += weights[i];
    }
    return clampSchedulerPercent(sum);
}

export function schedulerExpectedStepSeconds(progress) {
    const phase = String(progress?.phase || "");
    return SCHEDULER_INTEGRATION_PHASES.has(phase)
        ? SCHEDULER_INTEGRATION_EXPECTED_SECONDS
        : SCHEDULER_STEP_EXPECTED_SECONDS;
}

export function schedulerUseCreep(progress) {
    if (!progress?.active) return false;
    if (SCHEDULER_NO_CREEP_PHASES.has(String(progress?.phase || ""))) return false;
    const updatedUnix = Number(progress?.updated_unix);
    return Number.isFinite(updatedUnix) && updatedUnix > 0;
}

// The elapsed / expected step durations (ms) for the #4 readout, or null.
export function schedulerTimingParts(progress, nowMs = Date.now()) {
    const updatedUnix = Number(progress?.updated_unix);
    if (!Number.isFinite(updatedUnix) || updatedUnix <= 0) {
        return null;
    }
    const elapsedMs = Math.max(0, nowMs - updatedUnix * 1000);
    const expectedMs = schedulerExpectedStepSeconds(progress) * 1000;
    return { elapsedMs, expectedMs };
}

// The displayed percent: the published value (authoritative), optionally
// CREEPED toward the current step's weighted end boundary — monotone, strictly
// below that boundary, and never 100 (only the truthful terminal publish
// reaches 100).
export function schedulerProgressPercent(progress, nowMs = Date.now()) {
    if (!progress) return 0;
    let base;
    if (progress.percent !== undefined && progress.percent !== null) {
        base = clampSchedulerPercent(progress.percent);
    } else {
        const stepCount = Number(progress.step_count);
        const stepIndex = Number(progress.step_index);
        if (!Number.isFinite(stepCount) || stepCount <= 0 || !Number.isFinite(stepIndex)) {
            return 0;
        }
        const boundedStep = Math.max(1, Math.min(stepCount, stepIndex));
        const completedSteps = progress.active ? boundedStep - 1 : boundedStep;
        base = clampSchedulerPercent((completedSteps / stepCount) * 100);
    }
    if (!schedulerUseCreep(progress)) {
        return base; // terminal / idle / a wait: the published value is authoritative
    }
    const endPercent = schedulerStepEndPercent(progress);
    if (endPercent <= base) {
        return base;
    }
    const updatedUnix = Number(progress.updated_unix);
    const elapsedSeconds = Math.max(0, nowMs / 1000 - updatedUnix);
    const expected = Math.max(0.001, schedulerExpectedStepSeconds(progress));
    const fraction = Math.min(1, elapsedSeconds / expected);
    const creeped = base + (endPercent - base) * fraction;
    const cap = Math.max(base, Math.min(endPercent - 1, 99));
    return clampSchedulerPercent(Math.min(creeped, cap));
}
