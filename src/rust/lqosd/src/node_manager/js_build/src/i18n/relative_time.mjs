// Short relative time for a past epoch-seconds timestamp.
//
// The wording comes from `Intl.RelativeTimeFormat`, so "5 sec. ago" and
// "hace 5 s" are the browser's translation rather than another catalog string
// to maintain; this module only picks the unit. It lives outside `template.js`
// because that file is a bundle entrypoint: importing it runs the shell's DOM
// initialization, so the ladder could not be unit-tested in place.

import {relativeTime} from "./format.js";

const SHORT = {style: "short", numeric: "auto"};

/**
 * Relative text for a timestamp in the past, e.g. "5 sec. ago" / "hace 5 s".
 * Values under one minute stay in seconds, then the unit steps up at 60
 * rounds, so a 59.6-second delta reads "1 min. ago" rather than "60 sec. ago".
 * @param {number|null|undefined} unixSeconds Timestamp in epoch seconds.
 * @param {number} [nowMs] Clock override for tests; defaults to `Date.now()`.
 * @returns {string} The relative text, or "" when the timestamp is not a
 *   positive number, so each caller keeps its own missing-value wording.
 */
export function relativeTimeText(unixSeconds, nowMs = Date.now()) {
    const seconds = Number(unixSeconds);
    if (!Number.isFinite(seconds) || seconds <= 0) return "";
    const delta = Math.max(0, Math.round(nowMs / 1000 - seconds));
    if (delta < 60) return relativeTime(-delta, "second", SHORT);
    const minutes = Math.round(delta / 60);
    if (minutes < 60) return relativeTime(-minutes, "minute", SHORT);
    const hours = Math.round(minutes / 60);
    if (hours < 24) return relativeTime(-hours, "hour", SHORT);
    return relativeTime(-Math.round(hours / 24), "day", SHORT);
}
