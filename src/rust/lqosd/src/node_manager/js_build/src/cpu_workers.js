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
import {
    PHASE_COLORS,
    fleetOccupancy,
    normalizeWorkerRows,
    coreRowKind,
    retainLastGoodSections,
    stackedOccupancySegments,
} from "./worker_occupancy_model.mjs";
import {ONE_DECIMAL, formatNumber, formatTime} from "./i18n/format.js";
import {t, tAttr, tn} from "./i18n/index.js";

const REFRESH_MS = 5000;
let refreshTimer = null;

/**
 * The stacked phase bar for one utilization row: one span per phase with a
 * positive share, then the poll-excluded remainder. The page list and the
 * per-worker detail both render this bar, so it lives in one place.
 * @param {{busyPpm: number, phases: {name: string, label: string, fraction: number}[]}} util
 * @returns {string} The bar's spans.
 */
function occupancySegmentsHtml(util) {
    const stack = stackedOccupancySegments(util.busyPpm, util.phases);
    const segs = stack.segments
        .filter((seg) => seg.pct > 0)
        .map((seg) => `<span class="bg-${PHASE_COLORS[seg.name] ?? "secondary"}" style="display:inline-block;height:100%;width:${seg.pct.toFixed(3)}%" title="${tAttr("cpu_workers.segment_title", {label: seg.label, percent: formatNumber(seg.pct, ONE_DECIMAL)})}"></span>`)
        .join("");
    const idle = `<span class="bg-dark-subtle" style="display:inline-block;height:100%;width:${stack.idlePct.toFixed(3)}%" title="${tAttr("cpu_workers.idle_title", {percent: formatNumber(stack.idlePct, ONE_DECIMAL)})}"></span>`;
    return segs + idle;
}
let inFlight = false;
const lastSignatures = {
    state: null,
    cpuUsage: null,
    topology: null,
    workers: null,
    tree: null,
    moves: null,
    utilization: null,
};
let lastOwnershipView = null;
let lastCpuUsage = null;
let lastTopology = null;
let lastWorkers = null;
let lastUtilization = null;
let lastGoodData = null;
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
        return `<span class="cpu-worker-chip cpu-worker-chip-unknown">${t("cpu_workers.unowned")}</span>`;
    }
    return `<span class="cpu-worker-chip" style="background:${workerColor(workerId)}">worker ${workerId}</span>`;
}

function renderState(data) {
    const badge = document.getElementById("cpuWorkersState");
    if (badge) {
        badge.className = `badge ${stateBadgeClass(data.state)}`;
        badge.textContent = stateLabel(data.state);
        badge.setAttribute("aria-label", t("cpu_workers.state_aria", {state: stateLabel(data.state)}));
    }
    if (data.unavailableReasons.length) {
        html("cpuWorkersReasons", data.unavailableReasons
            .map((reason) => `<span class="badge bg-secondary me-1 mb-1">${escapeHtml(reasonText(reason))}</span>`)
            .join(""));
    } else if (data.state === "ok") {
        html("cpuWorkersReasons", "");
    } else {
        html("cpuWorkersReasons", `<span class="text-muted">${t("cpu_workers.no_reason")}</span>`);
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

function renderCpuUsage(usage, topology, workers, utilization = null) {
    lastCpuUsage = usage;
    lastTopology = topology;
    lastWorkers = workers;
    lastUtilization = utilization;
    const utilAvailable = utilization?.available === true;
    const utilRows = utilAvailable ? normalizeWorkerRows(utilization.rows) : [];
    const utilByWorker = new Map(utilRows.map((row) => [row.queueId, row]));
    const fleet = utilAvailable ? fleetOccupancy(utilization.rows) : null;
    const fleetEl = document.getElementById("cpuWorkersFleet");
    if (fleetEl) {
        if (fleet && fleet.count > 0) {
            fleetEl.className = fleet.warning
                ? "badge bg-warning text-dark"
                : "badge bg-secondary";
            fleetEl.textContent = t("cpu_workers.fleet_max", {
                label: t("cpu_workers.fleet_badge"),
                max: formatNumber(fleet.max_pct, ONE_DECIMAL),
                mean: formatNumber(fleet.mean_pct, ONE_DECIMAL),
                p95: formatNumber(fleet.p95_pct, ONE_DECIMAL),
            });
            fleetEl.title = t("cpu_workers.fleet_title");
        } else if (workers?.available) {
            // The affinity says these ARE DPDK workers; the utilization is
            // merely pending (the first bridge sample) — never "unavailable".
            fleetEl.className = "badge bg-secondary";
            fleetEl.textContent = t("cpu_workers.fleet_waiting", {label: t("cpu_workers.fleet_badge")});
        } else {
            fleetEl.className = "badge bg-secondary";
            fleetEl.textContent = t("cpu_workers.fleet_unavailable", {label: t("cpu_workers.fleet_badge")});
        }
    }
    text("cpuWorkersCpuCount", usage.available ? formatCount(usage.numCpus) : "—");
    const container = document.getElementById("cpuWorkersCpuList");
    if (!container) return;
    if (!usage.available) {
        container.innerHTML = `<div class="text-muted">${t("cpu_workers.usage_unavailable", {reason: escapeHtml(reasonText(usage.reason))})}</div>`;
        renderSelectedCpu();
        return;
    }
    if (!usage.rows.length) {
        container.innerHTML = `<div class="text-muted">${t("cpu_workers.no_cpus")}</div>`;
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
    container.setAttribute("aria-label", t("cpu_workers.per_cpu_aria"));
    container.innerHTML = usage.rows.map((row) => {
        const worker = row.cpuId !== null ? workersByLcore.get(row.cpuId) : null;
        const util = worker ? utilByWorker.get(worker.workerId) : null;
        const kind = coreRowKind(!!worker, !!util);
        const selected = row.cpuId === selectedCpuId ? " is-selected" : "";
        const shapingBadge = showShapingBadge && row.cpuId !== null && shaping.has(row.cpuId)
            ? `<span class="badge bg-info text-dark ms-2">shaping</span>`
            : "";
        if (kind === "worker") {
            // 74t171 rework: a DPDK worker core — the stacked phase bar with a
            // neutral idle/poll remainder; the value is the POLL-EXCLUDED busy.
            const value = t("cpu_workers.percent_value", {percent: formatNumber(util.busyPct, ONE_DECIMAL)});
            const aria = t("cpu_workers.row_aria_busy", {label: row.label, value});
            return `<button type="button" class="cpu-meter-row${selected}" data-cpu="${row.cpuId ?? ""}" aria-pressed="${row.cpuId === selectedCpuId}" aria-label="${escapeHtml(aria)}">
                <div class="cpu-meter-head">
                    <span><strong>${escapeHtml(row.label)}</strong>${shapingBadge}<span class="badge bg-secondary ms-2">${t("common.worker")}</span></span>
                    <span class="cpu-meter-value ${util.color === "warning" ? "text-warning fw-bold" : ""}">${escapeHtml(value)}</span>
                </div>
                <div class="cpu-meter-track" style="display:flex;overflow:hidden">${occupancySegmentsHtml(util)}</div>
            </button>`;
        }
        if (kind === "worker-pending") {
            // A DPDK worker lcore with the utilization not yet sampled: a
            // neutral waiting row, NEVER the OS/LB usage fallback.
            return `<button type="button" class="cpu-meter-row${selected}" data-cpu="${row.cpuId ?? ""}" aria-pressed="${row.cpuId === selectedCpuId}" aria-label="${escapeHtml(t("cpu_workers.row_aria_waiting", {label: row.label}))}">
                <div class="cpu-meter-head">
                    <span><strong>${escapeHtml(row.label)}</strong>${shapingBadge}<span class="badge bg-secondary ms-2">${t("common.worker")}</span></span>
                    <span class="cpu-meter-value text-muted">—</span>
                </div>
                <div class="cpu-meter-track"><div class="cpu-meter-bar bg-secondary" style="width:0%"></div></div>
            </button>`;
        }
        const percent = Number.isFinite(row.percent)
            ? Math.max(0, Math.min(100, row.percent))
            : null;
        const value = percent === null ? "—" : formatPercent(percent);
        const width = percent === null ? 0 : percent;
        const aria = percent === null
            ? t("cpu_workers.row_aria_unavailable", {label: row.label})
            : `${row.label}: ${value}`;
        return `<button type="button" class="cpu-meter-row${selected}" data-cpu="${row.cpuId ?? ""}" aria-pressed="${row.cpuId === selectedCpuId}" aria-label="${escapeHtml(aria)}">
            <div class="cpu-meter-head">
                <span><strong>${escapeHtml(row.label)}</strong>${shapingBadge}<span class="badge bg-light text-dark ms-2">OS/LB</span></span>
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
        title.textContent = t("cpu_workers.select_cpu");
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
        fact(t("common.worker"), worker ? String(worker.workerId) : "—"),
        fact("LCore", worker ? formatCount(worker.lcoreId) : "—"),
        fact("NUMA", worker ? formatCount(worker.numaNode) : "—"),
        fact("RX", worker ? formatQueueIds(worker.rxQueueIds) : "—"),
        fact("TX", worker ? formatCount(worker.txQueueId) : "—"),
        fact(t("common.table.circuits"), worker ? formatCount(worker.circuitCount) : "—"),
        fact(t("cpu_workers.sole_nodes"), worker ? formatCount(worker.solelyOwnedNodeCount) : "—"),
    ].join("")}</div>`;
    const utilRow = worker
        ? normalizeWorkerRows(lastUtilization?.available ? lastUtilization.rows : [])
            .find((candidate) => candidate.queueId === worker.workerId)
        : null;
    if (utilRow) {
        usage.innerHTML = `<div class="cpu-meter-head"><span>${t("cpu_workers.fleet_badge")}</span><span class="cpu-meter-value ${utilRow.color === "warning" ? "text-warning fw-bold" : ""}">${t("cpu_workers.percent_value", {percent: formatNumber(utilRow.busyPct, ONE_DECIMAL)})}</span></div>
        <div class="cpu-meter-track" style="height:1rem;display:flex;overflow:hidden">${occupancySegmentsHtml(utilRow)}</div>
        <div class="small text-muted mt-1">${t("cpu_workers.phases_label")} ${utilRow.phases.map((phase) => `<span class="d-inline-block rounded-circle bg-${PHASE_COLORS[phase.name] ?? "secondary"}" style="width:8px;height:8px" title="${phase.label}"></span> ${phase.label} ${t("cpu_workers.percent_value", {percent: formatNumber(phase.fraction * 100, ONE_DECIMAL)})}`).join(" · ")}</div>
        <div class="small text-muted">${t("cpu_workers.phase_totals", {
            cumulative: formatNumber(utilRow.busyPpmCumulative / 10_000, ONE_DECIMAL),
            backlog: utilRow.backlogMean,
            laps: utilRow.busyLaps,
            rate: utilRow.lapsPerS,
        })}</div>`;
    } else if (worker) {
        // A DPDK worker lcore whose utilization is still pending: never show
        // the old CPU-usage detail (nor flip to it on a row click).
        usage.innerHTML = `<div class="cpu-meter-head"><span>${t("cpu_workers.fleet_badge")}</span><span class="cpu-meter-value text-muted">${t("cpu_workers.waiting_sample")}</span></div>
        <div class="cpu-meter-track" style="height:1rem"><div class="cpu-meter-bar bg-secondary" style="width:0%"></div></div>`;
    } else {
        usage.innerHTML = `<div class="cpu-meter-head"><span>${t("cpu_workers.usage")}</span><span class="cpu-meter-value">${escapeHtml(value)}</span></div>
        <div class="cpu-meter-track" style="height:1rem"><div class="cpu-meter-bar ${cpuTone(percent)}" style="width:${percent === null ? 0 : percent}%"></div></div>`;
    }

    if (!worker) {
        workload.innerHTML = `<div class="text-muted small p-2">${t("cpu_workers.no_owned_circuits")}</div>`;
        return;
    }
    if (!lastOwnershipView?.available) {
        workload.innerHTML = `<div class="text-muted small p-2">${t("cpu_workers.ownership_unavailable", {reason: escapeHtml(reasonText(lastOwnershipView?.reason))})}</div>`;
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
        ? `<div class="text-muted small px-2 pt-2">${t("cpu_workers.owned_summary", {count: circuits.length, sites: siteRows.length})}</div>
           <table class="lqos-table lqos-table-compact align-middle mb-0">
            <thead><tr><th>${t("cpu_workers.site_ap")}</th><th>${t("common.table.circuits")}</th></tr></thead>
            <tbody>${siteRows.map(([siteName, count]) => {
                const sharded = shardedBySite.get(siteName);
                const badge = sharded
                    ? ` <span class="badge bg-info text-dark">${tn("cpu_workers.sharded_badge", sharded)}</span>`
                    : "";
                return `<tr><td>${escapeHtml(siteName)}${badge}</td><td>${count}</td></tr>`;
            }).join("")}</tbody>
           </table>`
        : `<div class="text-muted small p-2">${t("cpu_workers.no_worker_circuits")}</div>`;
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
    // 74t171 work order (1): never flip the Cores rows to the usage fallback
    // on a transient workers_unavailable when a prior good payload exists.
    const retained = retainLastGoodSections(lastGoodData, data);
    const view = retained.data;
    if (!retained.stale) {
        lastGoodData = data;
    }
    const signatures = sectionSignatures(raw);
    if (signatures.state !== lastSignatures.state) {
        renderState(data);
        lastSignatures.state = signatures.state;
    }
    if (signatures.cpuUsage !== lastSignatures.cpuUsage) {
        renderCpuUsage(view.cpuUsage, view.topology, view.workers, view.utilization);
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
    if (signatures.utilization !== lastSignatures.utilization) {
        // The utilization feeds the SAME Cores rows (the stacked bars).
        renderCpuUsage(view.cpuUsage, view.topology, view.workers, view.utilization);
        lastSignatures.utilization = signatures.utilization;
    }
    text("cpuWorkersUpdated", retained.stale
        ? t("cpu_workers.snapshot_stale")
        : (data.generatedAtUnix
            ? t("cpu_workers.snapshot_time", {time: formatTime(data.generatedAtUnix * 1000)})
            : t("cpu_workers.no_snapshot_yet")));
}

function renderUnavailable(reason) {
    const data = normalizeCpuWorkers(null);
    data.state = "unavailable";
    data.unavailableReasons = [reason];
    for (const key of Object.keys(lastSignatures)) lastSignatures[key] = null;
    renderState(data);
    renderCpuUsage(data.cpuUsage, data.topology, data.workers, data.utilization);
    renderTopology(data.topology);
    renderOwnership(data.ownership);
    text("cpuWorkersUpdated", t("cpu_workers.no_snapshot"));
}

function renderProtocolError(reason) {
    const data = normalizeCpuWorkers(null);
    data.state = "protocol_error";
    data.unavailableReasons = [reason];
    for (const key of Object.keys(lastSignatures)) lastSignatures[key] = null;
    renderState(data);
    renderCpuUsage(data.cpuUsage, data.topology, data.workers, data.utilization);
    renderTopology(data.topology);
    renderOwnership(data.ownership);
    text("cpuWorkersUpdated", t("cpu_workers.protocol_error"));
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
            renderProtocolError(t("cpu_workers.unexpected_payload"));
            return;
        }
        const signatures = sectionSignatures(payload);
        if (signaturesEqual(signatures, lastSignatures)) {
            text("cpuWorkersUpdated", t("cpu_workers.checked_unchanged", {time: formatTime(Date.now())}));
            return;
        }
        render(normalizeCpuWorkers(payload), payload);
    } catch (error) {
        // 74t171 work order (1): a refresh failure must NOT wipe the last
        // good layout/signatures; keep the last snapshot with a stale marker.
        if (lastGoodData) {
            text("cpuWorkersUpdated", t("cpu_workers.refresh_failed", {reason: error.message || error}));
            return;
        }
        for (const key of Object.keys(lastSignatures)) lastSignatures[key] = null;
        renderUnavailable(t("cpu_workers.local_api_unavailable", {reason: error.message || error}));
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
            // translatable: false — "button[data-cpu]" is a DOM selector, not a label
            const row = event.target.closest("button[data-cpu]");
            if (!row || row.dataset.cpu === "") return;
            const cpuId = Number.parseInt(row.dataset.cpu, 10);
            if (!Number.isInteger(cpuId)) return;
            selectedCpuId = cpuId;
            renderCpuUsage(lastCpuUsage, lastTopology, lastWorkers, lastUtilization);
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
