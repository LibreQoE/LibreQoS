import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
let probeState = null;

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

function escapeHtml(text) {
    return String(text ?? "")
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll('"', "&quot;")
        .replaceAll("'", "&#39;");
}

function setStatus(label, badgeClass) {
    const status = document.getElementById("topologyProbesStatus");
    if (!status) {
        return;
    }
    status.className = `badge ${badgeClass}`;
    status.textContent = label;
}

function formatUnix(unix) {
    if (!unix) {
        return "—";
    }
    return new Date(unix * 1000).toLocaleString();
}

function statusBadge(status) {
    switch (status) {
    case "suppressed":
        return "bg-danger-subtle text-danger";
    case "probe_unavailable":
        return "bg-warning-subtle text-warning-emphasis";
    case "disabled":
        return "bg-secondary-subtle text-secondary";
    case "healthy":
        return "bg-success-subtle text-success";
    default:
        return "bg-light text-body-secondary";
    }
}

function statusLabel(status) {
    switch (status) {
    case "suppressed":
        return "Suppressed";
    case "probe_unavailable":
        return "Probe Unavailable";
    case "disabled":
        return "Disabled";
    case "healthy":
        return "Healthy";
    default:
        return status || "Unknown";
    }
}

function endpointSummary(entry) {
    if (!Array.isArray(entry.endpoint_status) || entry.endpoint_status.length === 0) {
        return "—";
    }
    return entry.endpoint_status.map((endpoint) => {
        const reachable = endpoint.reachable
            ? "<span class='badge bg-success-subtle text-success'>Reachable</span>"
            : "<span class='badge bg-danger-subtle text-danger'>Down</span>";
        return `<div>${reachable} <span class="text-muted">${escapeHtml(endpoint.ip)}</span></div>`;
    }).join("");
}

function matchesSearch(entry, term) {
    if (!term) {
        return true;
    }
    const haystack = [
        entry.child_node_name,
        entry.child_node_id,
        entry.parent_node_name,
        entry.parent_node_id,
        entry.attachment_name,
        entry.attachment_id,
        entry.attachment_pair_id,
        entry.local_probe_ip,
        entry.remote_probe_ip,
        entry.reason,
    ]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();
    return haystack.includes(term);
}

function filteredEntries() {
    const entries = probeState?.entries || [];
    const search = document.getElementById("topologyProbesSearch")?.value.trim().toLowerCase() || "";
    const statusFilter = document.getElementById("topologyProbesStatusFilter")?.value || "all";
    const enabledFilter = document.getElementById("topologyProbesEnabledFilter")?.value || "enabled";
    return entries.filter((entry) => {
        if (!matchesSearch(entry, search)) {
            return false;
        }
        if (statusFilter !== "all" && entry.status !== statusFilter) {
            return false;
        }
        if (enabledFilter === "enabled" && !entry.enabled) {
            return false;
        }
        if (enabledFilter === "disabled" && entry.enabled) {
            return false;
        }
        return true;
    });
}

function renderStats() {
    const container = document.getElementById("topologyProbesStats");
    if (!container) {
        return;
    }
    const entries = probeState?.entries || [];
    const enabled = entries.filter((entry) => entry.enabled).length;
    const healthy = entries.filter((entry) => entry.status === "healthy").length;
    const suppressed = entries.filter((entry) => entry.status === "suppressed").length;
    const unavailable = entries.filter((entry) => entry.status === "probe_unavailable").length;

    container.innerHTML = [
        {
            label: "Enabled",
            value: enabled,
            note: "Probe pairs in service",
        },
        {
            label: "Healthy",
            value: healthy,
            note: "Passing both endpoints",
        },
        {
            label: "Suppressed",
            value: suppressed,
            note: "Currently held down",
        },
        {
            label: "Unavailable",
            value: unavailable,
            note: "Missing usable probe inputs",
        },
    ].map((card) => `
        <div class="topology-probes-stat">
            <div class="topology-probes-stat-label">${escapeHtml(card.label)}</div>
            <div class="topology-probes-stat-value">${escapeHtml(String(card.value))}</div>
            <div class="topology-probes-stat-note">${escapeHtml(card.note)}</div>
        </div>
    `).join("");
}

function renderTable() {
    const table = document.getElementById("topologyProbesTable");
    const summary = document.getElementById("topologyProbesSummary");
    if (!table || !summary) {
        return;
    }
    const entries = filteredEntries();
    const total = probeState?.entries?.length || 0;
    summary.textContent = `${entries.length} of ${total} probe pair${total === 1 ? "" : "s"}`;
    renderStats();

    if (!probeState) {
        table.innerHTML = '<tr><td colspan="8" class="text-muted">Loading topology probe state…</td></tr>';
        return;
    }

    if (entries.length === 0) {
        table.innerHTML = '<tr><td colspan="8" class="text-muted">No probe pairs match the current filters.</td></tr>';
        return;
    }

    table.innerHTML = entries.map((entry) => {
        const topologyHref = entry.child_node_id
            ? `/topology_manager.html?node_id=${encodeURIComponent(entry.child_node_id)}`
            : "/topology_manager.html";
        const healthBits = [
            `<div>Misses <strong>${entry.consecutive_misses ?? 0}</strong> / Successes <strong>${entry.consecutive_successes ?? 0}</strong></div>`,
            `<div class="text-muted">Last success ${escapeHtml(formatUnix(entry.last_success_unix))}</div>`,
            `<div class="text-muted">Last failure ${escapeHtml(formatUnix(entry.last_failure_unix))}</div>`,
        ];
        if (entry.suppressed_until_unix) {
            healthBits.push(`<div class="text-muted">Suppressed until ${escapeHtml(formatUnix(entry.suppressed_until_unix))}</div>`);
        }
        return `
            <tr>
                <td>
                    <div class="fw-semibold">${escapeHtml(entry.child_node_name || "—")}</div>
                    <div class="small text-muted">${escapeHtml(entry.child_node_id || "—")}</div>
                </td>
                <td>
                    <div class="fw-semibold">${escapeHtml(entry.parent_node_name || "—")}</div>
                    <div class="small text-muted">${escapeHtml(entry.parent_node_id || "—")}</div>
                </td>
                <td>
                    <div class="fw-semibold">${escapeHtml(entry.attachment_name || "—")}</div>
                    <div class="small text-muted">${escapeHtml(entry.attachment_id || entry.attachment_pair_id || "—")}</div>
                    <div class="small text-muted">${escapeHtml(entry.attachment_pair_id || "—")}</div>
                </td>
                <td>
                    <div>${escapeHtml(entry.local_probe_ip || "—")}</div>
                    <div class="text-muted">${escapeHtml(entry.remote_probe_ip || "—")}</div>
                </td>
                <td>
                    <div><span class="badge ${statusBadge(entry.status)}">${escapeHtml(statusLabel(entry.status))}</span></div>
                    <div class="small text-muted mt-1">${entry.enabled ? "Enabled" : "Disabled"} · ${entry.probeable ? "Probeable" : "Not probeable"}</div>
                    <div class="small text-muted mt-1">${escapeHtml(entry.reason || "—")}</div>
                </td>
                <td class="small">
                    ${healthBits.join("")}
                </td>
                <td class="small">
                    ${endpointSummary(entry)}
                </td>
                <td>
                    <a class="btn btn-sm btn-outline-primary" href="${topologyHref}">
                        <i class="fa fa-diagram-project"></i> Open in Topology
                    </a>
                </td>
            </tr>
        `;
    }).join("");
}

function updateUpdatedLabel() {
    const updated = document.getElementById("topologyProbesUpdated");
    if (!updated) {
        return;
    }
    updated.textContent = probeState?.generated_unix
        ? `Updated ${new Date(probeState.generated_unix * 1000).toLocaleString()}`
        : "No runtime probe snapshot found";
}

async function loadPage() {
    setStatus("Loading…", "bg-secondary");
    try {
        const response = await sendWsRequest("GetTopologyProbesState", {GetTopologyProbesState: {}});
        probeState = response.data || {entries: []};
        updateUpdatedLabel();
        renderTable();
        setStatus("Loaded", "bg-success");
    } catch (error) {
        probeState = {entries: []};
        updateUpdatedLabel();
        renderTable();
        setStatus("Error", "bg-danger");
        const summary = document.getElementById("topologyProbesSummary");
        if (summary) {
            summary.textContent = error?.message || "Unable to load topology probe state";
        }
    }
}

document.getElementById("topologyProbesSearch")?.addEventListener("input", renderTable);
document.getElementById("topologyProbesStatusFilter")?.addEventListener("change", renderTable);
document.getElementById("topologyProbesEnabledFilter")?.addEventListener("change", renderTable);

loadPage();
window.setInterval(loadPage, 5000);
