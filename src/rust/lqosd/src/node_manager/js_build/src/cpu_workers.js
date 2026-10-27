// CPU & Workers page (lqos-74t.118.3). Same-origin local-api surface:
// GET /local-api/cpu-workers. Every absent source is rendered as an explicit
// unavailable/unsupported state; no zero is fabricated for a missing value.
import {
    countShardedSites,
    formatCount,
    shardedSiteWorkerCounts,
    formatPercent,
    formatQueueIds,
    isAssembledCpuWorkersPayload,
    normalizeCpuWorkers,
    ownershipView,
    reasonText,
    sectionSignatures,
    stateBadgeClass,
    stateLabel,
    topologySummary,
    workerColor,
} from "./cpu_workers_model.mjs";

const REFRESH_MS = 5000;
let refreshTimer = null;
let inFlight = false;
const lastSignatures = {
    state: null,
    cpuUsage: null,
    topology: null,
    workers: null,
    tree: null,
    moves: null,
};
let lastOwnershipView = null;
let lastCpuUsage = null;
let lastTopology = null;
let lastWorkers = null;
let selectedCpuId = null;

function text(id, value) {
    const element = document.getElementById(id);
    if (element) element.textContent = value;
}

function html(id, value) {
    const element = document.getElementById(id);
    if (element) element.innerHTML = value;
}

function escapeHtml(value) {
    return String(value)
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll("\"", "&quot;")
        .replaceAll("'", "&#39;");
}

function workerChip(workerId) {
    if (!Number.isInteger(workerId)) {
        return `<span class="cpu-worker-chip cpu-worker-chip-unknown">unowned</span>`;
    }
    return `<span class="cpu-worker-chip" style="background:${workerColor(workerId)}">worker ${workerId}</span>`;
}

function renderState(data) {
    const badge = document.getElementById("cpuWorkersState");
    if (badge) {
        badge.className = `badge ${stateBadgeClass(data.state)}`;
        badge.textContent = stateLabel(data.state);
        badge.setAttribute("aria-label", `CPU and Workers state: ${stateLabel(data.state)}`);
    }
    if (data.unavailableReasons.length) {
        html("cpuWorkersReasons", data.unavailableReasons
            .map((reason) => `<span class="badge bg-secondary me-1 mb-1">${escapeHtml(reasonText(reason))}</span>`)
            .join(""));
    } else if (data.state === "ok") {
        html("cpuWorkersReasons", "");
    } else {
        html("cpuWorkersReasons", `<span class="text-muted">No unavailable reason was reported.</span>`);
    }
}

function cpuTone(percent) {
    if (!Number.isFinite(percent)) return "cpu-meter-bar-secondary";
    if (percent >= 90) return "cpu-meter-bar-danger";
    if (percent >= 75) return "cpu-meter-bar-warning";
    return "cpu-meter-bar-success";
}

function workerByLcore(workers) {
    const map = new Map();
    for (const worker of workers?.available ? workers.rows : []) {
        if (Number.isInteger(worker.lcoreId)) map.set(worker.lcoreId, worker);
    }
    return map;
}

function renderCpuUsage(usage, topology, workers) {
    lastCpuUsage = usage;
    lastTopology = topology;
    lastWorkers = workers;
    text("cpuWorkersCpuCount", usage.available ? formatCount(usage.numCpus) : "—");
    const container = document.getElementById("cpuWorkersCpuList");
    if (!container) return;
    if (!usage.available) {
        container.innerHTML = `<div class="text-muted">CPU usage unavailable: ${escapeHtml(reasonText(usage.reason))}</div>`;
        renderSelectedCpu();
        return;
    }
    if (!usage.rows.length) {
        container.innerHTML = `<div class="text-muted">No CPUs sampled.</div>`;
        renderSelectedCpu();
        return;
    }
    const shaping = new Set(Array.isArray(topology?.shaping) ? topology.shaping : []);
    const workersByLcore = workerByLcore(workers);
    if (
        selectedCpuId === null
        || !usage.rows.some((row) => row.cpuId === selectedCpuId)
    ) {
        const firstWorkerCpu = usage.rows.find(
            (row) => row.cpuId !== null && workersByLcore.has(row.cpuId),
        );
        selectedCpuId = firstWorkerCpu?.cpuId ?? usage.rows[0].cpuId;
    }
    const showShapingBadge = shaping.size > 0 && shaping.size < usage.rows.length;
    container.setAttribute("role", "list");
    container.setAttribute("aria-label", "Per-CPU usage");
    container.innerHTML = usage.rows.map((row) => {
        const percent = Number.isFinite(row.percent)
            ? Math.max(0, Math.min(100, row.percent))
            : null;
        const value = percent === null ? "—" : formatPercent(percent);
        const width = percent === null ? 0 : percent;
        const aria = percent === null
            ? `${row.label}: usage unavailable`
            : `${row.label}: ${value}`;
        const shapingBadge = showShapingBadge && row.cpuId !== null && shaping.has(row.cpuId)
            ? `<span class="badge bg-info text-dark ms-2">shaping</span>`
            : "";
        const selected = row.cpuId === selectedCpuId ? " is-selected" : "";
        return `<button type="button" class="cpu-meter-row${selected}" data-cpu="${row.cpuId ?? ""}" aria-pressed="${row.cpuId === selectedCpuId}" aria-label="${escapeHtml(aria)}">
            <div class="cpu-meter-head">
                <span><strong>${escapeHtml(row.label)}</strong>${shapingBadge}</span>
                <span class="cpu-meter-value">${escapeHtml(value)}</span>
            </div>
            <div class="cpu-meter-track">
                <div class="cpu-meter-bar ${cpuTone(percent)}" style="width:${width}%"></div>
            </div>
        </button>`;
    }).join("");
    renderSelectedCpu();
}

function renderSelectedCpu() {
    const title = document.getElementById("cpuSelectedTitle");
    const badges = document.getElementById("cpuSelectedBadges");
    const kpis = document.getElementById("cpuSelectedKpis");
    const usage = document.getElementById("cpuSelectedUsage");
    const workload = document.getElementById("cpuSelectedWorkload");
    if (!title || !badges || !kpis || !usage || !workload) return;

    const row = lastCpuUsage?.available
        ? lastCpuUsage.rows.find((candidate) => candidate.cpuId === selectedCpuId)
        : null;
    if (!row) {
        title.textContent = "Select a CPU";
        badges.innerHTML = "";
        kpis.innerHTML = "";
        usage.innerHTML = "";
        workload.innerHTML = "";
        return;
    }
    const worker = row.cpuId !== null ? workerByLcore(lastWorkers).get(row.cpuId) : null;
    const shaping = new Set(Array.isArray(lastTopology?.shaping) ? lastTopology.shaping : []);
    const isShaping = row.cpuId !== null && shaping.has(row.cpuId);
    const percent = Number.isFinite(row.percent)
        ? Math.max(0, Math.min(100, row.percent))
        : null;
    const value = percent === null ? "unavailable" : formatPercent(percent);
    title.textContent = row.label;
    badges.innerHTML = [
        worker ? workerChip(worker.workerId) : "",
        isShaping ? `<span class="badge bg-info text-dark">shaping</span>` : "",
    ].filter(Boolean).join(" ");

    const fact = (label, value) => `<div><span class="cpu-detail-fact-label">${label}</span><strong>${escapeHtml(value)}</strong></div>`;
    kpis.innerHTML = `<div class="cpu-detail-facts">${[
        fact("Worker", worker ? String(worker.workerId) : "—"),
        fact("LCore", worker ? formatCount(worker.lcoreId) : "—"),
        fact("NUMA", worker ? formatCount(worker.numaNode) : "—"),
        fact("RX", worker ? formatQueueIds(worker.rxQueueIds) : "—"),
        fact("TX", worker ? formatCount(worker.txQueueId) : "—"),
        fact("Circuits", worker ? formatCount(worker.circuitCount) : "—"),
        fact("Sole nodes", worker ? formatCount(worker.solelyOwnedNodeCount) : "—"),
    ].join("")}</div>`;
    usage.innerHTML = `<div class="cpu-meter-head"><span>Usage</span><span class="cpu-meter-value">${escapeHtml(value)}</span></div>
        <div class="cpu-meter-track" style="height:1rem"><div class="cpu-meter-bar ${cpuTone(percent)}" style="width:${percent === null ? 0 : percent}%"></div></div>`;

    if (!worker) {
        workload.innerHTML = `<div class="text-muted small p-2">No worker-owned circuits for this CPU.</div>`;
        return;
    }
    if (!lastOwnershipView?.available) {
        workload.innerHTML = `<div class="text-muted small p-2">Circuit ownership unavailable: ${escapeHtml(reasonText(lastOwnershipView?.reason))}</div>`;
        return;
    }
    const circuits = lastOwnershipView.circuits
        .filter((circuit) => circuit.workerId === worker.workerId);
    const bySite = new Map();
    for (const circuit of circuits) {
        bySite.set(circuit.siteName, (bySite.get(circuit.siteName) || 0) + 1);
    }
    const shardedBySite = lastOwnershipView.available
        ? shardedSiteWorkerCounts(lastOwnershipView.nodes)
        : new Map();
    const siteRows = [...bySite.entries()]
        .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
    workload.innerHTML = siteRows.length
        ? `<div class="text-muted small px-2 pt-2">${circuits.length} circuit(s) across ${siteRows.length} site/AP(s)</div>
           <table class="lqos-table lqos-table-compact align-middle mb-0">
            <thead><tr><th>Site / AP</th><th>Circuits</th></tr></thead>
            <tbody>${siteRows.map(([siteName, count]) => {
                const sharded = shardedBySite.get(siteName);
                const badge = sharded
                    ? ` <span class="badge bg-info text-dark">sharded · ${sharded} workers</span>`
                    : "";
                return `<tr><td>${escapeHtml(siteName)}${badge}</td><td>${count}</td></tr>`;
            }).join("")}</tbody>
           </table>`
        : `<div class="text-muted small p-2">No owned circuits for this worker.</div>`;
}

function renderTopology(topology) {
    const shaping = topology?.available && Array.isArray(topology.shaping)
        ? String(topology.shaping.length)
        : "—";
    text("cpuWorkersShapingCount", shaping);
    const element = document.getElementById("cpuWorkersShapingCount");
    if (element) {
        element.title = topology?.available
            ? topologySummary(topology)
            : reasonText(topology?.reason);
    }
}

function renderOwnership(ownership) {
    const view = ownershipView(ownership);
    lastOwnershipView = view;
    if (!view.available) {
        text("cpuWorkersUnowned", "—");
        text("cpuWorkersShardedCount", "—");
    } else {
        text("cpuWorkersUnowned", formatCount(ownership.unownedCircuits));
        text("cpuWorkersShardedCount", formatCount(countShardedSites(view.nodes)));
    }
    renderSelectedCpu();
}

function signaturesEqual(left, right) {
    return Object.keys(left).every((key) => left[key] === right[key]);
}

function render(data, raw) {
    const signatures = sectionSignatures(raw);
    if (signatures.state !== lastSignatures.state) {
        renderState(data);
        lastSignatures.state = signatures.state;
    }
    if (signatures.cpuUsage !== lastSignatures.cpuUsage) {
        renderCpuUsage(data.cpuUsage, data.topology, data.workers);
        lastSignatures.cpuUsage = signatures.cpuUsage;
    }
    if (signatures.topology !== lastSignatures.topology) {
        renderTopology(data.topology);
        lastSignatures.topology = signatures.topology;
    }
    if (signatures.workers !== lastSignatures.workers) {
        lastSignatures.workers = signatures.workers;
    }
    if (signatures.tree !== lastSignatures.tree) {
        renderOwnership(data.ownership);
        lastSignatures.tree = signatures.tree;
    }
    if (signatures.moves !== lastSignatures.moves) {
        lastSignatures.moves = signatures.moves;
    }
    text("cpuWorkersUpdated", data.generatedAtUnix
        ? `Snapshot ${new Date(data.generatedAtUnix * 1000).toLocaleTimeString()}`
        : "No snapshot yet");
}

function renderUnavailable(reason) {
    const data = normalizeCpuWorkers(null);
    data.state = "unavailable";
    data.unavailableReasons = [reason];
    for (const key of Object.keys(lastSignatures)) lastSignatures[key] = null;
    renderState(data);
    renderCpuUsage(data.cpuUsage, data.topology, data.workers);
    renderTopology(data.topology);
    renderOwnership(data.ownership);
    text("cpuWorkersUpdated", "No snapshot");
}

function renderProtocolError(reason) {
    const data = normalizeCpuWorkers(null);
    data.state = "protocol_error";
    data.unavailableReasons = [reason];
    for (const key of Object.keys(lastSignatures)) lastSignatures[key] = null;
    renderState(data);
    renderCpuUsage(data.cpuUsage, data.topology, data.workers);
    renderTopology(data.topology);
    renderOwnership(data.ownership);
    text("cpuWorkersUpdated", "Protocol error");
}

async function refresh() {
    if (inFlight) return;
    inFlight = true;
    try {
        const response = await fetch("/local-api/cpu-workers", {
            method: "GET",
            credentials: "same-origin",
            headers: {Accept: "application/json"},
            cache: "no-store",
        });
        if (!response.ok) {
            throw new Error(`HTTP ${response.status}`);
        }
        const payload = await response.json();
        if (!isAssembledCpuWorkersPayload(payload)) {
            renderProtocolError("unexpected /local-api/cpu-workers shape");
            return;
        }
        const signatures = sectionSignatures(payload);
        if (signaturesEqual(signatures, lastSignatures)) {
            text("cpuWorkersUpdated", `Checked ${new Date().toLocaleTimeString()} — unchanged`);
            return;
        }
        render(normalizeCpuWorkers(payload), payload);
    } catch (error) {
        for (const key of Object.keys(lastSignatures)) lastSignatures[key] = null;
        renderUnavailable(`local_api_unavailable: ${error.message || error}`);
    } finally {
        inFlight = false;
    }
}

function schedule() {
    if (refreshTimer !== null) {
        window.clearTimeout(refreshTimer);
        refreshTimer = null;
    }
    if (document.hidden) return;
    refreshTimer = window.setTimeout(async () => {
        await refresh();
        schedule();
    }, REFRESH_MS);
}

function init() {
    const button = document.getElementById("cpuWorkersRefresh");
    if (button) {
        button.addEventListener("click", () => {
            refresh();
        });
    }
    const cpuList = document.getElementById("cpuWorkersCpuList");
    if (cpuList) {
        cpuList.addEventListener("click", (event) => {
            const row = event.target.closest("button[data-cpu]");
            if (!row || row.dataset.cpu === "") return;
            const cpuId = Number.parseInt(row.dataset.cpu, 10);
            if (!Number.isInteger(cpuId)) return;
            selectedCpuId = cpuId;
            renderCpuUsage(lastCpuUsage, lastTopology, lastWorkers);
        });
    }
    document.addEventListener("visibilitychange", () => {
        if (document.hidden) {
            if (refreshTimer !== null) {
                window.clearTimeout(refreshTimer);
                refreshTimer = null;
            }
            return;
        }
        refresh().finally(schedule);
    });
    refresh().finally(schedule);
}

if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
} else {
    init();
}
