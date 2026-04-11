import { clearDiv, enableTooltips, simpleRow, theading } from "./helpers/builders";
import { redactCell } from "./helpers/redact";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client } from "./pubsub/ws";

const wsClient = get_ws_client();

let state = {
    selectedCpu: null,
    detailTab: "nodes",
    direction: "down",
    page: 1,
    pageSize: 10,
    showExcludedCores: false,
    plannerEnabled: null,
    excludeEfficiencyCores: null,
    runtimeSnapshot: null,
    liveCpuUsage: [],
    overviewScrollTop: 0,
};

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function requestConfig() {
    return new Promise((resolve) => {
        let done = false;
        const finish = (cfg) => {
            if (done) return;
            done = true;
            resolve(cfg);
        };
        const timeout = setTimeout(() => finish(null), 5000);
        listenOnce("GetConfig", (msg) => {
            clearTimeout(timeout);
            finish(msg && msg.data ? (msg.data.config || null) : null);
        });
        wsClient.send({ GetConfig: {} });
    });
}

function requestRuntimeSnapshot() {
    return new Promise((resolve) => {
        const timeout = setTimeout(() => resolve(null), 10000);
        listenOnce("CpuAffinityRuntimeSnapshot", (msg) => {
            clearTimeout(timeout);
            resolve(msg && msg.data ? msg.data : null);
        });
        wsClient.send({ CpuAffinityRuntimeSnapshot: {} });
    });
}

function fmtMbps(x) {
    if (x === null || x === undefined) return "-";
    let s = Number(x).toFixed(2);
    s = s.replace(/\.00$/, "").replace(/(\.\d*?[1-9])0+$/, "$1");
    return `${s} Mbps`;
}

function fmtBps(bytesPerSecond) {
    const bitsPerSecond = Math.max(0, toNumber(bytesPerSecond, 0) * 8);
    return `${scaleNumber(bitsPerSecond, bitsPerSecond >= 1_000_000 ? 1 : 0)}bps`;
}

function nodeUtilizationPercent(node) {
    const downMaxMbps = Math.max(0, toNumber(node?.effective_max_mbps?.[0], 0));
    const upMaxMbps = Math.max(0, toNumber(node?.effective_max_mbps?.[1], 0));
    const downBps = Math.max(0, toNumber(node?.current_throughput_bps?.[0], 0) * 8);
    const upBps = Math.max(0, toNumber(node?.current_throughput_bps?.[1], 0) * 8);

    const downUtilization = downMaxMbps > 0 ? (downBps / (downMaxMbps * 1_000_000)) * 100 : null;
    const upUtilization = upMaxMbps > 0 ? (upBps / (upMaxMbps * 1_000_000)) * 100 : null;
    if (!Number.isFinite(downUtilization) && !Number.isFinite(upUtilization)) {
        return null;
    }
    return Math.max(0, Math.max(downUtilization ?? 0, upUtilization ?? 0));
}

function fmtUtilizationPercent(node) {
    const utilization = nodeUtilizationPercent(node);
    if (!Number.isFinite(utilization)) {
        return "-";
    }
    return `${Math.round(utilization)}%`;
}

function cpuUsageValue(core) {
    const live = state.liveCpuUsage?.[core.cpu];
    if (Number.isFinite(live)) {
        return live;
    }
    const fallback = toNumber(core.live_usage_pct, NaN);
    return Number.isFinite(fallback) ? fallback : null;
}

function usageToneClass(usage) {
    if (!Number.isFinite(usage)) return "bg-secondary";
    if (usage >= 90) return "bg-danger";
    if (usage >= 75) return "bg-warning";
    return "bg-success";
}

function assignmentBadge(node) {
    const reason = (node?.assignment_reason || "").toString();
    if (reason === "runtime_virtualized_hidden") {
        return { label: "Runtime Virtualized", cls: "bg-warning text-dark" };
    }
    if (reason === "inherited_from_virtualized_ancestor") {
        return { label: "Inherited", cls: "bg-info text-dark" };
    }
    if (reason === "planned") {
        return { label: "Planned", cls: "bg-success" };
    }
    return { label: "Unknown", cls: "bg-secondary" };
}

function topLevelNodesForCore(core) {
    const nodes = Array.isArray(core?.nodes) ? core.nodes : [];
    return [...nodes]
        .filter((node) => !!node?.is_cpu_root)
        .sort((a, b) => {
            const circuitDelta = toNumber(b?.subtree_circuit_count, 0) - toNumber(a?.subtree_circuit_count, 0);
            if (circuitDelta !== 0) {
                return circuitDelta;
            }
            const throughputA = toNumber(a?.current_throughput_bps?.[0], 0) + toNumber(a?.current_throughput_bps?.[1], 0);
            const throughputB = toNumber(b?.current_throughput_bps?.[0], 0) + toNumber(b?.current_throughput_bps?.[1], 0);
            if (throughputB !== throughputA) {
                return throughputB - throughputA;
            }
            return String(a?.name || "").localeCompare(String(b?.name || ""));
        });
}

function getCores() {
    const cores = Array.isArray(state.runtimeSnapshot?.cores) ? [...state.runtimeSnapshot.cores] : [];
    cores.sort((a, b) => toNumber(a.cpu, 0) - toNumber(b.cpu, 0));
    return cores;
}

function getShapingCpuSet() {
    const shaping = Array.isArray(state.runtimeSnapshot?.shaping_cpus)
        ? state.runtimeSnapshot.shaping_cpus
        : [];
    return new Set(shaping.map((cpu) => toNumber(cpu, -1)).filter((cpu) => cpu >= 0));
}

function getExcludedCpuSet() {
    const excluded = Array.isArray(state.runtimeSnapshot?.excluded_cpus)
        ? state.runtimeSnapshot.excluded_cpus
        : [];
    return new Set(excluded.map((cpu) => toNumber(cpu, -1)).filter((cpu) => cpu >= 0));
}

function hasExcludedCpuFilter() {
    return getExcludedCpuSet().size > 0;
}

function hasUsableShapingCpuFilter() {
    const shaping = getShapingCpuSet();
    return shaping.size > 0 && shaping.size < getCores().length;
}

function getInferredIncludedCpuSet() {
    if (!state.excludeEfficiencyCores) {
        return new Set();
    }
    const included = getCores()
        .filter((core) =>
            toNumber(core.planned_circuit_count, 0) > 0
            || toNumber(core.planned_max_mbps, 0) > 0
            || toNumber(core.effective_node_count, 0) > 0
            || toNumber(core.effective_circuit_count, 0) > 0
        )
        .map((core) => toNumber(core.cpu, -1))
        .filter((cpu) => cpu >= 0);
    return new Set(included);
}

function hasFallbackIncludedCpuFilter() {
    const included = getInferredIncludedCpuSet();
    return included.size > 0 && included.size < getCores().length;
}

function getIncludedCpuSet() {
    if (hasExcludedCpuFilter()) {
        const excluded = getExcludedCpuSet();
        return new Set(
            getCores()
                .map((core) => toNumber(core.cpu, -1))
                .filter((cpu) => cpu >= 0 && !excluded.has(cpu))
        );
    }
    if (hasUsableShapingCpuFilter()) {
        return getShapingCpuSet();
    }
    if (hasFallbackIncludedCpuFilter()) {
        return getInferredIncludedCpuSet();
    }
    return new Set();
}

function getVisibleCores() {
    const cores = getCores();
    if (state.showExcludedCores) {
        return cores;
    }
    const included = getIncludedCpuSet();
    if (included.size > 0) {
        return cores.filter((core) => included.has(toNumber(core.cpu, -1)));
    }
    return cores;
}

function getSelectedCore() {
    return getVisibleCores().find((core) => toNumber(core.cpu, -1) === toNumber(state.selectedCpu, -2)) || null;
}

function selectDefaultCpu() {
    const cores = getVisibleCores();
    if (cores.length === 0) {
        state.selectedCpu = null;
        return;
    }
    const stillExists = cores.some((core) => toNumber(core.cpu, -1) === toNumber(state.selectedCpu, -2));
    if (stillExists) {
        return;
    }
    state.selectedCpu = cores[0].cpu;
}

function renderBinpackBadge() {
    const badge = document.getElementById("binpackBadge");
    if (!badge) return;
    const enabled = !!state.plannerEnabled;
    badge.className = `cpu-affinity-chip ${enabled ? "is-positive" : "is-muted"}`;
    badge.textContent = enabled ? "Binpacking: Enabled" : "Binpacking: Disabled";
}

function applyOverviewRowLiveMetrics(row, core) {
    const usage = cpuUsageValue(core);
    const chip = row.querySelector(".cpu-affinity-overview-usage");
    const bar = row.querySelector(".cpu-affinity-overview-progress-bar");
    if (chip) {
        chip.className = `cpu-affinity-overview-usage ${usageToneClass(usage)}`;
        chip.textContent = Number.isFinite(usage) ? `${Math.round(usage)}%` : "N/A";
    }
    if (bar) {
        bar.className = `cpu-affinity-overview-progress-bar ${usageToneClass(usage)}`;
        bar.style.width = `${Math.max(0, Math.min(100, toNumber(usage, 0)))}%`;
    }
}

function updateOverviewLiveMetrics() {
    const overview = document.getElementById("cpuOverview");
    if (!overview) return;
    const coreMap = new Map(getVisibleCores().map((core) => [String(toNumber(core.cpu, -1)), core]));
    overview.querySelectorAll(".cpu-affinity-overview-row[data-cpu]").forEach((row) => {
        const core = coreMap.get(row.dataset.cpu);
        if (!core) return;
        applyOverviewRowLiveMetrics(row, core);
    });
}

function renderOverview() {
    const target = document.getElementById("cpuOverview");
    const existingList = target?.querySelector(".cpu-affinity-overview-list");
    if (existingList) {
        state.overviewScrollTop = existingList.scrollTop;
    }
    clearDiv(target);
    renderBinpackBadge();

    const cores = getVisibleCores();
    if (cores.length === 0) {
        target.innerHTML = (hasExcludedCpuFilter() || hasUsableShapingCpuFilter() || hasFallbackIncludedCpuFilter())
            ? '<p class="text-muted">No shaping CPUs are visible with the current filter.</p>'
            : '<p class="text-muted">No runtime CPU assignment data found. Run LibreQoS to generate queuingStructure.json and live tree state.</p>';
        return;
    }

    const panel = document.createElement("div");
    panel.className = "cpu-affinity-card h-100";

    const body = document.createElement("div");
    body.className = "cpu-affinity-card-body";

    const header = document.createElement("div");
    header.className = "cpu-affinity-card-header";

    const headerText = document.createElement("div");
    const shapingCount = Array.isArray(state.runtimeSnapshot?.shaping_cpus) ? state.runtimeSnapshot.shaping_cpus.length : 0;
    const excludedCount = Array.isArray(state.runtimeSnapshot?.excluded_cpus) ? state.runtimeSnapshot.excluded_cpus.length : 0;
    const hasHybridSplit = !!state.runtimeSnapshot?.has_hybrid_split;
    const inferredIncludedCount = getInferredIncludedCpuSet().size;
    const overviewSubtitle = (hasExcludedCpuFilter() || hasUsableShapingCpuFilter() || hasFallbackIncludedCpuFilter()) && !state.showExcludedCores
        ? `Showing shaping CPUs only${hasHybridSplit && excludedCount > 0 ? ` (${shapingCount} shaping cores, ${excludedCount} excluded host cores hidden)` : ""}.`
        : hasHybridSplit && excludedCount > 0
            ? `Showing all host cores, including ${excludedCount} excluded non-shaping core${excludedCount === 1 ? "" : "s"}.`
            : state.showExcludedCores && hasFallbackIncludedCpuFilter()
                ? `Showing all host cores (${inferredIncludedCount} included cores inferred from current queue ownership).`
                : hasFallbackIncludedCpuFilter()
                    ? `Showing included cores only (${inferredIncludedCount} included cores inferred from current queue ownership).`
                    : "Choose a core to inspect live ownership, promoted runtime branches, and planner drift.";
    headerText.innerHTML = `
        <h3>CPU Core List</h3>
        <p>${overviewSubtitle}</p>
    `;
    header.appendChild(headerText);

    const generatedAt = toNumber(state.runtimeSnapshot?.generated_at_unix_ms, Date.now());
    const status = document.createElement("div");
    status.className = "cpu-affinity-card-meta";
    status.textContent = `Updated ${new Date(generatedAt).toLocaleTimeString()}`;
    header.appendChild(status);
    body.appendChild(header);

    if (!window.hasInsight) {
        const promo = document.createElement("div");
        promo.className = "cpu-affinity-inline-note mb-3";
        promo.innerHTML = 'Enable <a href="lts_trial.html">Insight</a> to improve CPU planning with historical data.';
        body.appendChild(promo);
    }

    const list = document.createElement("div");
    list.className = "cpu-affinity-overview-list";
    list.addEventListener("scroll", () => {
        state.overviewScrollTop = list.scrollTop;
    });

    cores.forEach((core) => {
        const row = document.createElement("button");
        row.type = "button";
        row.className = `cpu-affinity-overview-row ${toNumber(state.selectedCpu, -1) === toNumber(core.cpu, -2) ? "is-selected" : ""}`;
        row.dataset.cpu = String(toNumber(core.cpu, -1));
        row.onclick = () => {
            state.selectedCpu = core.cpu;
            state.page = 1;
            renderOverview();
            renderSelectedCore();
            if (state.detailTab === "circuits") {
                fetchCircuits();
            }
        };

        const topRow = document.createElement("div");
        topRow.className = "d-flex justify-content-between align-items-start gap-3 mb-2";

        const titleWrap = document.createElement("div");
        const title = document.createElement("div");
        title.className = "fw-semibold";
        title.textContent = `CPU ${core.cpu}`;
        titleWrap.appendChild(title);
        const cpuId = toNumber(core.cpu, -1);
        const included = getIncludedCpuSet();
        const isExcluded = included.size > 0 && !included.has(cpuId);
        topRow.appendChild(titleWrap);

        const summary = document.createElement("div");
        summary.className = "cpu-affinity-overview-summary";
        summary.textContent = `${core.effective_node_count.toLocaleString()} nodes · ${core.effective_circuit_count.toLocaleString()} circuits`;
        topRow.appendChild(summary);

        const usage = cpuUsageValue(core);
        row.appendChild(topRow);

        if (isExcluded) {
            const subtitle = document.createElement("div");
            subtitle.className = "text-muted small mb-2";
            subtitle.textContent = "Excluded from shaping";
            row.appendChild(subtitle);
        }

        const progress = document.createElement("div");
        progress.className = "cpu-affinity-overview-progress";
        const bar = document.createElement("div");
        bar.className = `cpu-affinity-overview-progress-bar ${usageToneClass(usage)}`;
        bar.style.width = `${Math.max(0, Math.min(100, toNumber(usage, 0)))}%`;
        progress.appendChild(bar);
        row.appendChild(progress);
        list.appendChild(row);
    });

    body.appendChild(list);
    panel.appendChild(body);
    target.appendChild(panel);
    list.scrollTop = state.overviewScrollTop;
}

function renderDetailsHeader() {
    const label = document.getElementById("currentCpuLabel");
    if (state.selectedCpu === null || state.selectedCpu === undefined) {
        label.textContent = "";
        return;
    }
    label.textContent = ` – CPU ${state.selectedCpu}`;
}

function renderSelectedSummary() {
    const target = document.getElementById("cpuSelectedSummary");
    clearDiv(target);
    const core = getSelectedCore();
    if (!core) {
        target.innerHTML = '<span class="text-muted small">Select a core from the list.</span>';
        return;
    }

    const row = document.createElement("div");
    row.className = "cpu-affinity-kpi-grid";
    const cards = [
        {
            title: "Live Load",
            value: Number.isFinite(cpuUsageValue(core)) ? `${Math.round(cpuUsageValue(core))}%` : "N/A",
        },
        {
            title: "Assigned Nodes",
            value: toNumber(core.effective_node_count, 0).toLocaleString(),
        },
        {
            title: "Assigned Circuits",
            value: toNumber(core.effective_circuit_count, 0).toLocaleString(),
        },
    ];

    cards.forEach((entry) => {
        const card = document.createElement("div");
        card.className = "cpu-affinity-kpi";
        const label = document.createElement("div");
        label.className = "cpu-affinity-kpi-label";
        label.textContent = entry.title;
        const value = document.createElement("div");
        value.className = "cpu-affinity-kpi-value";
        value.textContent = entry.value;
        card.appendChild(label);
        card.appendChild(value);
        row.appendChild(card);
    });
    target.appendChild(row);
}

function renderNodesTable(core) {
    const target = document.getElementById("cpuDetailsContent");
    clearDiv(target);

    const nodes = topLevelNodesForCore(core);
    if (nodes.length === 0) {
        target.innerHTML = '<p class="text-secondary">No top-level owned branches are associated with this CPU.</p>';
        return;
    }

    const note = document.createElement("div");
    note.className = "cpu-affinity-inline-note mb-2";
    note.textContent = "Showing the branches directly attached to this CPU's effective HTB subtree after runtime virtualization and promotion.";
    target.appendChild(note);

    const tableWrap = document.createElement("div");
    tableWrap.className = "lqos-table-wrap";
    const table = document.createElement("table");
    table.className = "lqos-table lqos-table-compact";
    const thead = document.createElement("thead");
    thead.appendChild(theading("Node"));
    thead.appendChild(theading("Assignment"));
    thead.appendChild(theading("Utilization"));
    thead.appendChild(theading("Sub-Nodes"));
    thead.appendChild(theading("Circuits"));
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    nodes.forEach((node) => {
        const tr = document.createElement("tr");
        if (node.runtime_virtualized || node.assignment_reason !== "planned") {
            tr.classList.add("cpu-affinity-row-change");
        }

        const nameCell = document.createElement("td");
        nameCell.textContent = node.name || "";
        redactCell(nameCell);
        tr.appendChild(nameCell);

        const assignmentCell = document.createElement("td");
        const badgeMeta = assignmentBadge(node);
        const badge = document.createElement("span");
        badge.className = `badge ${badgeMeta.cls}`;
        badge.textContent = badgeMeta.label;
        assignmentCell.appendChild(badge);
        tr.appendChild(assignmentCell);

        tr.appendChild(simpleRow(fmtUtilizationPercent(node)));
        tr.appendChild(simpleRow(toNumber(node.subtree_node_count, 0).toLocaleString()));
        tr.appendChild(simpleRow(toNumber(node.subtree_circuit_count, 0).toLocaleString()));
        tbody.appendChild(tr);
    });

    table.appendChild(tbody);
    tableWrap.appendChild(table);
    target.appendChild(tableWrap);
}

function renderCircuits(page) {
    const target = document.getElementById("cpuDetailsContent");
    clearDiv(target);

    if (!page || !Array.isArray(page.items)) {
        target.innerHTML = '<p class="text-secondary">No circuits found for this CPU.</p>';
        return;
    }

    const smallNote = document.createElement("div");
    smallNote.className = "text-muted small mb-1";
    smallNote.innerText = `Total: ${page.total.toLocaleString()} planned downlink circuits`;
    target.appendChild(smallNote);

    const tableWrap = document.createElement("div");
    tableWrap.classList.add("lqos-table-wrap");
    const table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-compact");
    const thead = document.createElement("thead");
    thead.appendChild(theading("Circuit Name"));
    thead.appendChild(theading("Parent"));
    thead.appendChild(theading("ClassID"));
    thead.appendChild(theading("Weight"));
    thead.appendChild(theading("Max (Mbps)"));
    thead.appendChild(theading("IPs"));
    table.appendChild(thead);
    const tbody = document.createElement("tbody");

    page.items.forEach((c) => {
        const tr = document.createElement("tr");
        if (c.ignored || (c.weight !== undefined && c.weight <= 0)) {
            tr.classList.add("text-muted");
        }
        const nameCell = document.createElement("td");
        if (c.circuit_id && c.circuit_name) {
            const a = document.createElement("a");
            a.href = `circuit.html?id=${encodeURIComponent(c.circuit_id)}`;
            a.textContent = c.circuit_name;
            redactCell(a);
            nameCell.appendChild(a);
        } else {
            nameCell.textContent = c.circuit_name || "";
            redactCell(nameCell);
        }
        tr.appendChild(nameCell);
        tr.appendChild(simpleRow(c.parent_node || "", true));
        tr.appendChild(simpleRow(c.classid || ""));
        const weightCell = document.createElement("td");
        if (c.ignored || (c.weight !== undefined && c.weight <= 0)) {
            const badge = document.createElement("span");
            badge.className = "cpu-affinity-chip is-muted";
            badge.textContent = "ignored";
            weightCell.appendChild(badge);
        } else {
            weightCell.textContent = (c.weight && c.weight > 0) ? c.weight.toLocaleString() : "-";
        }
        tr.appendChild(weightCell);
        tr.appendChild(simpleRow(fmtMbps(c.max_mbps)));
        tr.appendChild(simpleRow(String(c.ip_count || 0)));
        tbody.appendChild(tr);
    });

    table.appendChild(tbody);
    tableWrap.appendChild(table);
    target.appendChild(tableWrap);
}

function renderPagerVisibility() {
    const pager = document.getElementById("cpuDetailsPager");
    pager.style.display = state.detailTab === "circuits" && state.selectedCpu !== null ? "flex" : "none";
}

function renderActiveTab() {
    const tabButtons = [
        ["btnTabNodes", "nodes"],
        ["btnTabCircuits", "circuits"],
    ];
    tabButtons.forEach(([id, tab]) => {
        const btn = document.getElementById(id);
        if (!btn) return;
        btn.className = `cpu-affinity-tab ${state.detailTab === tab ? "is-active" : ""}`;
    });
}

function renderSelectedCore() {
    renderDetailsHeader();
    renderSelectedSummary();
    renderActiveTab();
    renderPagerVisibility();

    const core = getSelectedCore();
    if (!core) {
        const target = document.getElementById("cpuDetailsContent");
        target.innerHTML = '<i class="fa fa-info-circle"></i> <span class="text-secondary">Select a CPU from the list.</span>';
        return;
    }

    if (state.detailTab === "circuits") {
        fetchCircuits();
        return;
    }
    renderNodesTable(core);
    enableTooltips();
}

function fetchCircuits() {
    const target = document.getElementById("cpuDetailsContent");
    target.innerHTML = '<i class="fa fa-spinner fa-spin"></i> Loading circuits...';
    const cpu = state.selectedCpu;
    if (cpu === null || cpu === undefined) {
        target.innerHTML = '<i class="fa fa-info-circle"></i> <span class="text-secondary">Select a CPU from the list.</span>';
        return;
    }
    const timeout = setTimeout(() => {
        target.innerHTML = '<div class="text-danger">Failed to load circuits: timeout</div>';
    }, 10000);
    listenOnce("CpuAffinityCircuits", (msg) => {
        clearTimeout(timeout);
        if (!msg || !msg.data) {
            target.innerHTML = '<div class="text-danger">Failed to load circuits.</div>';
            return;
        }
        renderCircuits(msg.data);
    });
    wsClient.send({
        CpuAffinityCircuits: {
            cpu,
            direction: state.direction,
            page: state.page,
            page_size: state.pageSize,
        },
    });
}

async function refreshAll() {
    const overview = document.getElementById("cpuOverview");
    const showExcludedCores = document.getElementById("showExcludedCores");
    if (overview && !state.runtimeSnapshot) {
        overview.innerHTML = '<i class="fa fa-spinner fa-spin"></i> Loading core assignment...';
    }

    const configPromise = requestConfig().then((cfg) => {
        state.plannerEnabled = !!(cfg && cfg.queues && cfg.queues.use_binpacking);
        state.excludeEfficiencyCores = !!(cfg && cfg.exclude_efficiency_cores);
    });
    const snapshot = await requestRuntimeSnapshot();
    await configPromise;
    state.runtimeSnapshot = snapshot;
    if (showExcludedCores) {
        showExcludedCores.disabled = !hasExcludedCpuFilter() && !hasUsableShapingCpuFilter() && !hasFallbackIncludedCpuFilter();
        if (showExcludedCores.disabled) {
            state.showExcludedCores = false;
            showExcludedCores.checked = false;
        } else {
            showExcludedCores.checked = !!state.showExcludedCores;
        }
    }
    selectDefaultCpu();
    renderOverview();
    renderSelectedCore();
    enableTooltips();
}

function initControls() {
    const pageSizeInput = document.getElementById("pageSize");
    const btnPrev = document.getElementById("btnPrev");
    const btnNext = document.getElementById("btnNext");
    const btnRefresh = document.getElementById("btnRefresh");
    const btnTabNodes = document.getElementById("btnTabNodes");
    const btnTabCircuits = document.getElementById("btnTabCircuits");
    const showExcludedCores = document.getElementById("showExcludedCores");

    pageSizeInput.onchange = () => {
        let v = parseInt(pageSizeInput.value || "10", 10);
        if (!Number.isFinite(v) || v < 10) v = 10;
        if (v > 1000) v = 1000;
        state.pageSize = v;
        state.page = 1;
        if (state.detailTab === "circuits") {
            fetchCircuits();
        }
    };
    btnPrev.onclick = () => {
        if (state.page > 1) {
            state.page -= 1;
            if (state.detailTab === "circuits") fetchCircuits();
        }
    };
    btnNext.onclick = () => {
        state.page += 1;
        if (state.detailTab === "circuits") fetchCircuits();
    };
    btnRefresh.onclick = () => {
        refreshAll();
    };
    if (showExcludedCores) {
        showExcludedCores.checked = !!state.showExcludedCores;
        showExcludedCores.disabled = !hasExcludedCpuFilter() && !hasUsableShapingCpuFilter() && !hasFallbackIncludedCpuFilter();
        showExcludedCores.onchange = () => {
            state.showExcludedCores = !!showExcludedCores.checked;
            state.overviewScrollTop = 0;
            selectDefaultCpu();
            renderOverview();
            renderSelectedCore();
            if (state.detailTab === "circuits") {
                fetchCircuits();
            }
        };
    }
    btnTabNodes.onclick = () => {
        state.detailTab = "nodes";
        renderSelectedCore();
    };
    btnTabCircuits.onclick = () => {
        state.detailTab = "circuits";
        state.page = 1;
        renderSelectedCore();
    };
}

function initLiveCpuSubscription() {
    wsClient.subscribe(["Cpu"]);
    wsClient.on("Cpu", (msg) => {
        state.liveCpuUsage = Array.isArray(msg?.data) ? msg.data.map((value) => toNumber(value, 0)) : [];
        updateOverviewLiveMetrics();
        renderSelectedSummary();
    });
}

initControls();
renderDetailsHeader();
renderActiveTab();
renderPagerVisibility();
initLiveCpuSubscription();
refreshAll();
