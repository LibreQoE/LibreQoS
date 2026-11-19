// L4 (`docs/control_plane_structural_convergence_card.md` §5): the Bridge page.
// Shows control-plane health + convergence, the HELD op in operator language,
// recent publish outcomes, and the reload action. No bridge restart action —
// the control plane quarantines and converges automatically.
import {get_ws_client} from "./pubsub/ws";
import {formatDateTime, formatTime} from "./i18n/format.js";
import {t, tHtml, tn} from "./i18n/index.js";

const wsClient = get_ws_client();
let reloadBound = false;

// translatable: false — "Error" is the pubsub client's websocket event name, not UI text
function sendWsRequest(responseEvent, request) {
    return new Promise((resolve, reject) => {
        let done = false;
        const responseHandler = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, responseHandler);
            wsClient.off("Error", errorHandler);
            resolve(msg);
        };
        const errorHandler = (msg) => {
            if (done) return;
            done = true;
            wsClient.off(responseEvent, responseHandler);
            wsClient.off("Error", errorHandler);
            reject(msg);
        };
        wsClient.on(responseEvent, responseHandler);
        wsClient.on("Error", errorHandler);
        wsClient.send(request);
    });
}

function formatUnix(unix) {
    if (!unix) return "—";
    return formatDateTime(unix * 1000);
}

/**
 * A convergence state or probe verdict token, as an operator-facing label.
 * The broker reports both as lowercase tokens; the page shows neither raw.
 */
function bridgeLabel(value) {
    switch (value) {
    case "healthy":
        return t("bridge.page.label.healthy");
    case "converging":
        return t("bridge.page.label.converging");
    case "held":
        return t("bridge.page.label.held");
    case "starting":
        return t("bridge.page.label.starting");
    case "pending":
        return t("bridge.page.label.pending");
    case "degraded":
        return t("bridge.page.label.degraded");
    case "down":
        return t("bridge.page.label.down");
    default:
        return t("bridge.page.label.unknown");
    }
}

/** A publish-outcome enum name, as an operator-facing label. */
function outcomeLabel(name) {
    switch (name) {
    case "Rejected":
        return t("bridge.page.outcome.rejected_label");
    case "Pending":
        return t("bridge.page.outcome.pending_label");
    case "Retrying":
        return t("bridge.page.outcome.retrying_label");
    default:
        return name;
    }
}

function stateBadge(state) {
    switch (state) {
    case "healthy":
        return "bg-success";
    case "converging":
        return "bg-warning text-dark";
    case "held":
        return "bg-danger";
    default:
        return "bg-secondary";
    }
}

function verdictBadge(verdict) {
    switch (verdict) {
    case "healthy":
        return "text-success";
    case "starting":
    case "pending":
        return "text-body-secondary";
    case "degraded":
        return "text-warning-emphasis";
    case "down":
        return "text-danger";
    default:
        return "text-body-secondary";
    }
}

function setBadge(id, label, cls) {
    const el = document.getElementById(id);
    if (!el) return;
    el.className = `badge ${cls}`;
    el.textContent = label;
}

function setText(id, text) {
    const el = document.getElementById(id);
    if (el) el.textContent = text;
}

function renderConvergence(snapshot) {
    const state = snapshot?.state || "unknown";
    setBadge("bridgePageState", bridgeLabel(state), stateBadge(state));
    setText("bridgePageConvergenceState", bridgeLabel(state));
    setText("bridgePageHeldCount", String((snapshot?.held || []).length));
    setText("bridgePageApplied", snapshot?.applied_generation
        ? String(snapshot.applied_generation).slice(0, 12)
        : "—");
    setText("bridgePageAppliedNote", snapshot?.last_applied_unix
        ? t("bridge.page.last_applied", {time: formatUnix(snapshot.last_applied_unix)})
        : t("bridge.page.applied_note"));
    setText("bridgePageConvergenceNote", snapshot?.held_since_unix
        ? t("bridge.page.held_since", {time: formatUnix(snapshot.held_since_unix)})
        : t("bridge.page.state_hint"));

    const container = document.getElementById("bridgePageConvergence");
    if (!container) return;
    const held = snapshot?.held || [];
    if (state === "held" && held.length > 0) {
        container.className = "lqos-config-note lqos-config-note-warning";
        container.innerHTML = `<div>${tHtml("bridge.page.holding", {count: held.length})}</div>
            <ul class="mb-0 mt-2">${held.map((token) => `<li>${token}</li>`).join("")}</ul>`;
    } else if (state === "converging") {
        container.className = "lqos-config-note lqos-config-note-info";
        container.textContent = t("bridge.page.applying");
    } else {
        container.className = "lqos-config-note lqos-config-note-info";
        container.textContent = t("bridge.page.converged");
    }
}

function renderHealth(health) {
    if (!health) {
        setText("bridgePageService", t("bridge.page.not_configured"));
        setText("bridgePageServiceNote", t("bridge.page.no_bridge"));
        return;
    }
    const verdict = health.verdict || "pending";
    setText("bridgePageService", bridgeLabel(verdict));
    const el = document.getElementById("bridgePageService");
    if (el) el.className = `bridge-stat-value ${verdictBadge(verdict)}`;
    setText(
        "bridgePageServiceNote",
        health.last_success_unix
            ? t("bridge.page.last_healthy_probe", {time: formatUnix(health.last_success_unix)})
            : tn("bridge.page.failed_probes", health.consecutive_failures ?? 0),
    );
}

// translatable: false — "Rejected", "Pending", "Retrying" are the broker's
// serde enum keys, compared here; the operator-facing labels are catalog keys
function describeState(state) {
    if (typeof state === "string") return outcomeLabel(state);
    if (state && typeof state === "object") {
        const key = Object.keys(state)[0];
        if (!key) return t("bridge.page.outcome.pending_label");
        const detail = state[key];
        if (key === "Rejected" && detail) return t("bridge.page.outcome.rejected", {reason: detail});
        if (key === "Pending" && detail) {
            return t("bridge.page.outcome.pending", {position: detail.seq_pos, total: detail.seq_len});
        }
        if (key === "Retrying" && detail) {
            return t("bridge.page.outcome.retrying", {attempt: detail.attempt, max: detail.max});
        }
        return outcomeLabel(key);
    }
    return "—";
}

function renderOutcomes(records) {
    const tbody = document.getElementById("bridgePageOutcomes");
    if (!tbody) return;
    const rows = records || [];
    if (rows.length === 0) {
        tbody.innerHTML = `<tr><td colspan="3" class="text-muted">${t("bridge.page.no_outcomes")}</td></tr>`;
        return;
    }
    tbody.innerHTML = rows.map((r) => {
        const gen = String(r.generation || "—").slice(0, 16);
        const state = describeState(r.state);
        const updated = formatUnix(r.updated_unix);
        return `<tr><td><code>${gen}</code></td><td>${state}</td><td>${updated}</td></tr>`;
    }).join("");
}

async function refresh() {
    try {
        const health = await sendWsRequest("BridgeHealthStatus", {BridgeHealthStatus: {}});
        renderHealth(health?.data);
    } catch (e) {
        setText("bridgePageService", t("bridge.page.unavailable"));
    }
    try {
        const convergence = await sendWsRequest("BridgeConvergence", {BridgeConvergence: {}});
        renderConvergence(convergence?.data);
    } catch (e) {
        setBadge("bridgePageState", "unknown", "bg-secondary");
    }
    try {
        const outcomes = await sendWsRequest("BridgePublishStatus", {BridgePublishStatus: {}});
        renderOutcomes(outcomes?.data);
    } catch (e) {
        // leave the placeholder row
    }
    setText("bridgePageUpdated", t("bridge.page.updated_at", {time: formatTime(Date.now(), {timeStyle: "medium"})}));
}

function bindReload() {
    if (reloadBound) return;
    const button = document.getElementById("bridgePageReload");
    if (!button) return;
    reloadBound = true;
    button.addEventListener("click", () => {
        setText("bridgePageReloadResult", t("bridge.page.resyncing"));
        const handler = (msg) => {
            wsClient.off("ReloadResult", handler);
            setText("bridgePageReloadResult", msg?.message || t("bridge.page.reload_requested"));
            setTimeout(refresh, 1500);
        };
        wsClient.on("ReloadResult", handler);
        wsClient.send({ReloadLibreQoS: {}});
    });
}

function init() {
    bindReload();
    refresh();
    setInterval(refresh, 5000);
}

if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
} else {
    init();
}
