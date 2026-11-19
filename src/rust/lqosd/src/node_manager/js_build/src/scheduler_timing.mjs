// The System Health elapsed/expected readout.
//
// `scheduler_progress.mjs` supplies the parts (elapsed/expected); this module
// renders them through the active catalog. It sits outside the page bundle
// because a bundle runs DOM initialization at import, so a rendered string can
// only be asserted from a module Node can import.

import {t} from "./i18n/index.js";
import {schedulerTimingParts} from "./scheduler_progress.mjs";

/**
 * A duration as the readout shows it: sub-second values in milliseconds,
 * longer ones in seconds with one decimal.
 * @param {unknown} ms Duration in milliseconds.
 * @returns {string}
 */
export function schedulerDurationText(ms) {
    const n = Number(ms);
    if (!Number.isFinite(n) || n < 0) return "—";
    if (n < 1000) return `${Math.round(n)}ms`;
    return `${(n / 1000).toFixed(1)}s`;
}

/**
 * The elapsed/expected readout ("elapsed 4.0s · ~3.0s left"), or "" when the
 * progress carries no timing.
 *
 * The parameter name is `duration`, which is the placeholder both catalogs
 * define for `scheduler.timing.elapsed_remaining`; `extract.mjs --check` fails
 * a call site whose parameter names disagree with its message.
 * @param {object|null|undefined} progress The scheduler progress payload.
 * @param {number} [nowMs] Clock, injectable for tests.
 * @returns {string}
 */
export function schedulerTimingText(progress, nowMs = Date.now()) {
    const parts = schedulerTimingParts(progress, nowMs);
    if (!parts) {
        return "";
    }
    if (parts.elapsedMs < parts.expectedMs) {
        return t("scheduler.timing.elapsed_remaining", {
            duration: schedulerDurationText(parts.elapsedMs),
            remaining: schedulerDurationText(parts.expectedMs - parts.elapsedMs),
        });
    }
    return t("scheduler.timing.elapsed", {duration: schedulerDurationText(parts.elapsedMs)});
}
