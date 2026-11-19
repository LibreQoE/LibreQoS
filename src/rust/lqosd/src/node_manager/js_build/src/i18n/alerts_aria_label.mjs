// The alerts nav item's accessible name.
//
// It is the one place the UI picks between a plural message and a plain one, so
// it lives outside `template.js` — a bundle entrypoint whose import runs the
// shell's DOM initialization — and is unit-tested directly.

import {t, tn} from "./index.js";

/**
 * Accessible name for the alerts nav item.
 * @param {number} count How many alerts are active.
 * @returns {string} The plural active count, or the none-active message at zero.
 */
export function alertsAriaLabel(count) {
    return count > 0
        ? tn("common.alerts.aria_active", count)
        : t("common.alerts.aria_none");
}
