// Last-seen text for a device or circuit, in the operator's locale.
//
// The backend reports a device's last sighting as a nanosecond *age*
// (`last_seen_nanos`), so this module turns that age into an absolute time and
// lets `i18n/relative_time.mjs` choose the wording and the unit ladder. It sits
// outside `helpers/builders.js` for the same reason `scheduler_timing.mjs` sits
// outside `template.js`: that file uses extensionless imports and cannot be
// loaded by Node, so nothing in it can be unit-tested in place.

import {relativeTimeText} from "../i18n/relative_time.mjs";

/**
 * Relative text for a last-seen age, e.g. "5 sec. ago" / "hace 5 s".
 *
 * The five-minute clamp the page used to carry ("> 5 Minutes ago") is gone: it
 * was another hand-written English sentence, and the shared helper already
 * steps to minutes, hours and days.
 * @param {number} nanos Age in nanoseconds since the last sighting. Zero,
 *   absent or non-numeric renders "-", which is this column's missing marker.
 * @returns {string}
 */
export function formatLastSeen(nanos) {
    const age = Number(nanos);
    if (!Number.isFinite(age) || age <= 0) return "-";
    return relativeTimeText(Date.now() / 1000 - age / 1e9) || "-";
}
