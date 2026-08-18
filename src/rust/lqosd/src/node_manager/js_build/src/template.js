import {clearDiv, safeRelativeHref} from "./helpers/builders";
import {initRedact} from "./helpers/redact";
import {initDayNightMode} from "./helpers/dark_mode";
import {initColorBlind} from "./helpers/colorblind";
import {listenOnceMatchingWithTimeout as listenWsOnceMatchingWithTimeout} from "./pubsub/listeners";
import {get_ws_client} from "./pubsub/ws";
import {clearOperationalNetworkModeStorage} from "./config/network_mode_storage.mjs";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function escapeAttr(text) {
    if (text === undefined || text === null) return "";
    return String(text)
        .replaceAll('&', '&amp;')
        .replaceAll('"', '&quot;')
        .replaceAll("'", '&apos;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;');
}

function escapeHtml(text) {
    return escapeAttr(text);
}

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

function clampSchedulerPercent(value) {
    const parsed = Number(value);
    if (!Number.isFinite(parsed)) return 0;
    return Math.max(0, Math.min(100, Math.round(parsed)));
}

function schedulerProgressPercent(progress) {
    if (!progress) return 0;
    if (progress.percent !== undefined && progress.percent !== null) {
        return clampSchedulerPercent(progress.percent);
    }
    const stepCount = Number(progress.step_count);
    const stepIndex = Number(progress.step_index);
    if (!Number.isFinite(stepCount) || stepCount <= 0 || !Number.isFinite(stepIndex)) {
        return 0;
    }
    const boundedStep = Math.max(1, Math.min(stepCount, stepIndex));
    const completedSteps = progress.active ? boundedStep - 1 : boundedStep;
    return clampSchedulerPercent((completedSteps / stepCount) * 100);
}

function schedulerProgressSummary(progress) {
    if (!progress) return "";
    const label = progress.phase_label || progress.phase || "Scheduler activity";
    const percent = schedulerProgressPercent(progress);
    const stepIndex = Number(progress.step_index);
    const stepCount = Number(progress.step_count);
    if (Number.isFinite(stepIndex) && Number.isFinite(stepCount) && stepCount > 0) {
        return `${label} (${percent}%, step ${stepIndex}/${stepCount})`;
    }
    return `${label} (${percent}%)`;
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

function schedulerRelativeTime(updatedUnix) {
    const unixSeconds = Number(updatedUnix);
    if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) {
        return "Update time unavailable";
    }
    const deltaSeconds = Math.max(0, Math.round((Date.now() / 1000) - unixSeconds));
    if (deltaSeconds < 5) return "Updated just now";
    if (deltaSeconds < 60) return `Updated ${deltaSeconds}s ago`;
    const deltaMinutes = Math.round(deltaSeconds / 60);
    if (deltaMinutes < 60) return `Updated ${deltaMinutes}m ago`;
    const deltaHours = Math.round(deltaMinutes / 60);
    if (deltaHours < 24) return `Updated ${deltaHours}h ago`;
    const deltaDays = Math.round(deltaHours / 24);
    return `Updated ${deltaDays}d ago`;
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
        return "Update time unavailable";
    }
    return schedulerRelativeTime(updatedUnix);
}

function schedulerTransportWarningText(message, data) {
    const updatedText = schedulerUpdatedText(data);
    if (updatedText === "Update time unavailable") {
        return `${message} Retrying in the background.`;
    }
    return `${message} Showing cached scheduler data. ${updatedText}.`;
}

function schedulerStaleStatusLabel(data) {
    const updatedText = schedulerUpdatedText(data);
    if (updatedText === "Update time unavailable") {
        return "Scheduler status is stale";
    }
    return `Scheduler status is stale. ${updatedText}.`;
}

function schedulerLooksRecovered(data) {
    const progress = data?.progress || null;
    if (!data?.available || !progress || progress.active) {
        return false;
    }
    return String(progress.phase || "").trim() === "ready";
}

function schedulerHasLiveError(data) {
    const errorText = schedulerErrorText(data);
    if (!errorText) {
        return false;
    }
    return !schedulerLooksRecovered(data);
}

function schedulerWaitsForTopologyRuntime(data) {
    const progress = data?.progress || null;
    if (!progress) {
        return false;
    }
    return String(progress.phase || "").trim() === "waiting_for_topology_runtime";
}

function schedulerStateDescriptor(data) {
    const progress = data?.progress || null;
    const hasError = schedulerHasLiveError(data);
    const available = !!data?.available;
    const active = !!progress?.active;
    const setupRequired = !!data?.setup_required;
    const topologyRuntimeWait = schedulerWaitsForTopologyRuntime(data);

    if (hasError) {
        return {
            tone: "danger",
            badgeClass: "text-bg-danger",
            label: "Error",
            title: "Scheduler reported an internal error",
            subtitle: progress?.phase_label || progress?.phase || "Scheduler needs attention",
            ringTone: "tone-danger",
            icon: "fa-triangle-exclamation",
        };
    }
    if (setupRequired) {
        return {
            tone: "warning",
            badgeClass: "text-bg-warning",
            label: "Setup Required",
            title: "Choose a topology source",
            subtitle: data?.setup_message || "LibreQoS needs subscriber data before the scheduler can run",
            ringTone: "tone-warning",
            icon: "fa-list-check",
        };
    }
    if (data?.stale) {
        return {
            tone: "warning",
            badgeClass: "text-bg-warning",
            label: "Stale",
            title: "Scheduler status is stale",
            subtitle: "Showing the last scheduler snapshot older than five minutes",
            ringTone: "tone-warning",
            icon: "fa-clock",
        };
    }
    if (topologyRuntimeWait) {
        return {
            tone: "warning",
            badgeClass: "text-bg-warning",
            label: "Deferred",
            title: progress?.phase_label || "Waiting for topology runtime",
            subtitle: "Scheduler is waiting for topology runtime outputs",
            ringTone: "tone-warning",
            icon: "fa-hourglass-half",
        };
    }
    if (active) {
        return {
            tone: "info",
            badgeClass: "text-bg-info",
            label: "Running",
            title: progress?.phase_label || progress?.phase || "Scheduler is running",
            subtitle: "Work is in progress right now",
            ringTone: "tone-progress",
            icon: "fa-arrows-rotate",
        };
    }
    if (available) {
        return {
            tone: "success",
            badgeClass: "text-bg-success",
            label: "Idle",
            title: progress?.phase_label || progress?.phase || "Scheduler ready",
            subtitle: "No active scheduler work at the moment",
            ringTone: "tone-success",
            icon: "fa-check",
        };
    }
    return {
        tone: "warning",
        badgeClass: "text-bg-warning",
        label: "Starting",
        title: "Scheduler is not ready yet",
        subtitle: progress?.phase_label || progress?.phase || "Waiting for scheduler status",
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
        return "No recent scheduler output recorded";
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
            "Open Complete Setup to choose an integration or provide manual subscriber data.",
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

function renderSchedulerDetails(data, options = {}) {
    const progress = data?.progress || null;
    const descriptor = schedulerStateDescriptor(data);
    const liveError = schedulerHasLiveError(data) ? schedulerErrorText(data) : "";
    const historicalError = !liveError ? schedulerErrorText(data) : "";
    const percent = progress ? schedulerProgressPercent(progress) : (data?.available ? 100 : 0);
    const phaseLabel = progress?.phase_label || progress?.phase || descriptor.title;
    const stepCount = Number(progress?.step_count);
    const stepIndex = Number(progress?.step_index);
    const stepText = Number.isFinite(stepIndex) && Number.isFinite(stepCount) && stepCount > 0
        ? `Step ${stepIndex} of ${stepCount}`
        : "No active step reported";
    const updatedText = schedulerUpdatedText(data);
    const availabilityText = data?.setup_required
        ? "Setup Required"
        : data?.stale
            ? "Stale"
            : schedulerWaitsForTopologyRuntime(data)
                ? "Deferred"
            : (data?.available ? "Healthy" : "Unavailable");
    const recentResult = schedulerSetupMessage(data) || summarizeSchedulerOutput(data?.output, historicalError || liveError);
    const activity = schedulerActivityItems(data?.output, historicalError || liveError, data);
    let progressMeta = "Waiting for activity";
    if (options.transportWarning) {
        progressMeta = "Showing cached scheduler details while the UI reconnects";
    } else if (data?.setup_required) {
        progressMeta = "Complete runtime setup to enable scheduler work";
    } else if (schedulerWaitsForTopologyRuntime(data)) {
        progressMeta = "The next shaping update is waiting for topology runtime outputs";
    } else if (progress?.active) {
        progressMeta = `${percent}% complete`;
    } else if (descriptor.label === "Idle") {
        progressMeta = "Last run complete";
    }
    const setupAlert = data?.setup_required
        ? `<div class="alert alert-warning mt-3 mb-0" role="alert"><i class="fa fa-list-check me-2"></i>${escapeHtml(schedulerSetupMessage(data) || "Choose a topology source in Complete Setup before expecting scheduler activity.")}</div>`
        : "";
    const transportWarningMarkup = options.transportWarning
        ? `<div class="alert alert-warning mt-3 mb-0" role="alert"><i class="fa fa-wifi me-2"></i>${escapeHtml(options.transportWarning)}</div>`
        : "";
    const alertMarkup = liveError
        ? `<div class="alert alert-danger mt-3 mb-0" role="alert"><i class="fa fa-triangle-exclamation me-2"></i>${escapeHtml(liveError)}</div>`
        : "";
    const activityMarkup = activity.length
        ? activity.map(line => `
            <div class="lqos-scheduler-activity-item">
                <i class="fa fa-angle-right" aria-hidden="true"></i>
                <div>${escapeHtml(line)}</div>
            </div>`).join("")
        : `<p class="lqos-scheduler-empty">No recent scheduler output recorded.</p>`;

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
                <p class="lqos-scheduler-modal-subtitle">${escapeHtml(phaseLabel)}</p>
                <div class="lqos-scheduler-modal-updated">${escapeHtml(updatedText)}</div>
            </div>
        </div>
        ${setupAlert}
        ${transportWarningMarkup}
        ${alertMarkup}
        <div class="lqos-scheduler-progress-card">
            <div class="lqos-scheduler-progress-topline">
                <div>
                    <h3 class="lqos-scheduler-progress-title">${escapeHtml(stepText)}</h3>
                    <p class="lqos-scheduler-progress-meta mb-0">${escapeHtml(progressMeta)}</p>
                </div>
                <div class="fw-semibold">${escapeHtml(`${percent}%`)}</div>
            </div>
            <div class="progress lqos-scheduler-progress-bar" role="progressbar" aria-label="Scheduler progress" aria-valuenow="${percent}" aria-valuemin="0" aria-valuemax="100">
                <div class="progress-bar ${schedulerProgressBarClass(descriptor)}" style="width: ${percent}%"></div>
            </div>
            <div class="lqos-scheduler-stat-grid">
                <div class="lqos-scheduler-stat">
                    <span class="lqos-scheduler-stat-label">Availability</span>
                    <span class="lqos-scheduler-stat-value">${escapeHtml(availabilityText)}</span>
                </div>
                <div class="lqos-scheduler-stat">
                    <span class="lqos-scheduler-stat-label">Current Phase</span>
                    <span class="lqos-scheduler-stat-value">${escapeHtml(phaseLabel)}</span>
                </div>
                <div class="lqos-scheduler-stat">
                    <span class="lqos-scheduler-stat-label">Progress</span>
                    <span class="lqos-scheduler-stat-value">${escapeHtml(stepText)}</span>
                </div>
                <div class="lqos-scheduler-stat">
                    <span class="lqos-scheduler-stat-label">Recent Result</span>
                    <span class="lqos-scheduler-stat-value">${escapeHtml(recentResult)}</span>
                </div>
            </div>
        </div>
        <div class="lqos-scheduler-activity-card">
            <h3 class="lqos-scheduler-section-title">Recent Activity</h3>
            <div class="lqos-scheduler-activity-list">
                ${activityMarkup}
            </div>
        </div>
        <details class="lqos-scheduler-debug-card">
            <summary class="lqos-scheduler-debug-toggle">Show raw scheduler details</summary>
            <pre class="lqos-scheduler-debug-pre">${escapeHtml(data?.details || "No raw scheduler details available.")}</pre>
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
}

function renderSchedulerStatus(container, state, progress, labelOverride = null) {
    if (!container) return;

    let color = "text-secondary";
    let label = "Scheduler status is loading";
    const buttonText = "Scheduler";
    let indicator = schedulerRingMarkup(12, "tone-loading", "fa-circle-notch fa-spin");

    if (state === "healthy") {
        color = "text-success";
        label = "Scheduler is available";
        indicator = schedulerRingMarkup(100, "tone-success", "fa-check");
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
        label = "Scheduler is still unavailable";
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-clock");
    } else if (state === "error") {
        color = "text-danger";
        label = "Scheduler has an internal error";
        indicator = schedulerRingMarkup(100, "tone-danger", "fa-triangle-exclamation");
    } else if (state === "stale") {
        color = "text-warning";
        label = "Scheduler status is stale";
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-clock");
    } else if (state === "deferred") {
        color = "text-warning";
        label = labelOverride || "Scheduler is waiting for topology runtime";
        indicator = schedulerRingMarkup(
            schedulerProgressPercent(progress),
            "tone-warning",
            "fa-hourglass-half"
        );
    } else if (state === "setup") {
        color = "text-warning";
        label = "Scheduler needs topology setup";
        indicator = schedulerRingMarkup(100, "tone-warning", "fa-list-check");
    }
    if (labelOverride) {
        label = labelOverride;
    }

    container.innerHTML = `
        <button class="nav-link btn btn-link text-start w-100 p-0 border-0 ${color} lqos-scheduler-link" type="button" id="schedulerStatusLink" aria-label="${escapeAttr(label)}" title="${escapeAttr(label)}">
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
                    schedulerTransportWarningText("Scheduler status response was empty.", cachedSchedulerStatusData)
                );
            } else {
                renderSchedulerStatus(container, "unavailable", null);
            }
            scheduleNextSchedulerStatusPoll();
            return;
        }
        const data = msg.data;
        cachedSchedulerStatusData = data;
        const hasError = schedulerHasLiveError(data);
        const isHealthy = !!data.available && !hasError;
        const progress = data.progress || null;
        const progressActive = !!(progress && progress.active);
        const setupRequired = !!data.setup_required;
        const stale = !!data.stale;
        const elapsed = schedulerStatusFirstRequestedAt === null ? 0 : (Date.now() - schedulerStatusFirstRequestedAt);

        if (hasError) {
            renderSchedulerStatus(container, "error", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (setupRequired) {
            renderSchedulerStatus(container, "setup", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (stale) {
            renderSchedulerStatus(container, "stale", progress, schedulerStaleStatusLabel(data));
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (schedulerWaitsForTopologyRuntime(data)) {
            renderSchedulerStatus(
                container,
                "deferred",
                progress,
                progress?.phase_label || "Scheduler is waiting for topology runtime"
            );
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (progressActive) {
            renderSchedulerStatus(container, "progress", progress);
            scheduleNextSchedulerStatusPoll();
            return;
        }

        if (isHealthy) {
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
                schedulerTransportWarningText("Scheduler status refresh timed out.", cachedSchedulerStatusData)
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
            <span>Loading scheduler status...</span>
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
                        "Scheduler details response was empty.",
                        cachedSchedulerDetailsData
                    ),
                }));
            } else {
                updateSchedulerModalBody(`
                    <div class="alert alert-warning mb-0" role="alert">
                        <i class="fa fa-wifi me-2"></i>${escapeHtml("Failed to load scheduler details. Retrying in the background.")}
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
                    "Scheduler details refresh timed out.",
                    cachedSchedulerDetailsData
                ),
            }));
        } else {
            updateSchedulerModalBody(`
                <div class="alert alert-warning mb-0" role="alert">
                    <i class="fa fa-wifi me-2"></i>${escapeHtml("Timed out while loading scheduler details. Retrying in the background.")}
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
        if (e.key === '/' && !$(e.target).is('input, textarea, select')) {
            e.preventDefault();
            $('#txtSearch').focus();
            showResults();
        } else if (e.key === 'Escape') {
            hideResults();
            if ($(e.target).is('#txtSearch')) {
                $('#txtSearch').blur();
            }
        }
    });

    // Click-away to close results
    $(document).on('click', (e) => {
        if ($(e.target).closest('#searchResults, #txtSearch, #btnSearch').length === 0) {
            hideResults();
        }
    });
    // Prevent clicks inside the results from bubbling
    $("#searchResults").on('click', (e) => { e.stopPropagation(); });
}

function setupReload() {
    let link = document.getElementById("lnkReloadLqos");
    link.onclick = () => {
        triggerReloadLibreQoS("Reloading LibreQoS...");
    }
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
}

function triggerReloadLibreQoS(loadingMessage) {
    const myModal = new bootstrap.Modal(document.getElementById('reloadModal'), { focus: true });
    myModal.show();
    $("#reloadLibreResult").html(`<i class='fa fa-spinner fa-spin'></i> ${loadingMessage}`);
    listenOnce("ReloadResult", (msg) => {
        if (!msg) {
            $("#reloadLibreResult").text("Failed to reload LibreQoS");
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
            return;
        }

        listenOnce("GetConfig", (cfgMsg) => {
            if (!cfgMsg || !cfgMsg.data || !cfgMsg.data.config) {
                nav.classList.add("d-none");
                return;
            }
            cachedQueueModeConfig = cfgMsg.data.config;
            const mode = inferQueueMode(cachedQueueModeConfig);
            nav.classList.remove("d-none");
            renderQueueModeState(mode, false);
        });
        wsClient.send({ GetConfig: {} });
    });
    wsClient.send({ AdminCheck: {} });
}

function saveQueueModeAndReload(nextMode) {
    if (!cachedQueueModeConfig || !cachedQueueModeConfig.queues) {
        alert("Unable to load current configuration");
        return;
    }

    const currentMode = inferQueueMode(cachedQueueModeConfig);
    if (currentMode === nextMode) {
        return;
    }

    const confirmed = window.confirm(
        nextMode === "observe"
            ? "Switch LibreQoS to Observe mode? LibreQoS will reload without the active shaping tree so you can gather a true baseline. Traffic may briefly pause during the reload, typically for about 5 seconds."
            : "Switch LibreQoS to Shape mode? This reload will apply the LibreQoS shaping tree and begin active shaping. It may briefly interrupt traffic."
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
        wsClient.off("Error", onError);
    };
    const onSuccess = (msg) => {
        if (done) return;
        done = true;
        cleanup();
        if (!msg || !msg.ok) {
            renderQueueModeState(currentMode, false);
            alert((msg && msg.message) ? msg.message : "Failed to update configuration");
            return;
        }
        cachedQueueModeConfig = updated;
        renderQueueModeState(nextMode, false);
        triggerReloadLibreQoS(`Saving queue mode '${nextMode}' and reloading LibreQoS...`);
    };
    const onError = (msg) => {
        if (done) return;
        done = true;
        cleanup();
        renderQueueModeState(currentMode, false);
        alert((msg && msg.message) ? msg.message : "Failed to update configuration");
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
    const apiUrl = window.location.protocol === "https:"
        ? "/api/v1/api-docs/"
        : `${window.location.protocol}//${window.location.hostname}:9122/api-docs/`;

    // Construct Chat URL (port 9121)
    const chatUrl = "chatbot.html";

    // Update API link only if it has the placeholder
    const apiLink = document.getElementById('apiLink');
    if (apiLink) {
        const hrefAttr = apiLink.getAttribute('href');
        if (hrefAttr === '%%API_URL%%') {
            apiLink.href = apiUrl;
        }
    }
    
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
        cont.innerHTML = `
            <button class="nav-link btn btn-link text-start w-100 p-0 border-0 ${cls}" type="button" id="${linkId}" aria-label="Urgent issues${count > 0 ? `: ${count} active` : ': none active'}" title="Urgent issues">
                <i class="fa fa-fw fa-centerline ${icon}" aria-hidden="true"></i> Urgent Issues
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
            ? '<button type="button" class="btn btn-link btn-sm urgent-refresh">Retry</button>'
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
        holder.innerHTML = `<div class="text-center text-muted urgent-loading" role="status" aria-live="polite" tabindex="-1"><i class='fa fa-spinner fa-spin'></i> Loading...</div>`;
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
                    holder.innerHTML = '<div class="text-center text-success" role="status" aria-live="polite" tabindex="-1">No urgent issues.</div>';
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
                    const when = new Date(it.ts * 1000).toLocaleString();
                    const sev = it.severity === 'Error' ? 'danger' : 'warning';
                    const clearControl = it.clearable === false
                        ? '<span class="badge bg-secondary-subtle text-secondary border float-end ms-3">Active</span>'
                        : `<button type="button" class="btn btn-link btn-sm text-secondary float-end ms-3 p-0 urgent-clear" data-id="${it.id}" title="Acknowledge issue" aria-label="Acknowledge issue ${escapeAttr(it.code)}"><i class="fa fa-times" aria-hidden="true"></i></button>`;
                    td.innerHTML = `
                    <div>
                        <span class="badge bg-${sev}">${it.severity}</span>
                        <strong class="ms-2">${it.code}</strong>
                        <span class="text-muted ms-2">(${it.source})</span>
                        <span class="text-muted float-end">${when}</span>
                        ${clearControl}
                    </div>
                    <div class="mt-1" style="white-space: pre-wrap;">${it.message}</div>
                    ${it.context ? `<pre class="mt-2">${it.context}</pre>` : ''}
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
                        busyMessage: 'Acknowledging urgent issue...',
                        failureMessage: 'Unable to acknowledge urgent issue.',
                        timeoutMessage: 'Timed out while acknowledging urgent issue.',
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
                showUrgentRetryState('Unable to refresh urgent issues.');
                setClearControlsDisabled(true);
            }
        );
        wsClient.send({ UrgentList: { request_id: requestId } });
    }

    if (!document.getElementById(containerId)) {
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
            busyMessage: 'Acknowledging urgent issues...',
            failureMessage: 'Unable to acknowledge urgent issues.',
            timeoutMessage: 'Timed out while acknowledging urgent issues.',
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

function initSchedulerTooltips() {
    // Initialize Bootstrap tooltips for scheduler status elements
    const schedulerElements = document.querySelectorAll('[data-bs-toggle="tooltip"]');
    schedulerElements.forEach(element => {
        new bootstrap.Tooltip(element);
    });
}

initLogout();
initDayNightMode();
initRedact();
initColorBlind();
getDeviceCounts();
setupSearch();
setupReload();
initObserveShapeToggle();
setupDynamicUrls();
window.lqosInitUrgentIssues = initUrgentIssues;
initSchedulerTooltips();
restartSchedulerStatusPolling();

document.addEventListener("visibilitychange", () => {
    if (!document.hidden) {
        restartSchedulerStatusPolling();
    }
});
