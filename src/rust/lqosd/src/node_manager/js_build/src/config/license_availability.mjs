// License & Services availability rendering: the mapped-circuit limit figure
// and the availability alert beneath the status cards, both driven by the
// server's `LtsCapabilitiesSummary` payload. The alert element is a parameter
// so a test can witness every branch with a plain object standing in for it.

import {t, tHtml} from "../i18n/index.js";
import {formatNumber} from "../i18n/format.js";

// `license_state` is the numeric `LtsStatus` the server sends
// (lts2_sys/shared_types.rs): Invalid (0) is the unlicensed state — the
// effective-license default and the load_config-failure fallback both land
// there. The cap-notice decision reads that payload field, not the English
// `authority_label` display text beside it.
const LICENSE_STATE_INVALID = 0;

// The unlicensed notice funnels to the licence channel's entry page.
const BUY_LICENSE_URL = "lts_trial.html";
const HIDDEN = "d-none";

/**
 * The mapped-circuit limit as the page shows it: a formatted number, or the
 * localized "unlimited" when the license sets no cap.
 * @param {number|null} limit The payload's `mapped_circuit_limit`.
 * @returns {string}
 */
export function formatMappedCircuitLimit(limit) {
    if (limit === null || limit === undefined) {
        return t("config.lts.status.unlimited");
    }
    return formatNumber(Number(limit));
}

/**
 * Renders the availability alert for one capability payload. The unlicensed
 * notice is the only branch with markup — its funnel link needs `tHtml` — so
 * it renders and returns, while the plain-sentence branches keep
 * `textContent`. The alert is hidden exactly when it carries no message.
 *
 * @param {Element} alertElement The `#licenseAvailabilityAlert` element.
 * @param {object} capabilities An `LtsCapabilitiesSummary`-shaped payload.
 * @returns {void}
 */
export function renderAvailabilityAlert(alertElement, capabilities) {
    if (
        capabilities.license_state === LICENSE_STATE_INVALID
        && capabilities.mapped_circuit_limit !== null
        && capabilities.mapped_circuit_limit !== undefined
    ) {
        alertElement.innerHTML = tHtml("config.lts.alert.unlicensed_cap", {
            limit: formatMappedCircuitLimit(capabilities.mapped_circuit_limit),
            url: BUY_LICENSE_URL,
        });
        alertElement.classList.remove(HIDDEN);
        return;
    }

    let message = "";
    if (
        !capabilities.control_service_reachable
        && (capabilities.can_use_support_tickets || capabilities.can_use_chatbot)
    ) {
        message = t("config.lts.alert.control_unavailable");
    } else if (capabilities.bootstrap_suppressed) {
        message = t("config.lts.alert.bootstrap_suppressed");
    } else if (
        capabilities.bootstrap_intent
        && !capabilities.control_service_reachable
        // translatable: false — "Bootstrap pending" is the authority token the server returns and this branch compares
        && capabilities.authority_label === "Bootstrap pending"
    ) {
        message = t("config.lts.alert.bootstrap_pending");
    }

    if (message) {
        alertElement.textContent = message;
        alertElement.classList.remove(HIDDEN);
    } else {
        alertElement.textContent = "";
        alertElement.classList.add(HIDDEN);
    }
}
