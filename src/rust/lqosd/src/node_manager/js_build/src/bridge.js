// L4 (`docs/control_plane_structural_convergence_card.md` §5): the Bridge page.
// Shows control-plane health + convergence, the HELD op in operator language,
// recent publish outcomes, and the reload action. No bridge restart action —
// the control plane quarantines and converges automatically.
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
let reloadBound = false;

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
    return new Date(unix * 1000).toLocaleString();
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
    setBadge("bridgePageState", state, stateBadge(state));
    setText("bridgePageConvergenceState", state);
    setText("bridgePageHeldCount", String((snapshot?.held || []).length));
    setText("bridgePageApplied", snapshot?.applied_generation
        ? String(snapshot.applied_generation).slice(0, 12)
        : "—");
    setText("bridgePageAppliedNote", snapshot?.last_applied_unix
        ? `Last applied ${formatUnix(snapshot.last_applied_unix)}`
        : "Last applied world");
    setText("bridgePageConvergenceNote", snapshot?.held_since_unix
        ? `Held since ${formatUnix(snapshot.held_since_unix)}`
        : "healthy / converging / held");

    const container = document.getElementById("bridgePageConvergence");
    if (!container) return;
    const held = snapshot?.held || [];
    if (state === "held" && held.length > 0) {
        container.className = "lqos-config-note lqos-config-note-warning";
        container.innerHTML = `<div><strong>The control plane is holding ${held.length} change(s)</strong> — everything else keeps applying; no restart is needed.</div>
            <ul class="mb-0 mt-2">${held.map((t) => `<li>${t}</li>`).join("")}</ul>`;
    } else if (state === "converging") {
        container.className = "lqos-config-note lqos-config-note-info";
        container.textContent = "Applying pending changes…";
    } else {
        container.className = "lqos-config-note lqos-config-note-info";
        container.textContent = "Converged — the applied world matches the desired configuration.";
    }
}

function renderHealth(health) {
    if (!health) {
        setText("bridgePageService", "not configured");
        setText("bridgePageServiceNote", "No DPDK bridge configured");
        return;
    }
    const verdict = health.verdict || "pending";
    setText("bridgePageService", verdict);
    const el = document.getElementById("bridgePageService");
    if (el) el.className = `bridge-stat-value ${verdictBadge(verdict)}`;
    setText(
        "bridgePageServiceNote",
        health.last_success_unix
            ? `Last healthy probe ${formatUnix(health.last_success_unix)}`
            : `${health.consecutive_failures ?? 0} consecutive failed probe(s)`,
    );
}

function describeState(state) {
    if (typeof state === "string") return state;
    if (state && typeof state === "object") {
        const key = Object.keys(state)[0];
        if (!key) return "pending";
        const detail = state[key];
        if (key === "Rejected" && detail) return `rejected (${detail})`;
        if (key === "Pending" && detail) return `pending ${detail.seq_pos}/${detail.seq_len}`;
        if (key === "Retrying" && detail) return `retrying ${detail.attempt}/${detail.max}`;
        return key;
    }
    return "—";
}

function renderOutcomes(records) {
    const tbody = document.getElementById("bridgePageOutcomes");
    if (!tbody) return;
    const rows = records || [];
    if (rows.length === 0) {
        tbody.innerHTML = `<tr><td colspan="3" class="text-muted">No publish outcomes yet.</td></tr>`;
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
        setText("bridgePageService", "unavailable");
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
    setText("bridgePageUpdated", `Updated ${new Date().toLocaleTimeString()}`);
}

function bindReload() {
    if (reloadBound) return;
    const button = document.getElementById("bridgePageReload");
    if (!button) return;
    reloadBound = true;
    button.addEventListener("click", () => {
        setText("bridgePageReloadResult", "Re-syncing and applying…");
        const handler = (msg) => {
            wsClient.off("ReloadResult", handler);
            setText("bridgePageReloadResult", msg?.message || "Reload requested.");
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
