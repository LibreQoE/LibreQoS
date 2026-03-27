import { clearDiv, enableTooltips, simpleRow, theading } from "./helpers/builders";
import { scaleNumber, toNumber } from "./lq_js_common/helpers/scaling";
import { get_ws_client } from "./pubsub/ws";

const wsClient = get_ws_client();

let state = {
    selectedCpu: null,
    detailTab: "nodes",
    direction: "down",
    page: 1,
    pageSize: 50,
    plannerEnabled: null,
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
            finish(msg && msg.data ? msg.data : null);
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

function fmtPair(pair) {
    const down = Array.isArray(pair) ? pair[0] : 0;
    const up = Array.isArray(pair) ? pair[1] : 0;
    return `↓ ${fmtBps(down)} · ↑ ${fmtBps(up)}`;
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

function nodeTypeLabel(node) {
    const raw = (node?.node_type || "").toString().trim();
    if (!raw) return "Unknown";
    return raw.toUpperCase();
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

function getSelectedCore() {
    return getCores().find((core) => toNumber(core.cpu, -1) === toNumber(state.selectedCpu, -2)) || null;
}

function selectDefaultCpu() {
    const cores = getCores();
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
    badge.className = `badge ${enabled ? "bg-success" : "bg-secondary"}`;
    badge.textContent = enabled ? "Binpacking: Enabled" : "Binpacking: Disabled";
}

function renderOverview() {
    const target = document.getElementById("cpuOverview");
    const existingList = target?.querySelector(".cpu-affinity-overview-list");
    if (existingList) {
        state.overviewScrollTop = existingList.scrollTop;
    }
    clearDiv(target);
    renderBinpackBadge();

    const cores = getCores();
    if (cores.length === 0) {
        target.innerHTML = '<p class="text-muted">No runtime CPU assignment data found. Run LibreQoS to generate queuingStructure.json and live tree state.</p>';
        return;
    }

    const panel = document.createElement("div");
    panel.className = "card h-100";

    const body = document.createElement("div");
    body.className = "card-body";

    const header = document.createElement("div");
    header.className = "d-flex flex-wrap justify-content-between align-items-start gap-2 mb-3";

    const headerText = document.createElement("div");
    headerText.innerHTML = `
        <div class="fw-semibold">CPU Core List</div>
        <div class="text-muted small">Choose a core to inspect its live ownership and runtime branch assignment.</div>
    `;
    header.appendChild(headerText);

    const generatedAt = toNumber(state.runtimeSnapshot?.generated_at_unix_ms, Date.now());
    const status = document.createElement("div");
    status.className = "text-muted small";
    status.textContent = `Updated ${new Date(generatedAt).toLocaleTimeString()}`;
    header.appendChild(status);
    body.appendChild(header);

    if (!window.hasInsight) {
        const promo = document.createElement("div");
        promo.className = "text-muted small mb-3";
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
        topRow.className = "d-flex justify-content-between align-items-start gap-2 mb-2";

        const titleWrap = document.createElement("div");
        const title = document.createElement("div");
        title.className = "fw-semibold";
        title.textContent = `CPU ${core.cpu}`;
        titleWrap.appendChild(title);
        const subtitle = document.createElement("div");
        subtitle.className = "text-muted small";
        subtitle.textContent = `${core.effective_node_count.toLocaleString()} nodes · ${core.effective_circuit_count.toLocaleString()} circuits`;
        titleWrap.appendChild(subtitle);
        topRow.appendChild(titleWrap);

        const badgeWrap = document.createElement("div");
        const usageChip = document.createElement("span");
        const usage = cpuUsageValue(core);
        usageChip.className = `badge ${usageToneClass(usage)} me-1`;
        usageChip.textContent = Number.isFinite(usage) ? `${Math.round(usage)}%` : "N/A";
        badgeWrap.appendChild(usageChip);
        if (toNumber(core.runtime_changed_count, 0) > 0) {
            const badge = document.createElement("span");
            badge.className = "badge bg-warning text-dark";
            badge.title = `${core.runtime_changed_count.toLocaleString()} nodes differ from their planned runtime placement.`;
            badge.textContent = `${core.runtime_changed_count} changed`;
            badgeWrap.appendChild(badge);
        }
        topRow.appendChild(badgeWrap);
        row.appendChild(topRow);

        const progress = document.createElement("div");
        progress.className = "progress mb-2";
        const bar = document.createElement("div");
        bar.className = `progress-bar ${usageToneClass(usage)}`;
        bar.style.width = `${Math.max(0, Math.min(100, toNumber(usage, 0)))}%`;
        progress.appendChild(bar);
        row.appendChild(progress);

        const metrics = document.createElement("div");
        metrics.className = "small text-muted mb-2";
        metrics.innerHTML = `
            <div class="d-flex flex-wrap gap-3">
                <span>Planned ${toNumber(core.planned_circuit_count, 0).toLocaleString()} circuits</span>
                <span>Sites/APs ${toNumber(core.effective_site_count, 0).toLocaleString()}/${toNumber(core.effective_ap_count, 0).toLocaleString()}</span>
                <span>Planned max ${fmtMbps(core.planned_max_mbps)}</span>
            </div>
        `;
        row.appendChild(metrics);

        const rootsPreview = document.createElement("div");
        rootsPreview.className = "small";
        const roots = topLevelNodesForCore(core);
        const preview = roots.slice(0, 2);
        if (preview.length === 0) {
            const empty = document.createElement("div");
            empty.className = "text-secondary";
            empty.textContent = "No top-level branches";
            rootsPreview.appendChild(empty);
        } else {
            const label = document.createElement("div");
            label.className = "text-muted mb-1";
            label.textContent = "Top-level branches";
            rootsPreview.appendChild(label);
            preview.forEach((node) => {
                const rowEl = document.createElement("div");
                rowEl.className = "d-flex justify-content-between gap-2";
                const name = document.createElement("span");
                name.className = "text-truncate";
                name.textContent = node.name || "Unnamed node";
                const throughput = document.createElement("span");
                throughput.className = "text-muted";
                throughput.textContent = fmtPair(node.current_throughput_bps);
                rowEl.appendChild(name);
                rowEl.appendChild(throughput);
                rootsPreview.appendChild(rowEl);
            });
        }
        row.appendChild(rootsPreview);
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
    row.className = "row g-2";
    const cards = [
        {
            title: "Live Load",
            value: Number.isFinite(cpuUsageValue(core)) ? `${Math.round(cpuUsageValue(core))}%` : "N/A",
            tone: usageToneClass(cpuUsageValue(core)),
        },
        {
            title: "Assigned Nodes",
            value: toNumber(core.effective_node_count, 0).toLocaleString(),
            tone: "bg-primary",
        },
        {
            title: "Assigned Circuits",
            value: toNumber(core.effective_circuit_count, 0).toLocaleString(),
            tone: "bg-info",
        },
        {
            title: "Runtime Changes",
            value: toNumber(core.runtime_changed_count, 0).toLocaleString(),
            tone: toNumber(core.runtime_changed_count, 0) > 0 ? "bg-warning" : "bg-secondary",
        },
    ];

    cards.forEach((entry) => {
        const col = document.createElement("div");
        col.className = "col-12 col-md-6 col-xl-3";
        const card = document.createElement("div");
        card.className = "card h-100";
        const body = document.createElement("div");
        body.className = "card-body py-2";
        const label = document.createElement("div");
        label.className = "text-muted small";
        label.textContent = entry.title;
        const value = document.createElement("div");
        value.className = "fw-semibold";
        value.textContent = entry.value;
        const badge = document.createElement("span");
        badge.className = `badge ${entry.tone}`;
        badge.textContent = " ";
        badge.style.width = "0.75rem";
        body.appendChild(label);
        body.appendChild(value);
        body.appendChild(badge);
        card.appendChild(body);
        col.appendChild(card);
        row.appendChild(col);
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
    note.className = "text-muted small mb-2";
    note.textContent = "Showing only top-level owned branches directly attached to this CPU's effective HTB subtree. Use Changes or Circuits for deeper detail.";
    target.appendChild(note);

    const tableWrap = document.createElement("div");
    tableWrap.className = "lqos-table-wrap";
    const table = document.createElement("table");
    table.className = "lqos-table lqos-table-compact";
    const thead = document.createElement("thead");
    thead.appendChild(theading("Node"));
    thead.appendChild(theading("Type"));
    thead.appendChild(theading("Planned"));
    thead.appendChild(theading("Effective"));
    thead.appendChild(theading("Assignment"));
    thead.appendChild(theading("Throughput"));
    thead.appendChild(theading("Subtree Nodes"));
    thead.appendChild(theading("Subtree Circuits"));
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    nodes.forEach((node) => {
        const tr = document.createElement("tr");
        if (node.runtime_virtualized || node.assignment_reason !== "planned") {
            tr.classList.add("table-warning");
        }

        const nameCell = document.createElement("td");
        nameCell.textContent = node.name || "";
        tr.appendChild(nameCell);
        tr.appendChild(simpleRow(nodeTypeLabel(node)));
        tr.appendChild(simpleRow(node.planned_cpu === null || node.planned_cpu === undefined ? "-" : `CPU ${node.planned_cpu}`));
        tr.appendChild(simpleRow(node.effective_cpu === null || node.effective_cpu === undefined ? "-" : `CPU ${node.effective_cpu}`));

        const assignmentCell = document.createElement("td");
        const badgeMeta = assignmentBadge(node);
        const badge = document.createElement("span");
        badge.className = `badge ${badgeMeta.cls}`;
        badge.textContent = badgeMeta.label;
        assignmentCell.appendChild(badge);
        tr.appendChild(assignmentCell);

        tr.appendChild(simpleRow(fmtPair(node.current_throughput_bps)));
        tr.appendChild(simpleRow(toNumber(node.subtree_node_count, 0).toLocaleString()));
        tr.appendChild(simpleRow(toNumber(node.subtree_circuit_count, 0).toLocaleString()));
        tbody.appendChild(tr);
    });

    table.appendChild(tbody);
    tableWrap.appendChild(table);
    target.appendChild(tableWrap);
}

function renderChangesTable(core) {
    const target = document.getElementById("cpuDetailsContent");
    clearDiv(target);
    const nodes = (Array.isArray(core?.nodes) ? core.nodes : []).filter((node) =>
        node.runtime_virtualized
        || (node.assignment_reason || "") !== "planned"
        || toNumber(node.planned_cpu, -1) !== toNumber(node.effective_cpu, -1)
    );

    if (nodes.length === 0) {
        target.innerHTML = '<p class="text-secondary">This CPU has no runtime differences from planned placement.</p>';
        return;
    }

    const tableWrap = document.createElement("div");
    tableWrap.className = "lqos-table-wrap";
    const table = document.createElement("table");
    table.className = "lqos-table lqos-table-compact";
    const thead = document.createElement("thead");
    thead.appendChild(theading("Node"));
    thead.appendChild(theading("Type"));
    thead.appendChild(theading("Planned"));
    thead.appendChild(theading("Effective"));
    thead.appendChild(theading("Change"));
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    nodes.forEach((node) => {
        const tr = document.createElement("tr");
        tr.appendChild(simpleRow(node.name || ""));
        tr.appendChild(simpleRow(nodeTypeLabel(node)));
        tr.appendChild(simpleRow(node.planned_cpu === null || node.planned_cpu === undefined ? "-" : `CPU ${node.planned_cpu}`));
        tr.appendChild(simpleRow(node.effective_cpu === null || node.effective_cpu === undefined ? "-" : `CPU ${node.effective_cpu}`));
        tr.appendChild(simpleRow(assignmentBadge(node).label));
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
    thead.appendChild(theading("Circuit ID"));
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
        const idCell = document.createElement("td");
        if (c.circuit_id) {
            const a = document.createElement("a");
            a.href = `circuit.html?id=${encodeURIComponent(c.circuit_id)}`;
            a.textContent = c.circuit_id;
            idCell.appendChild(a);
        }
        tr.appendChild(idCell);
        tr.appendChild(simpleRow(c.circuit_name || "", true));
        tr.appendChild(simpleRow(c.parent_node || "", true));
        tr.appendChild(simpleRow(c.classid || ""));
        const weightCell = document.createElement("td");
        if (c.ignored || (c.weight !== undefined && c.weight <= 0)) {
            const badge = document.createElement("span");
            badge.className = "badge bg-secondary";
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
        ["btnTabChanges", "changes"],
    ];
    tabButtons.forEach(([id, tab]) => {
        const btn = document.getElementById(id);
        if (!btn) return;
        btn.className = `btn ${state.detailTab === tab ? "btn-primary" : "btn-outline-secondary"}`;
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
    if (state.detailTab === "changes") {
        renderChangesTable(core);
        enableTooltips();
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
    if (overview && !state.runtimeSnapshot) {
        overview.innerHTML = '<i class="fa fa-spinner fa-spin"></i> Loading core assignment...';
    }

    const configPromise = requestConfig().then((cfg) => {
        state.plannerEnabled = !!(cfg && cfg.queues && cfg.queues.use_binpacking);
    });
    const snapshot = await requestRuntimeSnapshot();
    await configPromise;
    state.runtimeSnapshot = snapshot;
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
    const btnTabChanges = document.getElementById("btnTabChanges");

    pageSizeInput.onchange = () => {
        let v = parseInt(pageSizeInput.value || "50", 10);
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
    btnTabNodes.onclick = () => {
        state.detailTab = "nodes";
        renderSelectedCore();
    };
    btnTabCircuits.onclick = () => {
        state.detailTab = "circuits";
        state.page = 1;
        renderSelectedCore();
    };
    btnTabChanges.onclick = () => {
        state.detailTab = "changes";
        renderSelectedCore();
    };
}

function initLiveCpuSubscription() {
    wsClient.subscribe(["Cpu"]);
    wsClient.on("Cpu", (msg) => {
        state.liveCpuUsage = Array.isArray(msg?.data) ? msg.data.map((value) => toNumber(value, 0)) : [];
        renderOverview();
        renderSelectedSummary();
        enableTooltips();
    });
}

initControls();
renderDetailsHeader();
renderActiveTab();
renderPagerVisibility();
initLiveCpuSubscription();
refreshAll();
