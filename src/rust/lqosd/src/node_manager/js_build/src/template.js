import {clearDiv, safeRelativeHref} from "./helpers/builders";
import {initRedact} from "./helpers/redact";
import {initDayNightMode} from "./helpers/dark_mode";
import {initColorBlind} from "./helpers/colorblind";
import {listenOnceMatchingWithTimeout as listenWsOnceMatchingWithTimeout} from "./pubsub/listeners";
import {get_ws_client} from "./pubsub/ws";
import {clearOperationalNetworkModeStorage} from "./config/network_mode_storage.mjs";
import {schedulerDegradedReason as renderDegradedReason} from "./scheduler_summary_reason.mjs";
import {clampSchedulerPercent, schedulerProgressPercent} from "./scheduler_progress.mjs";
import {schedulerDurationText, schedulerTimingText} from "./scheduler_timing.mjs";
import {isAdminRole} from "./circuit_admin_actions.mjs";
import {applyDomTranslations, t, tn} from "./i18n/index.js";
import {initLanguageSwitcher} from "./i18n/switcher.mjs";
import {formatDateTime, formatTime} from "./i18n/format.js";
import {relativeTimeText} from "./i18n/relative_time.mjs";
import {escapeAttr, escapeHtml} from "./i18n/escape.mjs";
import {alertsAriaLabel} from "./i18n/alerts_aria_label.mjs";
import {applyCancelCapNotice} from "./lts_cancel_notice.js";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

const SCHEDULER_STATUS_POLL_MS = 2000;
const SCHEDULER_STATUS_TIMEOUT_MS = 2500;
const URGENT_ISSUES_TIMEOUT_MS = 2500;
const SCHEDULER_STATUS_STARTING_GRACE_MS = 30000;
const SCHEDULER_MODAL_IDLE_POLL_MS = 3000;
const SCHEDULER_MODAL_ACTIVE_POLL_MS = 1000;
let schedulerStatusPollTimer = null;
let schedulerStatusRequestInFlight = false;
let schedulerStatusFirstRequestedAt = null;
let cachedSchedulerStatusData = null;
let schedulerModalPollTimer = null;
let schedulerModalRequestInFlight = false;
let schedulerModalInstance = null;
let cachedSchedulerDetailsData = null;
let cachedQueueModeConfig = null;

function schedulerProgressSummary(progress) {
    if (!progress) return "";
    const label = progress.phase_label || progress.phase || t("scheduler.progress.fallback");
    const percent = schedulerProgressPercent(progress);
    const stepIndex = Number(progress.step_index);
    const stepCount = Number(progress.step_count);
    if (Number.isFinite(stepIndex) && Number.isFinite(stepCount) && stepCount > 0) {
        return t("scheduler.progress.with_step", {label, percent, step: stepIndex, count: stepCount});
    }
    return t("scheduler.progress.without_step", {label, percent});
}

function schedulerRingMarkup(progressPercent, tone, centerIcon) {
    const percent = clampSchedulerPercent(progressPercent);
    return `
        <span class="lqos-scheduler-ring" aria-hidden="true">
            <svg class="lqos-scheduler-ring-svg" viewBox="0 0 36 36" focusable="false">
                <circle class="lqos-scheduler-ring-track" cx="18" cy="18" r="15.9155"></circle>
                <circle class="lqos-scheduler-ring-value ${tone}" cx="18" cy="18" r="15.9155" pathLength="100" stroke-dasharray="${percent} 100"></circle>
            </svg>
            <span class="lqos-scheduler-ring-icon"><i class="fa ${centerIcon}"></i></span>
        </span>`;
}

function schedulerErrorText(data) {
    const error = data?.error;
    if (!error) return "";
    return String(error).trim();
}

function schedulerSnapshotUpdatedUnix(data) {
    const progressUpdatedUnix = Number(data?.progress?.updated_unix);
    if (Number.isFinite(progressUpdatedUnix) && progressUpdatedUnix > 0) {
        return progressUpdatedUnix;
    }
    const statusUpdatedUnix = Number(data?.updated_unix);
    if (Number.isFinite(statusUpdatedUnix) && statusUpdatedUnix > 0) {
        return statusUpdatedUnix;
    }
    return null;
}

function schedulerUpdatedText(data) {
    const updatedUnix = schedulerSnapshotUpdatedUnix(data);
    if (!updatedUnix) {
        return null;
    }
    // The prefix is catalog prose; the timestamp itself is Intl's, so it needs
    // no catalog string and follows the active locale.
    return t("scheduler.updated", {time: relativeTimeText(updatedUnix)});
}

function schedulerTransportWarningText(message, data) {
    const updatedText = schedulerUpdatedText(data);
    if (!updatedText) {
        return t("scheduler.transport.retrying", {message});
    }
    return t("scheduler.transport.cached", {message, updated: updatedText});
}

function schedulerStaleStatusLabel(data) {
    const updatedText = schedulerUpdatedText(data);
    if (!updatedText) {
        return t("scheduler.stale.title");
    }
    return t("scheduler.stale.updated", {updated: updatedText});
}

function schedulerSummarySeverity(data) {
    // The served worst-of severity. The frontend never classifies; it only
    // renders the served value.
    const severity = String(data?.summary_severity || "").trim().toLowerCase();
    if (severity === "healthy" || severity === "degraded" || severity === "down"
        || severity === "running" || severity === "idle") {
        return severity;
    }
    return data?.available ? "healthy" : "idle";
}

function schedulerWaitsForTopologyRuntime(data) {
    const progress = data?.progress || null;
    if (!progress) {
        return false;
    }
    return String(progress.phase || "").trim() === "waiting_for_topology_runtime";
}

function schedulerIntegrationLabel(data) {
    const name = String(data?.integration_name || "").trim();
    return name || t("scheduler.integration.fallback");
}

function schedulerIntegrationSubtitle(data) {
    const name = schedulerIntegrationLabel(data);
    const failures = Number(data?.consecutive_failures) || 0;
    const detail = String(data?.integration_error_detail || "").trim();
    const cls = String(data?.integration_error_class || "").trim();
    const parts = [];
    if (failures > 0) parts.push(tn("scheduler.integration.consecutive_failures", failures));
    if (cls && cls !== "none") parts.push(cls.replace(/_/g, " "));
    if (detail) parts.push(detail);
    return parts.join(" · ") || t("scheduler.integration.import_failing", {name});
}

// Render the SERVED `summary_reason` (backend-produced, paired with the
// severity) into the degraded title/subtitle. The frontend never re-derives
// the axis — attributing a boot-world degrade to the import is the defect.
function schedulerDegraded(data) {
    return renderDegradedReason(
        data?.summary_reason,
        schedulerIntegrationLabel(data),
        schedulerIntegrationSubtitle(data),
    );
}

function schedulerShapingSubtitle(data) {
    const detail = String(data?.error || "").trim();
    return detail || t("scheduler.down.subtitle");
}

function schedulerHealthySubtitle(data) {
    if (data?.last_import_ok === true) {
        return t("scheduler.healthy.subtitle", {name: schedulerIntegrationLabel(data)});
    }
    return t("scheduler.healthy.title");
}

function schedulerStateDescriptor(data) {
    const progress = data?.progress || null;
    const setupRequired = !!data?.setup_required;
    const severity = schedulerSummarySeverity(data);
    const topologyRuntimeWait = schedulerWaitsForTopologyRuntime(data);

    if (setupRequired || severity === "idle") {
        return {
            tone: "secondary",
            badgeClass: "text-bg-secondary",
            label: t("scheduler.badge.idle"),
            title: setupRequired
                ? (data?.setup_message || t("scheduler.idle.setup_title"))
                : t("scheduler.idle.title"),
            subtitle: setupRequired
                ? (data?.setup_message || t("scheduler.idle.setup_subtitle"))
                : t("scheduler.idle.title"),
            ringTone: "tone-loading",
            icon: "fa-pause",
        };
    }
    if (data?.stale) {
        return {
            tone: "warning",
            badgeClass: "text-bg-warning",
            label: t("scheduler.badge.stale"),
            title: t("scheduler.stale.title"),
            subtitle: t("scheduler.stale.subtitle"),
            ringTone: "tone-warning",
            icon: "fa-clock",
        };
    }
    if (severity === "down") {
        return {
            tone: "danger",
            badgeClass: "text-bg-danger",
            label: t("scheduler.badge.down"),
            title: progress?.phase_label || progress?.phase || t("scheduler.down.title"),
            subtitle: schedulerShapingSubtitle(data),
            ringTone: "tone-danger",
            icon: "fa-triangle-exclamation",
        };
    }
    if (topologyRuntimeWait) {
        return {
            tone: "warning",
            badgeClass: "text-bg-warning",
            label: t("scheduler.badge.deferred"),
            title: progress?.phase_label || t("scheduler.deferred.title"),
            subtitle: t("scheduler.deferred.subtitle"),
            ringTone: "tone-warning",
            icon: "fa-hourglass-half",
        };
    }
    if (severity === "running") {
        return {
            tone: "info",
            badgeClass: "text-bg-info",
            label: t("scheduler.badge.running"),
            title: progress?.phase_label || progress?.phase || t("scheduler.running.title"),
            subtitle: t("scheduler.running.subtitle"),
            ringTone: "tone-progress",
            icon: "fa-arrows-rotate",
        };
    }
    if (severity === "degraded") {
        const degraded = schedulerDegraded(data);
        return {
            tone: "warning",
            badgeClass: "text-bg-warning",
            label: t("scheduler.badge.degraded"),
            title: degraded.title,
            subtitle: degraded.subtitle,
            ringTone: "tone-warning",
            icon: "fa-triangle-exclamation",
        };
    }
    if (severity === "healthy") {
        return {
            tone: "success",
            badgeClass: "text-bg-success",
            label: t("scheduler.badge.healthy"),
            title: t("scheduler.healthy.title"),
            subtitle: schedulerHealthySubtitle(data),
            ringTone: "tone-success",
            icon: "fa-check",
        };
    }
    return {
        tone: "warning",
        badgeClass: "text-bg-warning",
        label: t("scheduler.badge.starting"),
        title: t("scheduler.starting.title"),
        subtitle: progress?.phase_label || progress?.phase || t("scheduler.starting.subtitle"),
        ringTone: "tone-warning",
        icon: "fa-clock",
    };
}

function schedulerProgressBarClass(descriptor) {
    if (descriptor.tone === "danger") return "bg-danger";
    if (descriptor.tone === "warning") return "bg-warning";
    if (descriptor.tone === "success") return "bg-success";
    return "bg-info";
}

function summarizeSchedulerOutput(output, error) {
    if (error && String(error).trim().length > 0) {
        return String(error).trim();
    }
    const lines = String(output || "")
        .split(/\r?\n/)
        .map(line => line.trim())
        .filter(Boolean);
    if (!lines.length) {
        return t("scheduler.summary.no_output");
    }
    return lines[lines.length - 1];
}

function schedulerSetupMessage(data) {
    if (data?.setup_required && data?.setup_message) {
        return String(data.setup_message).trim();
    }
    return "";
}

function schedulerActivityItems(output, error, data) {
    const setupMessage = schedulerSetupMessage(data);
    if (setupMessage.length > 0) {
        return [
            setupMessage,
            t("scheduler.setup.open_hint"),
        ];
    }
    if (error && String(error).trim().length > 0) {
        return [String(error).trim()];
    }
    return String(output || "")
        .split(/\r?\n/)
        .map(line => line.trim())
        .filter(Boolean)
        .slice(-4)
        .reverse();
}

function schedulerAgeText(unixTs) {
    // "time unknown" stays the caller's wording; the age itself is Intl's.
    return relativeTimeText(unixTs) || t("scheduler.time_unknown");
}

function schedulerSummaryLine(data, descriptor) {
    const severity = schedulerSummarySeverity(data);
    if (severity === "degraded") {
        return schedulerDegraded(data).subtitle;
    }
    if (severity === "down") {
        return schedulerShapingSubtitle(data);
    }
    if (severity === "running") {
        return t("scheduler.running.title");
    }
    if (severity === "idle") {
        return data?.setup_required
            ? (data?.setup_message || t("scheduler.setup.title"))
            : t("scheduler.idle.title");
    }
    return schedulerHealthySubtitle(data);
}

function schedulerPanelRow(label, value) {
    return `<div class="lqos-scheduler-panel-row d-flex justify-content-between py-1">
        <span class="text-secondary">${escapeHtml(label)}</span>
        <span class="text-end">${escapeHtml(value)}</span>
    </div>`;
}

function schedulerShapingStateText(data) {
    const severity = schedulerSummarySeverity(data);
    if (severity === "down") return t("scheduler.shaping.stopped");
    if (severity === "running") return t("scheduler.shaping.applying");
    if (data?.topology_source === "last_known_good") return t("scheduler.shaping.active_lkg");
    if (data?.topology_source === "fresh") return t("scheduler.shaping.active_fresh");
    return t("scheduler.shaping.active");
}

// AVAIL (CARD v3_availability_hardening_card §7 b4): the SEPARATE boot-world
// tier the bridge reports (the world it is actually shaping). Independent of
// `topology_source` (the import axis); "unknown" when no Supervisor report.
function schedulerBootWorldTierText(data) {
    switch (data?.boot_world_tier) {
        case "fresh": return t("scheduler.boot_world.fresh");
        case "last_known_good": return t("scheduler.boot_world.last_known_good");
        case "bootstrap_trunk": return t("scheduler.boot_world.bootstrap_trunk");
        case "pass_through": return t("scheduler.boot_world.pass_through");
        default: return t("scheduler.boot_world.unknown");
    }
}

// The §7 b7(ii) latch names: they live on the bridge health report
// (`degraded_subsystems`), surfaced via `WsRequest::BridgeHealthStatus` — this
// is that JOIN, cached so the System Health modal can name the latch.
function schedulerBootWorldLatchText() {
    const degraded = lastBridgeHealthData?.report?.degraded_subsystems;
    if (Array.isArray(degraded) && degraded.length) {
        return degraded.join("/");
    }
    return t("scheduler.latched.none");
}

function schedulerShapingPanelMarkup(data) {
    const generation = String(data?.dataplane_generation || "").trim();
    const genText = generation ? generation.slice(0, 12) : t("scheduler.value.unknown");
    const leaves = Number(data?.leaf_count);
    const leavesText = Number.isFinite(leaves) ? String(leaves) : "—";
    const applied = data?.applied_at ? schedulerAgeText(data.applied_at) : t("scheduler.value.unknown");
    return `
        <div class="lqos-scheduler-panel border rounded p-3 h-100">
            <h3 class="lqos-scheduler-section-title">${escapeHtml(t("scheduler.panel.shaping"))}</h3>
            ${schedulerPanelRow(t("scheduler.panel.state"), schedulerShapingStateText(data))}
            ${schedulerPanelRow(t("scheduler.panel.boot_world"), schedulerBootWorldTierText(data))}
            ${schedulerPanelRow(t("scheduler.panel.latched"), schedulerBootWorldLatchText())}
            ${schedulerPanelRow(t("scheduler.panel.generation"), genText)}
            ${schedulerPanelRow(t("scheduler.panel.leaves"), leavesText)}
            ${schedulerPanelRow(t("scheduler.panel.applied"), applied)}
        </div>`;
}

function schedulerIntegrationPanelMarkup(data) {
    const name = schedulerIntegrationLabel(data);
    const hasIntegration = !!data?.integration_name || data?.last_import_ok === true || data?.last_import_ok === false;
    if (!hasIntegration) {
        return `
        <div class="lqos-scheduler-panel border rounded p-3 h-100">
            <h3 class="lqos-scheduler-section-title">${escapeHtml(t("scheduler.panel.integration"))}</h3>
            <p class="text-secondary mb-0">${escapeHtml(t("scheduler.idle.title"))}</p>
        </div>`;
    }
    const ok = data?.last_import_ok;
    const stateText = ok === true
        ? t("scheduler.integration.ok")
        : ok === false ? t("scheduler.integration.failing") : t("scheduler.integration.no_import");
    const ageText = data?.last_import_at ? schedulerAgeText(data.last_import_at) : "—";
    const durationText = schedulerDurationText(data?.last_import_ms);
    const cls = String(data?.integration_error_class || "").trim();
    const classText = cls && cls !== "none" ? cls.replace(/_/g, " ") : t("scheduler.integration.none");
    const failures = Number(data?.consecutive_failures) || 0;
    return `
        <div class="lqos-scheduler-panel border rounded p-3 h-100">
            <h3 class="lqos-scheduler-section-title">${escapeHtml(t("scheduler.panel.integration_named", {name}))}</h3>
            ${schedulerPanelRow(t("scheduler.panel.state"), stateText)}
            ${schedulerPanelRow(t("scheduler.panel.last_import"), ageText)}
            ${schedulerPanelRow(t("scheduler.panel.duration"), durationText)}
            ${schedulerPanelRow(t("scheduler.panel.consecutive_failures"), String(failures))}
            ${schedulerPanelRow(t("scheduler.panel.cause"), classText)}
        </div>`;
}

function schedulerErrorHint(cls, runtimeError) {
    switch (cls) {
        case "redirect_loop": return t("scheduler.error_hint.redirect_loop");
        case "auth": return t("scheduler.error_hint.auth");
        case "unreachable": return t("scheduler.error_hint.unreachable");
        case "exited_unknown": return t("scheduler.error_hint.exited_unknown");
        case "none": return t("scheduler.error_hint.none");
        default: return runtimeError ? t("scheduler.error_hint.runtime") : "";
    }
}

function schedulerWhatToCheckMarkup(data) {
    const detail = String(data?.integration_error_detail || "").trim();
    const cls = String(data?.integration_error_class || "").trim();
    const runtimeError = String(data?.error || "").trim();
    const failing = data?.last_import_ok === false;
    if (!failing && !runtimeError) return "";
    const hint = schedulerErrorHint(cls, runtimeError);
    const detailMarkup = detail
        ? `<pre class="lqos-scheduler-what-detail small mb-2">${escapeHtml(detail)}</pre>`
        : "";
    return `
        <div class="lqos-scheduler-whatcard border rounded p-3 mt-3">
            <h3 class="lqos-scheduler-section-title">${escapeHtml(t("scheduler.what_to_check.title"))}</h3>
            ${detailMarkup}
            ${hint ? `<p class="mb-2">${escapeHtml(hint)}</p>` : ""}
            <div class="d-flex gap-2">
                <button type="button" class="btn btn-sm btn-outline-primary" disabled title="${escapeAttr(t("scheduler.what_to_check.retry_unavailable"))}">${escapeHtml(t("scheduler.what_to_check.retry"))}</button>
                <button type="button" class="btn btn-sm btn-outline-secondary" id="schedulerOpenLog">${escapeHtml(t("scheduler.what_to_check.open_log"))}</button>
            </div>
        </div>`;
}

function schedulerRecentRunsMarkup(data) {
    const runs = Array.isArray(data?.recent_runs) ? data.recent_runs.slice().reverse() : [];
    const body = runs.length
        ? runs.map(run => {
            const when = run?.at
                ? formatTime(Number(run.at) * 1000)
                : "—";
            const result = run?.ok ? t("scheduler.runs.ok") : t("scheduler.runs.failed");
            const tone = run?.ok ? "text-success" : "text-danger";
            return `<div class="lqos-scheduler-run-row d-flex justify-content-between py-1">
                <span>${escapeHtml(when)}</span>
                <span class="${tone}">${escapeHtml(result)}</span>
                <span class="text-secondary">${escapeHtml(schedulerDurationText(run?.ms))}</span>
            </div>`;
        }).join("")
        : `<p class="text-secondary mb-0">${escapeHtml(t("scheduler.runs.empty"))}</p>`;
    return `
        <div class="lqos-scheduler-runs-card border rounded p-3 mt-3">
            <h3 class="lqos-scheduler-section-title">${escapeHtml(t("scheduler.runs.title"))}</h3>
            ${body}
        </div>`;
}

function schedulerIdleLine(data) {
    if (data?.last_import_ms !== null && data?.last_import_ms !== undefined && Number.isFinite(Number(data.last_import_ms))) {
        return t("scheduler.runs.last_completed", {duration: schedulerDurationText(data.last_import_ms)});
    }
    return t("scheduler.runs.idle");
}

function renderSchedulerDetails(data, options = {}) {
    const progress = data?.progress || null;
    const descriptor = schedulerStateDescriptor(data);
    const liveError = schedulerErrorText(data);
    const nowMs = Date.now();
    const percent = progress ? schedulerProgressPercent(progress, nowMs) : (data?.available ? 100 : 0);
    const stepCount = Number(progress?.step_count);
    const stepIndex = Number(progress?.step_index);
    const stepText = Number.isFinite(stepIndex) && Number.isFinite(stepCount) && stepCount > 0
        ? t("scheduler.progress.step", {step: stepIndex, count: stepCount})
        : t("scheduler.progress.no_step");
    const updatedText = schedulerUpdatedText(data) || t("scheduler.updated.unavailable");
    let progressMeta = t("scheduler.progress.waiting");
    if (options.transportWarning) {
        progressMeta = t("scheduler.progress.cached");
    } else if (data?.setup_required) {
        progressMeta = t("scheduler.progress.setup");
    } else if (schedulerWaitsForTopologyRuntime(data)) {
        progressMeta = t("scheduler.progress.topology_wait");
    } else if (progress?.active) {
        const timing = schedulerTimingText(progress, nowMs);
        progressMeta = timing
            ? t("scheduler.progress.active", {percent, timing})
            : t("scheduler.progress.percent_complete", {percent});
    }
    const setupAlert = data?.setup_required
        ? `<div class="alert alert-warning mt-3 mb-0" role="alert"><i class="fa fa-list-check me-2"></i>${escapeHtml(schedulerSetupMessage(data) || t("scheduler.setup.choose_before"))}</div>`
        : "";
    const transportWarningMarkup = options.transportWarning
        ? `<div class="alert alert-warning mt-3 mb-0" role="alert"><i class="fa fa-wifi me-2"></i>${escapeHtml(options.transportWarning)}</div>`
        : "";
    const alertMarkup = liveError
        ? `<div class="alert alert-danger mt-3 mb-0" role="alert"><i class="fa fa-triangle-exclamation me-2"></i>${escapeHtml(liveError)}</div>`
        : "";
    const summaryLine = schedulerSummaryLine(data, descriptor);
    const shapingPanel = schedulerShapingPanelMarkup(data);
    const integrationPanel = schedulerIntegrationPanelMarkup(data);
    const whatToCheck = schedulerWhatToCheckMarkup(data);
    const runsMarkup = schedulerRecentRunsMarkup(data);
    const progressMarkup = progress?.active
        ? `<div class="lqos-scheduler-progress-card">
            <div class="lqos-scheduler-progress-topline">
                <div>
                    <h3 class="lqos-scheduler-progress-title">${escapeHtml(stepText)}</h3>
                    <p class="lqos-scheduler-progress-meta mb-0">${escapeHtml(progressMeta)}</p>
                </div>
                <div class="fw-semibold">${escapeHtml(`${percent}%`)}</div>
            </div>
            <div class="progress lqos-scheduler-progress-bar" role="progressbar" aria-label="${escapeAttr(t("scheduler.progress.aria"))}" aria-valuenow="${percent}" aria-valuemin="0" aria-valuemax="100">
                <div class="progress-bar ${schedulerProgressBarClass(descriptor)}" style="width: ${percent}%"></div>
            </div>
        </div>`
        : `<div class="lqos-scheduler-idle-line text-secondary small mt-3">${escapeHtml(schedulerIdleLine(data))}</div>`;

    return `
        <div class="lqos-scheduler-modal-summary">
            <div class="lqos-scheduler-modal-indicator">
                ${schedulerRingMarkup(percent, descriptor.ringTone, descriptor.icon)}
            </div>
            <div class="lqos-scheduler-modal-copy">
                <div class="lqos-scheduler-modal-statusline">
                    <span class="badge ${descriptor.badgeClass}">${escapeHtml(descriptor.label)}</span>
                    <h2 class="lqos-scheduler-modal-title">${escapeHtml(descriptor.title)}</h2>
                </div>
                <p class="lqos-scheduler-modal-subtitle">${escapeHtml(summaryLine)}</p>
                <div class="lqos-scheduler-modal-updated">${escapeHtml(updatedText)}</div>
            </div>
        </div>
        ${setupAlert}
        ${transportWarningMarkup}
        ${alertMarkup}
        <div class="row g-3 mt-1">
            <div class="col-12 col-lg-6">${shapingPanel}</div>
            <div class="col-12 col-lg-6">${integrationPanel}</div>
        </div>
        ${whatToCheck}
        ${runsMarkup}
        ${progressMarkup}
        <details class="lqos-scheduler-debug-card">
            <summary class="lqos-scheduler-debug-toggle">${escapeHtml(t("scheduler.modal.show_raw"))}</summary>
            <pre class="lqos-scheduler-debug-pre">${escapeHtml(data?.details || t("scheduler.modal.no_raw"))}</pre>
        </details>`;
}

function updateSchedulerModalBody(contentHtml) {
    const body = document.getElementById('schedulerDetailsBody');
    if (!body) return;
    const debugOpen = !!body.querySelector('.lqos-scheduler-debug-card[open]');
    body.innerHTML = contentHtml;
    if (debugOpen) {
        const details = body.querySelector('.lqos-scheduler-debug-card');
        if (details) {
            details.open = true;
        }
    }
    const openLog = body.querySelector('#schedulerOpenLog');
    if (openLog) {
        openLog.addEventListener('click', () => {
            const details = body.querySelector('.lqos-scheduler-debug-card');
            if (details) {
                details.open = true;
                details.scrollIntoView({ behavior: "smooth", block: "nearest" });
            }
        });
    }
}

function renderSchedulerStatus(container, state, progress, labelOverride = null) {
    if (!container) return;

    let color = "text-secondary";
    let label = t("scheduler.nav.loading");
    const buttonText = "LibreQoS";
    let indicator = schedulerRingMarkup(12, "tone-loading", "fa-circle-notch fa-spin");

    if (state === "healthy") {
        color = "text-success";
        label = t("scheduler.nav.healthy");
        indicator = schedulerRingMarkup(100, "tone-success", "fa-check");
    } else if (state === "degraded") {
        color = "text-warning";
        label = t("scheduler.nav.degraded");
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-triangle-exclamation");
    } else if (state === "idle") {
        color = "text-secondary";
        label = t("scheduler.idle.title");
        indicator = schedulerRingMarkup(100, "tone-loading", "fa-pause");
    } else if (state === "progress") {
        color = "text-info";
        label = schedulerProgressSummary(progress);
        indicator = schedulerRingMarkup(
            schedulerProgressPercent(progress),
            "tone-progress",
            "fa-arrows-rotate"
        );
    } else if (state === "unavailable") {
        color = "text-warning";
        label = t("scheduler.nav.unavailable");
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-clock");
    } else if (state === "error") {
        color = "text-danger";
        label = t("scheduler.nav.error");
        indicator = schedulerRingMarkup(100, "tone-danger", "fa-triangle-exclamation");
    } else if (state === "stale") {
        color = "text-warning";
        label = t("scheduler.stale.title");
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-clock");
    } else if (state === "deferred") {
        color = "text-warning";
        label = labelOverride || t("scheduler.deferred.label");
        indicator = schedulerRingMarkup(
            schedulerProgressPercent(progress),
            "tone-warning",
            "fa-hourglass-half"
        );
    } else if (state === "setup") {
        color = "text-warning";
        label = t("scheduler.nav.setup");
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-list-check");
    }
    if (labelOverride) {
        label = labelOverride;
    }

    container.innerHTML = `
        <button class="nav-link btn btn-link text-start w-100 border-0 ${color} lqos-scheduler-link" type="button" id="schedulerStatusLink" aria-label="${escapeAttr(label)}" title="${escapeAttr(label)}">
            ${indicator} <span>${escapeAttr(buttonText)}</span>
        </button>`;

    $("#schedulerStatus").off("click").on("click", "#schedulerStatusLink", (e) => {
        e.preventDefault();
        openSchedulerModal();
    });
}

function scheduleNextSchedulerStatusPoll(delayMs = SCHEDULER_STATUS_POLL_MS) {
    if (schedulerStatusPollTimer) {
        clearTimeout(schedulerStatusPollTimer);
    }
    schedulerStatusPollTimer = setTimeout(() => {
        schedulerStatusPollTimer = null;
        loadSchedulerStatus();
    }, delayMs);
}

function loadSchedulerStatus(force = false) {
    const container = document.getElementById('schedulerStatus');
    if (!container) return;

    if (schedulerStatusRequestInFlight) {
        return;
    }

    if (schedulerStatusFirstRequestedAt === null || force) {
        schedulerStatusFirstRequestedAt = Date.now();
        renderSchedulerStatus(container, "loading", null);
    }

    schedulerStatusRequestInFlight = true;
    listenWsOnceMatchingWithTimeout(wsClient, "SchedulerStatus", SCHEDULER_STATUS_TIMEOUT_MS, () => true, (msg) => {
        schedulerStatusRequestInFlight = false;
        if (!msg || !msg.data) {
            if (cachedSchedulerStatusData) {
                renderSchedulerStatus(
                    container,
                    "stale",
                    cachedSchedulerStatusData.progress || null,
                    schedulerTransportWarningText(t("scheduler.transport.status_empty"), cachedSchedulerStatusData)
                );
            } else {
                renderSchedulerStatus(container, "unavailable", null);
            }
            scheduleNextSchedulerStatusPoll();
            return;
        }
        const data = msg.data;
        cachedSchedulerStatusData = data;
        const severity = schedulerSummarySeverity(data);
        const progress = data.progress || null;
        const progressActive = !!(progress && progress.active);
        const setupRequired = !!data.setup_required;
        const stale = !!data.stale;
        const elapsed = schedulerStatusFirstRequestedAt === null ? 0 : (Date.now() - schedulerStatusFirstRequestedAt);

        if (setupRequired) {
            renderSchedulerStatus(container, "setup", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (severity === "down") {
            renderSchedulerStatus(container, "error", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (stale) {
            renderSchedulerStatus(container, "stale", progress, schedulerStaleStatusLabel(data));
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (severity === "degraded") {
            renderSchedulerStatus(container, "degraded", progress, schedulerDegraded(data).title);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (schedulerWaitsForTopologyRuntime(data)) {
            renderSchedulerStatus(
                container,
                "deferred",
                progress,
                progress?.phase_label || t("scheduler.deferred.label")
            );
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (severity === "running" || progressActive) {
            renderSchedulerStatus(container, "progress", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (severity === "idle") {
            renderSchedulerStatus(container, "idle", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (severity === "healthy" && data.available) {
            renderSchedulerStatus(container, "healthy", progress);
        } else {
            if (elapsed < SCHEDULER_STATUS_STARTING_GRACE_MS) {
                renderSchedulerStatus(container, "loading", progress);
            } else {
                renderSchedulerStatus(container, "unavailable", progress);
            }
        }
        scheduleNextSchedulerStatusPoll();
    }, () => {
        schedulerStatusRequestInFlight = false;
        const elapsed = schedulerStatusFirstRequestedAt === null ? 0 : (Date.now() - schedulerStatusFirstRequestedAt);
        if (cachedSchedulerStatusData) {
            renderSchedulerStatus(
                container,
                "stale",
                cachedSchedulerStatusData.progress || null,
                schedulerTransportWarningText(t("scheduler.transport.status_timeout"), cachedSchedulerStatusData)
            );
        } else if (elapsed < SCHEDULER_STATUS_STARTING_GRACE_MS) {
            renderSchedulerStatus(container, "loading", null);
        } else {
            renderSchedulerStatus(container, "unavailable", null);
        }
        scheduleNextSchedulerStatusPoll();
    });
    wsClient.send({ SchedulerStatus: {} });
}

function restartSchedulerStatusPolling() {
    schedulerStatusRequestInFlight = false;
    if (schedulerStatusPollTimer) {
        clearTimeout(schedulerStatusPollTimer);
        schedulerStatusPollTimer = null;
    }
    loadSchedulerStatus(true);
}

// ---------------------------------------------------------------------------
// E2: bridge Supervisor health widget (verdict over the raw report).
// Hidden entirely when the DPDK bridge is not configured (data null).
// ---------------------------------------------------------------------------

const BRIDGE_HEALTH_POLL_MS = 5000;
const BRIDGE_HEALTH_TIMEOUT_MS = 2500;
let bridgeHealthPollTimer = null;
let bridgeHealthRequestInFlight = false;
// Last bridge health payload (the boot-world tier + the §7 b7(ii) latched
// subsystem names), so the System Health modal can name the latch without a
// second fetch. Populated by `renderBridgeHealthStatus`.
let lastBridgeHealthData = null;

function bridgeHealthDetailText(data) {
    const parts = [];
    if (data.report) {
        parts.push(t("bridge.health.status", {status: data.report.status || "?"}));
        if (data.report.version) parts.push("v" + data.report.version);
        parts.push(t("bridge.health.uptime", {seconds: Math.round((data.report.uptime_ms || 0) / 1000)}));
        const workers = Array.isArray(data.report.workers) ? data.report.workers.length : 0;
        parts.push(tn("bridge.health.workers", workers));
        const ackDrops = data.report.channel ? (data.report.channel.ack_drops || 0) : 0;
        parts.push(t("bridge.health.ack_drops", {count: ackDrops}));
        // AVAIL (CARD v3_availability_hardening_card §7 R1/b4): NAME the
        // latched degraded subsystems and the live boot world, so a degraded
        // stats-collector/publish-driver is distinguishable by name.
        const degraded = Array.isArray(data.report.degraded_subsystems)
            ? data.report.degraded_subsystems : [];
        if (degraded.length) parts.push(t("bridge.health.degraded_subsystems", {names: degraded.join("/")}));
        if (data.report.topology_tier) parts.push(t("bridge.health.boot_world", {tier: data.report.topology_tier}));
    }
    parts.push(t("bridge.health.fail_streak", {count: data.consecutive_failures || 0, threshold: data.fails_to_down || 3}));
    if (data.last_success_unix) parts.push(t("bridge.health.last_ok", {time: formatTime(data.last_success_unix * 1000)}));
    return parts.join(", ");
}

function renderBridgeHealthStatus(container, data) {
    lastBridgeHealthData = data || null;
    if (!container) return;
    if (!data) {
        container.classList.add("d-none");
        return;
    }
    container.classList.remove("d-none");
    const verdict = String(data.verdict || "pending");
    let color = "text-secondary";
    let icon = "fa-circle-notch fa-spin";
    let label = t("bridge.health.pending");
    if (verdict === "healthy") {
        color = "text-success";
        icon = "fa-check";
        label = t("bridge.health.healthy");
    } else if (verdict === "starting") {
        color = "text-info";
        icon = "fa-hourglass-half";
        label = t("bridge.health.starting");
    } else if (verdict === "degraded") {
        color = "text-warning";
        icon = "fa-triangle-exclamation";
        label = t("bridge.health.degraded");
    } else if (verdict === "down") {
        color = "text-danger";
        icon = "fa-xmark";
        label = t("bridge.health.down");
    }
    const detail = bridgeHealthDetailText(data);
    // L4 (§5b): the sidebar Bridge item LINKS to the Bridge page. The server
    // template emits href="bridge.html"; this client re-render (every health
    // poll) MUST match, or it overrides the link with "#" and the button stops
    // navigating.
    container.innerHTML = `
        <a class="nav-link" href="bridge.html" id="bridgeStatusLink" aria-label="${escapeAttr(label)}" title="${escapeAttr(detail)}">
            <i class="fa fa-fw fa-centerline ${icon} ${color}"></i> <span class="${color}">${escapeHtml(t("common.nav.bridge"))}</span>
        </a>`;
}

function scheduleNextBridgeHealthPoll(delayMs = BRIDGE_HEALTH_POLL_MS) {
    if (bridgeHealthPollTimer) {
        clearTimeout(bridgeHealthPollTimer);
    }
    bridgeHealthPollTimer = setTimeout(() => {
        bridgeHealthPollTimer = null;
        loadBridgeHealthStatus();
    }, delayMs);
}

function loadBridgeHealthStatus() {
    const container = document.getElementById('bridgeStatus');
    if (!container) return;
    if (bridgeHealthRequestInFlight) {
        return;
    }
    bridgeHealthRequestInFlight = true;
    listenWsOnceMatchingWithTimeout(wsClient, "BridgeHealthStatus", BRIDGE_HEALTH_TIMEOUT_MS, () => true, (msg) => {
        bridgeHealthRequestInFlight = false;
        renderBridgeHealthStatus(container, msg && msg.data ? msg.data : null);
        scheduleNextBridgeHealthPoll();
    }, () => {
        bridgeHealthRequestInFlight = false;
        // On transport timeout keep the last render; retry.
        scheduleNextBridgeHealthPoll();
    });
    wsClient.send({ BridgeHealthStatus: {} });
}

function isSchedulerModalOpen() {
    const modalEl = document.getElementById('schedulerModal');
    return !!(modalEl && modalEl.classList.contains('show'));
}

function schedulerModalPollDelay(data) {
    const progressActive = !!(data?.progress && data.progress.active);
    return progressActive ? SCHEDULER_MODAL_ACTIVE_POLL_MS : SCHEDULER_MODAL_IDLE_POLL_MS;
}

function stopSchedulerModalPolling() {
    schedulerModalRequestInFlight = false;
    if (schedulerModalPollTimer) {
        clearTimeout(schedulerModalPollTimer);
        schedulerModalPollTimer = null;
    }
}

function scheduleNextSchedulerModalPoll(delayMs) {
    if (!isSchedulerModalOpen()) {
        stopSchedulerModalPolling();
        return;
    }
    if (schedulerModalPollTimer) {
        clearTimeout(schedulerModalPollTimer);
    }
    schedulerModalPollTimer = setTimeout(() => {
        schedulerModalPollTimer = null;
        refreshSchedulerModalDetails();
    }, delayMs);
}

function renderSchedulerModalLoading() {
    updateSchedulerModalBody(`
        <div class="d-flex align-items-center gap-2 text-secondary">
            <i class='fa fa-spinner fa-spin'></i>
            <span>${escapeHtml(t("scheduler.modal.loading"))}</span>
        </div>`);
}

function ensureSchedulerModalLifecycle() {
    const modalEl = document.getElementById('schedulerModal');
    if (!modalEl || modalEl.dataset.schedulerLiveBound === "1") {
        return;
    }
    modalEl.dataset.schedulerLiveBound = "1";
    modalEl.addEventListener('hidden.bs.modal', () => {
        stopSchedulerModalPolling();
    });
}

function refreshSchedulerModalDetails(showLoading = false) {
    if (!isSchedulerModalOpen() || schedulerModalRequestInFlight) {
        return;
    }
    if (showLoading) {
        renderSchedulerModalLoading();
    }
    schedulerModalRequestInFlight = true;
    listenWsOnceMatchingWithTimeout(wsClient, "SchedulerDetails", SCHEDULER_STATUS_TIMEOUT_MS, () => true, (msg) => {
        schedulerModalRequestInFlight = false;
        if (!isSchedulerModalOpen()) {
            stopSchedulerModalPolling();
            return;
        }
        if (!msg || !msg.data) {
            if (cachedSchedulerDetailsData) {
                updateSchedulerModalBody(renderSchedulerDetails(cachedSchedulerDetailsData, {
                    transportWarning: schedulerTransportWarningText(
                        t("scheduler.transport.details_empty"),
                        cachedSchedulerDetailsData
                    ),
                }));
            } else {
                updateSchedulerModalBody(`
                    <div class="alert alert-warning mb-0" role="alert">
                        <i class="fa fa-wifi me-2"></i>${escapeHtml(t("scheduler.modal.load_failed"))}
                    </div>`);
            }
            scheduleNextSchedulerModalPoll(SCHEDULER_MODAL_IDLE_POLL_MS);
            return;
        }
        cachedSchedulerDetailsData = msg.data;
        cachedSchedulerStatusData = msg.data;
        updateSchedulerModalBody(renderSchedulerDetails(msg.data));
        scheduleNextSchedulerModalPoll(schedulerModalPollDelay(msg.data));
    }, () => {
        schedulerModalRequestInFlight = false;
        if (!isSchedulerModalOpen()) {
            stopSchedulerModalPolling();
            return;
        }
        if (cachedSchedulerDetailsData) {
            updateSchedulerModalBody(renderSchedulerDetails(cachedSchedulerDetailsData, {
                transportWarning: schedulerTransportWarningText(
                    t("scheduler.transport.details_timeout"),
                    cachedSchedulerDetailsData
                ),
            }));
        } else {
            updateSchedulerModalBody(`
                <div class="alert alert-warning mb-0" role="alert">
                    <i class="fa fa-wifi me-2"></i>${escapeHtml(t("scheduler.modal.timeout"))}
                </div>`);
        }
        scheduleNextSchedulerModalPoll(SCHEDULER_MODAL_IDLE_POLL_MS);
    });
    wsClient.send({ SchedulerDetails: {} });
}

function openSchedulerModal() {
    const modalEl = document.getElementById('schedulerModal');
    if (!modalEl) return;
    ensureSchedulerModalLifecycle();
    stopSchedulerModalPolling();
    renderSchedulerModalLoading();
    if (!schedulerModalInstance) {
        schedulerModalInstance = new bootstrap.Modal(modalEl, { focus: true });
    }
    if (isSchedulerModalOpen()) {
        schedulerModalInstance.show();
        refreshSchedulerModalDetails();
        return;
    }
    modalEl.addEventListener('shown.bs.modal', () => {
        refreshSchedulerModalDetails();
    }, { once: true });
    schedulerModalInstance.show();
}

function getDeviceCounts() {
    listenOnce("DeviceCount", (msg) => {
        if (!msg || !msg.data) return;
        $("#shapedDeviceCount").text(msg.data.shaped_devices);
        $("#unknownIpCount").text(msg.data.unknown_ips);
    });
    wsClient.send({ DeviceCount: {} });
}

function initLogout() {
    $("#btnLogout").on('click', () => {
        //console.log("Logout");
        clearOperationalNetworkModeStorage();
        const cookies = document.cookie.split(";");

        for (let i = 0; i < cookies.length; i++) {
            const cookie = cookies[i];
            const eqPos = cookie.indexOf("=");
            const name = eqPos > -1 ? cookie.substr(0, eqPos) : cookie;
            // translatable: false — "=;expires=Thu, 01 Jan 1970 00:00:00 GMT" is the cookie expiry, not UI text
            document.cookie = name + "=;expires=Thu, 01 Jan 1970 00:00:00 GMT";
        }
        window.location.reload();
    });
}

let lastSearchTerm = "";
let searchHandlerReady = false;

function renderSearchResults(data) {
    let searchResults = document.getElementById("searchResults");
    // Position panel near the search input for consistent placement
    const inp = document.getElementById("txtSearch");
    if (inp && searchResults) {
        const rect = inp.getBoundingClientRect();
        // Use fixed positioning relative to viewport
        searchResults.style.position = 'fixed';
        searchResults.style.top = (rect.bottom + 8) + 'px';
        searchResults.style.left = rect.left + 'px';
        const widthPx = Math.max(320, rect.width + 200);
        searchResults.style.minWidth = widthPx + 'px';
        searchResults.style.width = widthPx + 'px';
        // Ensure it's not shifted or hidden by existing CSS
        searchResults.style.transform = 'none';
        searchResults.style.zIndex = '2000';
        searchResults.style.padding = '6px';
    }
    searchResults.style.visibility = "visible";
    let list = document.createElement("table");
    list.classList.add("lqos-table", "lqos-table-compact", "mb-0");
    let tbody = document.createElement("tbody");
    data.forEach((item) => {
        let r = document.createElement("tr");
        let c = document.createElement("td");

        if (item.Circuit !== undefined) {
            c.appendChild(searchResultLink(
                `/circuit.html?id=${encodeURIComponent(item.Circuit.id ?? "")}`,
                ["fa", "fa-user"],
                item.Circuit.name,
            ));
        } else if (item.Device !== undefined) {
            c.appendChild(searchResultLink(
                `/circuit.html?id=${encodeURIComponent(item.Device.circuit_id ?? "")}`,
                ["fa", "fa-computer"],
                item.Device.name,
            ));
        } else if (item.Site !== undefined) {
            c.appendChild(searchResultLink(
                `/tree.html?parent=${encodeURIComponent(item.Site.idx ?? "")}`,
                ["fa", "fa-building"],
                item.Site.name,
            ));
        } else {
            console.log(item);
            c.innerText = item;
        }
        r.appendChild(c);
        tbody.appendChild(r);
    });
    clearDiv(searchResults);
    list.appendChild(tbody);
    const wrap = document.createElement("div");
    wrap.classList.add("table-responsive", "lqos-table-wrap");
    wrap.appendChild(list);
    searchResults.appendChild(wrap);
}

function searchResultLink(href, iconClasses, label) {
    const link = document.createElement("a");
    link.classList.add("nav-link", "redactable");
    link.href = safeRelativeHref(href);

    const icon = document.createElement("i");
    icon.classList.add(...iconClasses);
    link.appendChild(icon);
    link.appendChild(document.createTextNode(` ${label ?? ""}`));
    return link;
}

function doSearch(search) {
    if (search.length > 2) {
        lastSearchTerm = search;
        if (!searchHandlerReady) {
            wsClient.on("SearchResults", (msg) => {
                if (!msg || msg.term !== lastSearchTerm) {
                    return;
                }
                renderSearchResults(msg.results || []);
            });
            searchHandlerReady = true;
        }
        wsClient.send({ Search: { term: search } });
    } else {
        // Close the search panel
        let searchResults = document.getElementById("searchResults");
        searchResults.style.visibility = "hidden";
    }
}

// Simple debounce helper
function debounce(fn, delay) {
    let timer = null;
    return function(...args) {
        clearTimeout(timer);
        timer = setTimeout(() => fn.apply(this, args), delay);
    }
}

function setupSearch() {
    const hideResults = () => {
        const panel = document.getElementById("searchResults");
        if (panel) panel.style.visibility = "hidden";
    };
    const showResults = () => {
        const panel = document.getElementById("searchResults");
        if (panel) panel.style.visibility = "visible";
    };

    $("#btnSearch").on('click', () => {
        const search = $("#txtSearch").val();
        doSearch(search);
    });
    const debouncedSearch = debounce(() => {
        const search = $("#txtSearch").val();
        doSearch(search);
    }, 300);
    $("#txtSearch").on('keyup', debouncedSearch);

    // Reposition results on resize/scroll to keep anchored under input on index
    const repositionResults = () => {
        const inp = document.getElementById('txtSearch');
        const panel = document.getElementById('searchResults');
        if (!inp || !panel || panel.style.visibility !== 'visible') return;
        const rect = inp.getBoundingClientRect();
        const widthPx = Math.max(320, rect.width + 200);
        panel.style.position = 'fixed';
        panel.style.top = (rect.bottom + 8) + 'px';
        panel.style.left = rect.left + 'px';
        panel.style.width = widthPx + 'px';
        panel.style.minWidth = widthPx + 'px';
        panel.style.transform = 'none';
        panel.style.zIndex = '2000';
        panel.style.padding = '6px';
    };
    window.addEventListener('resize', repositionResults);
    window.addEventListener('scroll', repositionResults, true);

    // Focus shows results if available
    $("#txtSearch").on('focus', () => {
        if ($("#txtSearch").val().length > 2) showResults();
    });
    // Blur hides results after short delay to allow clicking results
    $("#txtSearch").on('blur', () => {
        setTimeout(hideResults, 150);
    });

    // Add this new key handler for '/' to focus search
    $(document).on('keydown', (e) => {
        // translatable: false — "input, textarea, select" is a jQuery selector, not a label
        if (e.key === '/' && !$(e.target).is('input, textarea, select')) {
            e.preventDefault();
            $('#txtSearch').focus();
            showResults();
        // translatable: false — "Escape" is the keyboard event key compared against, not a label
        } else if (e.key === 'Escape') {
            hideResults();
            if ($(e.target).is('#txtSearch')) {
                $('#txtSearch').blur();
            }
        }
    });

    // Click-away to close results
    $(document).on('click', (e) => {
        // translatable: false — "#searchResults, #txtSearch, #btnSearch" is a DOM selector, not a label
        if ($(e.target).closest('#searchResults, #txtSearch, #btnSearch').length === 0) {
            hideResults();
        }
    });
    // Prevent clicks inside the results from bubbling
    $("#searchResults").on('click', (e) => { e.stopPropagation(); });
}

function inferQueueMode(config) {
    const queues = config && config.queues ? config.queues : {};
    if (queues.queue_mode === "observe" || queues.queue_mode === "shape") {
        return queues.queue_mode;
    }
    return queues.monitor_only ? "observe" : "shape";
}

function renderQueueModeState(mode, busy = false) {
    const nav = document.getElementById("observeShapeNav");
    const observeBtn = document.getElementById("btnQueueModeObserve");
    const shapeBtn = document.getElementById("btnQueueModeShape");
    if (!nav || !observeBtn || !shapeBtn) return;

    observeBtn.disabled = busy || mode === "observe";
    shapeBtn.disabled = busy || mode === "shape";
    observeBtn.className = `btn btn-sm ${mode === "observe" ? "btn-warning" : "btn-outline-secondary"}`;
    shapeBtn.className = `btn btn-sm ${mode === "shape" ? "btn-success" : "btn-outline-secondary"}`;
    const summary = document.getElementById("queueModeSummary");
    if (summary) {
        // The row's label, buttons and value all read from the catalog; the
        // English text matches the markup the server renders pre-translation.
        summary.textContent = mode === "observe" ? t("common.nav.observe") : t("common.nav.shape");
    }
}

function triggerReloadLibreQoS(loadingMessage) {
    const myModal = new bootstrap.Modal(document.getElementById('reloadModal'), { focus: true });
    myModal.show();
    $("#reloadLibreResult").html(`<i class='fa fa-spinner fa-spin'></i> ${escapeHtml(loadingMessage)}`);
    listenOnce("ReloadResult", (msg) => {
        if (!msg) {
            $("#reloadLibreResult").text(t("common.reload.failed"));
            return;
        }
        $("#reloadLibreResult").text(msg.message || "");
    });
    wsClient.send({ ReloadLibreQoS: {} });
}

function loadObserveShapeControl() {
    const nav = document.getElementById("observeShapeNav");
    if (!nav) return;

    listenOnce("AdminCheck", (msg) => {
        if (!msg || !msg.ok) {
            nav.classList.add("d-none");
            updateMaintenanceNav();
            return;
        }

        listenOnce("GetConfig", (cfgMsg) => {
            if (!cfgMsg || !cfgMsg.data || !cfgMsg.data.config) {
                nav.classList.add("d-none");
                updateMaintenanceNav();
                return;
            }
            cachedQueueModeConfig = cfgMsg.data.config;
            const mode = inferQueueMode(cachedQueueModeConfig);
            nav.classList.remove("d-none");
            renderQueueModeState(mode, false);
            updateMaintenanceNav();
        });
        wsClient.send({ GetConfig: {} });
    });
    wsClient.send({ AdminCheck: {} });
}

function saveQueueModeAndReload(nextMode) {
    if (!cachedQueueModeConfig || !cachedQueueModeConfig.queues) {
        alert(t("queue_mode.load_failed"));
        return;
    }

    const currentMode = inferQueueMode(cachedQueueModeConfig);
    if (currentMode === nextMode) {
        return;
    }

    const confirmed = window.confirm(
        nextMode === "observe"
            ? t("queue_mode.confirm.observe")
            : t("queue_mode.confirm.shape")
    );
    if (!confirmed) {
        return;
    }

    renderQueueModeState(currentMode, true);
    const updated = JSON.parse(JSON.stringify(cachedQueueModeConfig));
    updated.queues.queue_mode = nextMode;

    let done = false;
    const cleanup = () => {
        wsClient.off("UpdateConfigResult", onSuccess);
        // translatable: false — "Error" is a websocket event name, not UI text
        wsClient.off("Error", onError);
    };
    const onSuccess = (msg) => {
        if (done) return;
        done = true;
        cleanup();
        if (!msg || !msg.ok) {
            renderQueueModeState(currentMode, false);
            alert((msg && msg.message) ? msg.message : t("queue_mode.update_failed"));
            return;
        }
        cachedQueueModeConfig = updated;
        renderQueueModeState(nextMode, false);
        triggerReloadLibreQoS(t("common.reload.saving_queue_mode", {mode: nextMode}));
    };
    const onError = (msg) => {
        if (done) return;
        done = true;
        cleanup();
        renderQueueModeState(currentMode, false);
        alert((msg && msg.message) ? msg.message : t("queue_mode.update_failed"));
    };
    wsClient.on("UpdateConfigResult", onSuccess);
    wsClient.on("Error", onError);
    wsClient.send({ UpdateConfig: { config: updated } });
}

function initObserveShapeToggle() {
    $(document).off("click", "#btnQueueModeObserve").on("click", "#btnQueueModeObserve", () => {
        saveQueueModeAndReload("observe");
    });
    $(document).off("click", "#btnQueueModeShape").on("click", "#btnQueueModeShape", () => {
        saveQueueModeAndReload("shape");
    });
    loadObserveShapeControl();
}

function setupDynamicUrls() {
    // Construct Chat URL (port 9121)
    const chatUrl = "chatbot.html";

    // Update Chat link if it exists (only created when chatbot is available)
    const chatLink = document.getElementById('chatLink');
    if (chatLink) {
        // If server rendered a disabled span, swap it for an active link.
        if (chatLink.tagName && chatLink.tagName.toLowerCase() !== 'a') {
            const parentLi = chatLink.closest('li');
            const a = document.createElement('a');
            a.className = 'nav-link';
            a.id = 'chatLink';
            a.href = 'chatbot.html';
            a.innerHTML = '<i class="fa fa-fw fa-centerline fa-comments nav-icon"></i> Ask Libby';
            if (parentLi) parentLi.replaceChild(a, chatLink); else chatLink.replaceWith(a);
        } else {
            const hrefAttr = chatLink.getAttribute('href');
            if (hrefAttr === '%%CHAT_URL%%' || !hrefAttr) {
                chatLink.href = chatUrl;
            }
        }
    }
}

function initUrgentIssues() {
    const containerId = 'urgentStatus';
    const linkId = 'urgentStatusLink';
    const badgeId = 'urgentBadge';
    let urgentClearState = null;
    let urgentStatusPending = false;
    let urgentStatusRefreshQueued = false;
    let urgentListPending = false;
    let urgentListActiveRequestId = null;
    let urgentListCancel = null;
    let urgentRequestId = 0;
    let pendingUrgentActionError = null;
    let pendingUrgentRetryMessage = null;

    function ensurePlaceholder() {
        return document.getElementById(containerId) !== null;
    }

    function renderStatus(count) {
        const cont = document.getElementById(containerId);
        if (!cont) return;
        const cls = count > 0 ? 'text-danger' : 'text-secondary';
        const icon = count > 0 ? 'fa-bell' : 'fa-bell-slash';
        const label = t("common.nav.alerts");
        const ariaLabel = alertsAriaLabel(count);
        cont.innerHTML = `
            <button class="nav-link btn btn-link text-start w-100 border-0 ${cls}" type="button" id="${linkId}" aria-label="${escapeAttr(ariaLabel)}" title="${escapeAttr(label)}">
                <i class="fa fa-fw fa-centerline ${icon}" aria-hidden="true"></i> ${escapeHtml(label)}
                <span id="${badgeId}" class="badge bg-danger ${count>0?'':'d-none'}">${count}</span>
            </button>`;
        $("#" + containerId).off("click").on("click", `#${linkId}`, (e) => {
            e.preventDefault();
            showModal();
        });
    }

    function poll() {
        if (!ensurePlaceholder()) return;
        if (urgentStatusPending) {
            urgentStatusRefreshQueued = true;
            return;
        }
        urgentStatusPending = true;
        const requestId = ++urgentRequestId;
        listenWsOnceMatchingWithTimeout(
            wsClient,
            "UrgentStatus",
            URGENT_ISSUES_TIMEOUT_MS,
            (msg) => msg.request_id === requestId,
            (msg) => {
                urgentStatusPending = false;
                const count = msg && msg.data ? msg.data.count : 0;
                renderStatus(count || 0);
                runQueuedUrgentStatusRefresh();
            },
            () => {
                urgentStatusPending = false;
                runQueuedUrgentStatusRefresh();
            }
        );
        wsClient.send({ UrgentStatus: { request_id: requestId } });
    }

    function runQueuedUrgentStatusRefresh() {
        if (!urgentStatusRefreshQueued) return false;
        urgentStatusRefreshQueued = false;
        poll();
        return true;
    }

    function clearActionInFlight() {
        return urgentClearState !== null;
    }

    function clearActionBlocked() {
        return clearActionInFlight() || urgentListPending;
    }

    function setClearControlsDisabled(disabled) {
        document.querySelectorAll('.urgent-clear').forEach((button) => {
            button.disabled = disabled;
        });
        const clearAllButton = document.getElementById('urgentClearAll');
        if (clearAllButton) {
            clearAllButton.disabled = disabled;
        }
    }

    function showUrgentBusyState(message) {
        const holder = document.getElementById('urgentListContainer');
        if (!holder || !urgentModalIsOpen()) return;
        holder.setAttribute('aria-busy', 'true');
        holder.querySelector('.urgent-busy-status')?.remove();
        holder.insertAdjacentHTML('afterbegin', `
            <div class="alert alert-info mb-2 urgent-busy-status" role="status" aria-live="polite" tabindex="-1">
                <i class="fa fa-spinner fa-spin" aria-hidden="true"></i>
                ${escapeHtml(message)}
            </div>`);
        holder.querySelector('.urgent-busy-status')?.focus();
    }

    function clearUrgentBusyState(holder) {
        const target = holder || document.getElementById('urgentListContainer');
        if (!target) return;
        target.removeAttribute('aria-busy');
        target.querySelector('.urgent-busy-status')?.remove();
    }

    function renderUrgentWarning(holder, message, { focus = false, retry = false } = {}) {
        if (!holder) return;
        holder.querySelector('.urgent-action-error')?.remove();
        const retryButton = retry
            ? `<button type="button" class="btn btn-link btn-sm urgent-refresh">${escapeHtml(t("common.action.retry"))}</button>`
            : '';
        holder.insertAdjacentHTML('afterbegin', `
            <div class="alert alert-warning mb-2 urgent-action-error" role="alert" tabindex="-1">
                ${escapeHtml(message)}
                ${retryButton}
            </div>`);
        if (focus) {
            holder.querySelector(retry ? '.urgent-refresh' : '.urgent-action-error')?.focus();
        }
    }

    function showUrgentActionError(message) {
        pendingUrgentActionError = message;
        if (!urgentModalIsOpen()) return;
        const holder = document.getElementById('urgentListContainer');
        if (!holder) return;
        clearUrgentBusyState(holder);
        renderUrgentWarning(holder, message, { focus: true });
        pendingUrgentActionError = null;
    }

    function attachPendingUrgentActionError(holder) {
        if (!pendingUrgentActionError || !urgentModalIsOpen() || !holder) return;
        renderUrgentWarning(holder, pendingUrgentActionError);
        pendingUrgentActionError = null;
    }

    function showUrgentRetryState(message) {
        pendingUrgentRetryMessage = message;
        const holder = document.getElementById('urgentListContainer');
        if (!holder || !urgentModalIsOpen()) return;
        pendingUrgentRetryMessage = null;
        setClearControlsDisabled(true);
        clearUrgentBusyState(holder);
        holder.innerHTML = '';
        renderUrgentWarning(holder, message, { focus: true, retry: true });
        $(holder).off('click').on('click', '.urgent-refresh', function (e) {
            e.preventDefault();
            showModal();
        });
    }

    function showPendingUrgentRetryState() {
        if (!pendingUrgentRetryMessage || !urgentModalIsOpen()) return false;
        showUrgentRetryState(pendingUrgentRetryMessage);
        return true;
    }

    function urgentModalIsOpen() {
        return document.getElementById('urgentModal')?.classList.contains('show') === true;
    }

    function focusUrgentModalFallback() {
        const modalEl = document.getElementById('urgentModal');
        if (!modalEl || !urgentModalIsOpen()) return;
        // translatable: false — '.urgent-clear:not(:disabled), #urgentClearAll:not(:disabled), .btn-close:not(:disabled), [data-bs-dismiss="modal"]:not(:disabled)' is a CSS selector
        modalEl
            .querySelector('.urgent-clear:not(:disabled), #urgentClearAll:not(:disabled), .btn-close:not(:disabled), [data-bs-dismiss="modal"]:not(:disabled)')
            ?.focus();
    }

    function refreshUrgentIssues() {
        showModal();
        poll();
    }

    function completeClearAction() {
        urgentClearState = null;
        setClearControlsDisabled(clearActionInFlight());
        refreshUrgentIssues();
    }

    function startUrgentClear({
        responseEvent,
        requestPayload,
        busyMessage,
        failureMessage,
        timeoutMessage,
    }) {
        if (clearActionBlocked()) {
            return;
        }
        const requestId = ++urgentRequestId;
        setClearControlsDisabled(true);
        showUrgentBusyState(busyMessage);
        urgentClearState = {
            busyMessage,
            cancel: null,
        };
        urgentClearState.cancel = listenWsOnceMatchingWithTimeout(
            wsClient,
            responseEvent,
            URGENT_ISSUES_TIMEOUT_MS,
            (msg) => msg.request_id === requestId,
            (msg) => {
                urgentClearState = null;
                if (msg && msg.ok === false) {
                    setClearControlsDisabled(false);
                    showUrgentActionError(failureMessage);
                    return;
                }
                completeClearAction();
            },
            () => {
                urgentClearState = null;
                setClearControlsDisabled(false);
                showUrgentActionError(timeoutMessage);
            }
        );
        wsClient.send(requestPayload(requestId));
    }

    function showModal() {
        const modalEl = document.getElementById('urgentModal');
        if (!modalEl) {
            return;
        }
        bootstrap.Modal.getOrCreateInstance(modalEl, { focus: true }).show();
        const holder = document.getElementById('urgentListContainer');
        if (!holder) {
            return;
        }
        if (showPendingUrgentRetryState()) {
            return;
        }
        if (clearActionInFlight()) {
            setClearControlsDisabled(true);
            showUrgentBusyState(urgentClearState.busyMessage);
            return;
        }
        if (urgentListPending) {
            setClearControlsDisabled(true);
            return;
        }
        urgentListPending = true;
        const requestId = ++urgentRequestId;
        urgentListActiveRequestId = requestId;
        setClearControlsDisabled(true);
        clearUrgentBusyState(holder);
        holder.innerHTML = `<div class="text-center text-muted urgent-loading" role="status" aria-live="polite" tabindex="-1"><i class='fa fa-spinner fa-spin'></i> ${escapeHtml(t("common.state.loading"))}</div>`;
        holder.querySelector('.urgent-loading')?.focus();
        urgentListCancel = listenWsOnceMatchingWithTimeout(
            wsClient,
            "UrgentList",
            URGENT_ISSUES_TIMEOUT_MS,
            (msg) => msg.request_id === requestId && urgentListActiveRequestId === requestId,
            (msg) => {
                urgentListPending = false;
                urgentListActiveRequestId = null;
                urgentListCancel = null;
                const items = msg && msg.data ? msg.data.items || [] : [];
                clearUrgentBusyState(holder);
                if (items.length === 0) {
                    holder.innerHTML = `<div class="text-center text-success" role="status" aria-live="polite" tabindex="-1">${escapeHtml(t("common.alerts.empty"))}</div>`;
                    attachPendingUrgentActionError(holder);
                    setClearControlsDisabled(true);
                    holder.querySelector('[role="status"]')?.focus();
                    return;
                }
                const table = document.createElement('table');
                table.className = 'lqos-table lqos-table-compact mb-0';
                const tbody = document.createElement('tbody');
                items.forEach((it) => {
                    const tr = document.createElement('tr');
                    const td = document.createElement('td');
                    const alertTime = Number(it.ts) * 1000;
                    const when = Number.isFinite(alertTime) ? formatDateTime(alertTime) : "—";
                    const sev = it.severity === 'Error' ? 'danger' : 'warning';
                    const clearControl = it.clearable === false
                        ? `<span class="badge bg-secondary-subtle text-secondary border float-end ms-3">${escapeHtml(t("common.alerts.active"))}</span>`
                        : `<button type="button" class="btn btn-link btn-sm text-secondary float-end ms-3 p-0 urgent-clear" data-id="${escapeAttr(it.id)}" title="${escapeAttr(t("common.alerts.acknowledge"))}" aria-label="${escapeAttr(t("common.alerts.acknowledge_code", {code: it.code}))}"><i class="fa fa-times" aria-hidden="true"></i></button>`;
                    // Severity, code, source, message and context are the server's
                    // (alert messages are composed from node and circuit names, and
                    // the server only strips ANSI), so every one of them is escaped.
                    td.innerHTML = `
                    <div>
                        <span class="badge bg-${sev}">${escapeHtml(it.severity)}</span>
                        <strong class="ms-2">${escapeHtml(it.code)}</strong>
                        <span class="text-muted ms-2">(${escapeHtml(it.source)})</span>
                        <span class="text-muted float-end">${when}</span>
                        ${clearControl}
                    </div>
                    <div class="mt-1" style="white-space: pre-wrap;">${escapeHtml(it.message)}</div>
                    ${it.context ? `<pre class="mt-2">${escapeHtml(it.context)}</pre>` : ''}
                    `;
                    tr.appendChild(td);
                    tbody.appendChild(tr);
                });
                table.appendChild(tbody);
                holder.innerHTML = '';
                const tableWrap = document.createElement('div');
                tableWrap.className = 'table-responsive lqos-table-wrap';
                tableWrap.appendChild(table);
                holder.appendChild(tableWrap);
                attachPendingUrgentActionError(holder);
                $(holder).off('click').on('click', '.urgent-clear', function (e) {
                    e.preventDefault();
                    const id = $(this).data('id');
                    startUrgentClear({
                        responseEvent: 'UrgentClearResult',
                        requestPayload: (requestId) => ({ UrgentClear: { id, request_id: requestId } }),
                        busyMessage: t("common.alerts.acknowledging"),
                        failureMessage: t("common.alerts.acknowledge_failed"),
                        timeoutMessage: t("common.alerts.acknowledge_timeout"),
                    });
                });
                setClearControlsDisabled(clearActionInFlight());
                focusUrgentModalFallback();
            },
            () => {
                if (urgentListActiveRequestId !== requestId) return;
                urgentListPending = false;
                urgentListActiveRequestId = null;
                urgentListCancel = null;
                if (!urgentModalIsOpen()) {
                    setClearControlsDisabled(clearActionInFlight());
                    return;
                }
                showUrgentRetryState(t("common.alerts.refresh_failed"));
                setClearControlsDisabled(true);
            }
        );
        wsClient.send({ UrgentList: { request_id: requestId } });
    }

    if (!document.getElementById(containerId)) {
        // translatable: false — '.sidebar .navbar-nav' is a DOM selector, not a label
        const ul = document.querySelector('.sidebar .navbar-nav');
        if (ul) {
            const li = document.createElement('li');
            li.className = 'nav-item';
            li.id = containerId;
            ul.appendChild(li);
        }
    }

    $(document).off('click', '#urgentClearAll').on('click', '#urgentClearAll', () => {
        startUrgentClear({
            responseEvent: 'UrgentClearAllResult',
            requestPayload: (requestId) => ({ UrgentClearAll: { request_id: requestId } }),
            busyMessage: t("common.alerts.acknowledging_all"),
            failureMessage: t("common.alerts.acknowledge_all_failed"),
            timeoutMessage: t("common.alerts.acknowledge_all_timeout"),
        });
    });

    $(document).off('hidden.bs.modal', '#urgentModal').on('hidden.bs.modal', '#urgentModal', () => {
        pendingUrgentRetryMessage = null;
        if (urgentClearState?.cancel) {
            urgentClearState.cancel();
            urgentClearState = null;
        }
        if (urgentListPending) {
            if (urgentListCancel) {
                urgentListCancel();
                urgentListCancel = null;
            }
            urgentListPending = false;
            urgentListActiveRequestId = null;
        }
        clearUrgentBusyState();
        setClearControlsDisabled(clearActionInFlight());
    });

    poll();
    setInterval(poll, 30000);
}

function updateMaintenanceNav() {
    const nav = document.getElementById("maintenanceNav");
    const reload = document.getElementById("reloadLibreQoSItem");
    const queue = document.getElementById("observeShapeNav");
    if (!nav) return;
    const show = (reload && !reload.classList.contains("d-none"))
        || (queue && !queue.classList.contains("d-none"));
    nav.classList.toggle("d-none", !show);
}

function markActiveNav() {
    const page = (window.location.pathname.split("/").pop() || "index.html").toLowerCase();
    // translatable: false — ".sidebar .nav-link[href]" is a DOM selector, not a label
    document.querySelectorAll(".sidebar .nav-link[href]").forEach((link) => {
        if (link.classList.contains("lqos-brand-link")) {
            return;
        }
        const href = (link.getAttribute("href") || "").split("?")[0].split("#")[0].toLowerCase();
        if (href && href === page) {
            link.classList.add("active");
        }
    });
}

function initReloadLibreQoS() {
    // v2's explicit standalone "Reload LibreQoS" sidebar action (the queue-mode
    // switch reloads too). It asks the scheduler — the single apply owner — to
    // re-sync the integration and apply now; the DPDK bridge is never restarted
    // and traffic is not interrupted. An ADMIN affordance: hidden for read-only
    // roles (the backend re-checks).
    const item = document.getElementById("reloadLibreQoSItem");
    const link = document.getElementById("reloadLibreQoSLink");
    if (!item || !link) {
        return;
    }
    if (!isAdminRole(window.USER_ROLE)) {
        item.classList.add("d-none");
        updateMaintenanceNav();
        return;
    }
    item.classList.remove("d-none");
    updateMaintenanceNav();
    link.addEventListener("click", (event) => {
        event.preventDefault();
        // The sentence about the bridge lives in `common.reload.bridge_note`,
        // which the modal body also renders; the prompt composes it rather than
        // keeping a second copy that a translator can drift from.
        const message = `${t("common.reload.confirm")} ${t("common.reload.bridge_note")}`;
        if (!window.confirm(message)) {
            return;
        }
        triggerReloadLibreQoS(t("common.reload.resyncing"));
    });
}

function initSchedulerTooltips() {
    // Initialize Bootstrap tooltips for scheduler status elements
    const schedulerElements = document.querySelectorAll('[data-bs-toggle="tooltip"]');
    schedulerElements.forEach(element => {
        new bootstrap.Tooltip(element);
    });
}

// The sidebar's language options are built by `i18n/switcher.mjs` from the
// generated `i18n/locales.js` list, so the tags are never repeated in this file.

// The frame is translated once, before any page code re-renders a status item.
initLanguageSwitcher();
applyDomTranslations();
// The cancellation notice is filled here because one of its placements, the
// post-signup trial page, ships no bundle of its own; every other page that
// marks the element is filled by the same call.
applyCancelCapNotice();

initLogout();
initDayNightMode();
initRedact();
initColorBlind();
getDeviceCounts();
setupSearch();
initObserveShapeToggle();
setupDynamicUrls();
initReloadLibreQoS();
markActiveNav();
updateMaintenanceNav();
window.lqosInitUrgentIssues = initUrgentIssues;
initSchedulerTooltips();
restartSchedulerStatusPolling();
loadBridgeHealthStatus();

document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
        restartSchedulerStatusPolling();
        loadBridgeHealthStatus();
    }
});
