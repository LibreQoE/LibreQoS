import { clearDiv, theading, simpleRow, simpleRowHtml, enableTooltips } from "./helpers/builders";
import { DashboardGraph } from "./graphs/dashboard_graph";

let state = {
    selectedCpu: null,
    direction: "down",
    page: 1,
    pageSize: 50,
    plannerEnabled: null,
};

let dlMaxPie = null; // dashboard-graph wrapped chart for DL Max distribution

function fmtMbps(x) {
    if (x === null || x === undefined) return "-";
    // keep up to 2 decimals, trim trailing zeros
    let s = Number(x).toFixed(2);
    s = s.replace(/\.00$/, "");
    return s;
}

function renderSummary(data) {
    const target = document.getElementById("cpuSummary");
    clearDiv(target);
    if (!data || !Array.isArray(data) || data.length === 0) {
        target.innerHTML = '<p class="text-muted">No queue structure found. Run LibreQoS to generate queuingStructure.json.</p>';
        return;
    }

    // Update binpacking badge in header bar
    const badge = document.getElementById('binpackBadge');
    const enabled = !!state.plannerEnabled;
    if (badge) {
        badge.className = `badge ${enabled ? 'bg-success' : 'bg-secondary'}`;
        badge.textContent = enabled ? "Binpacking: Enabled" : "Binpacking: Disabled";
    }
    // Optional Insight promo below summary
    if (!window.hasInsight) {
        const promo = document.createElement("div");
        promo.className = "text-muted small mb-2";
        promo.innerHTML = `Enable <a href="lts_trial.html">Insight</a> to make binpacking smarter with historical and recent data.`;
        target.appendChild(promo);
    }

    // Build a row with table (left) and chart (right)
    const row = document.createElement('div');
    row.className = 'row';
    const left = document.createElement('div');
    left.className = 'col-12 col-lg-8';
    const right = document.createElement('div');
    right.className = 'col-12 col-lg-4';

    const table = document.createElement("table");
    table.classList.add("table", "table-striped", "table-sm");
    const thead = document.createElement("thead");
    thead.appendChild(theading("CPU"));
    thead.appendChild(theading("DL Circuits"));
    thead.appendChild(theading("Weight"));
    thead.appendChild(theading("DL Max (Mbps)"));
    thead.appendChild(theading("Actions"));
    table.appendChild(thead);
    const tbody = document.createElement("tbody");

    // helper to build a centered cell
    const centerCell = (text) => {
        const td = simpleRow(String(text));
        td.classList.add('text-center');
        return td;
    };

    // simple weight formatter
    const fmtWeight = (x) => {
        const n = Number(x || 0);
        return n > 0 ? n.toLocaleString() : '-';
    };

    data.forEach((row) => {
        const tr = document.createElement("tr");
        tr.appendChild(centerCell(row.cpu));
        tr.appendChild(centerCell(row.download.circuits));
        tr.appendChild(centerCell(fmtWeight(row.download && row.download.weight_sum)));
        tr.appendChild(centerCell(fmtMbps(row.download.max_sum_mbps)));

        const actions = document.createElement("td");
        actions.classList.add('text-center');
        const downBtn = document.createElement("button");
        downBtn.className = "btn btn-sm btn-outline-primary me-1";
        downBtn.innerHTML = "<i class='fa fa-eye'></i> View";
        downBtn.onclick = () => {
            state.selectedCpu = row.cpu;
            state.direction = "down";
            state.page = 1;
            renderDetailsHeader();
            fetchCircuits();
        };

        actions.appendChild(downBtn);
        tr.appendChild(actions);
        tbody.appendChild(tr);
    });

    table.appendChild(tbody);
    left.appendChild(table);

    // Right column pie chart (DL Max per CPU)
    const chartDiv = document.createElement('div');
    chartDiv.id = 'cpuDlMaxPie';
    chartDiv.style.width = '100%';
    chartDiv.style.height = '260px';
    right.appendChild(chartDiv);

    row.appendChild(left);
    row.appendChild(right);
    target.appendChild(row);

    // Build donut (ring) chart with theme support
    try {
        // Recreate the graph each render to bind to the fresh DOM
        if (dlMaxPie && dlMaxPie.chart) {
            try { dlMaxPie.chart.dispose(); } catch (_) {}
            dlMaxPie = null;
        }
        dlMaxPie = new DashboardGraph('cpuDlMaxPie');
        const seriesData = data.map((row) => {
            const w = Number((row.download && row.download.weight_sum) ? row.download.weight_sum : 0);
            const v = Number(row.download.max_sum_mbps || 0);
            return {
                name: `CPU ${row.cpu}`,
                value: (w > 0 ? w : v)
            };
        });
        dlMaxPie.option = {
            tooltip: { trigger: 'item' },
            legend: { show: false },
            series: [
                {
                    name: 'DL Max by CPU',
                    type: 'pie',
                    radius: ['55%','80%'],
                    center: ['50%', '50%'],
                    avoidLabelOverlap: true,
                    data: seriesData,
                    label: {
                        show: true,
                        formatter: '{b}: {d}%'
                    }
                }
            ]
        };
        dlMaxPie.chart.setOption(dlMaxPie.option);
        dlMaxPie.chart.hideLoading();
    } catch (e) {
        // If echarts is unavailable, leave chart blank
    }
}

function renderDetailsHeader() {
    const label = document.getElementById("currentCpuLabel");
    if (state.selectedCpu === null) {
        label.textContent = "";
    } else {
        label.textContent = ` – CPU ${state.selectedCpu}`;
    }
}

function renderCircuits(page) {
    const target = document.getElementById("circuitsTable");
    clearDiv(target);

    if (!page || !Array.isArray(page.items)) {
        target.innerHTML = '<p class="text-secondary">No circuits found for this CPU.</p>';
        return;
    }

    const smallNote = document.createElement("div");
    smallNote.className = "text-muted small mb-1";
    smallNote.innerText = `Total: ${page.total.toLocaleString()} circuits`;
    target.appendChild(smallNote);

    const table = document.createElement("table");
    table.classList.add("table", "table-striped", "table-sm");
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
        const idCell = document.createElement("td");
        if (c.circuit_id) {
            const a = document.createElement("a");
            a.href = `circuit.html?id=${encodeURIComponent(c.circuit_id)}`;
            a.textContent = c.circuit_id;
            idCell.appendChild(a);
        } else {
            idCell.textContent = "";
        }
        tr.appendChild(idCell);
        tr.appendChild(simpleRow(c.circuit_name || ""));
        tr.appendChild(simpleRow(c.parent_node || ""));
        tr.appendChild(simpleRow(c.classid || ""));
        const weightCell = document.createElement('td');
        weightCell.innerText = (c.weight && c.weight > 0) ? c.weight.toLocaleString() : '-';
        tr.appendChild(weightCell);
        tr.appendChild(simpleRow(fmtMbps(c.max_mbps)));
        tr.appendChild(simpleRow(String(c.ip_count || 0)));
        tbody.appendChild(tr);
    });

    table.appendChild(tbody);
    target.appendChild(table);
}

async function fetchSummary() {
    try {
        const r = await fetch("/local-api/cpuAffinity/summary");
        if (!r.ok) throw new Error(`${r.status}`);
        const data = await r.json();
        // data is { entries: [...] } or just array depending on server; we returned a tuple-like wrapper
        const entries = Array.isArray(data) ? data : (data["0"] ? data : (data["entries"] || data));
        // Get planner enabled flag from config
        try {
            const c = await fetch("/local-api/getConfig");
            if (c.ok) {
                const cfg = await c.json();
                state.plannerEnabled = !!(cfg.queues && cfg.queues.use_binpacking);
            }
        } catch (_) {}
        renderSummary(entries); // In our implementation: array
    } catch (e) {
        document.getElementById("cpuSummary").innerHTML = `<div class="text-danger">Failed to load summary: ${e}</div>`;
    }
}

async function fetchCircuits() {
    const target = document.getElementById("circuitsTable");
    target.innerHTML = '<i class="fa fa-spinner fa-spin"></i> Loading circuits...';
    const dir = state.direction;
    const cpu = state.selectedCpu;
    if (cpu === null || cpu === undefined) {
        target.innerHTML = '<i class="fa fa-info-circle"></i> <span class="text-secondary">Select a CPU from the table above.</span>';
        return;
    }
    const params = new URLSearchParams();
    params.set("direction", dir);
    params.set("page", String(state.page));
    params.set("page_size", String(state.pageSize));
    // no search param
    try {
        const r = await fetch(`/local-api/cpuAffinity/circuits/${cpu}?` + params.toString());
        if (!r.ok) throw new Error(`${r.status}`);
        const data = await r.json();
        renderCircuits(data);
    } catch (e) {
        target.innerHTML = `<div class="text-danger">Failed to load circuits: ${e}</div>`;
    }
}

// Wire up controls
function initControls() {
    const pageSizeInput = document.getElementById("pageSize");
    const btnPrev = document.getElementById("btnPrev");
    const btnNext = document.getElementById("btnNext");
    const btnRefresh = document.getElementById("btnRefresh");
    pageSizeInput.onchange = () => {
        let v = parseInt(pageSizeInput.value || "50", 10);
        if (!Number.isFinite(v) || v < 10) v = 10;
        if (v > 1000) v = 1000;
        state.pageSize = v;
        state.page = 1;
        fetchCircuits();
    };
    btnPrev.onclick = () => {
        if (state.page > 1) {
            state.page -= 1;
            fetchCircuits();
        }
    };
    btnNext.onclick = () => {
        state.page += 1;
        fetchCircuits();
    };
    btnRefresh.onclick = () => {
        fetchSummary();
        if (state.selectedCpu !== null) fetchCircuits();
    };
}

// Initialize
initControls();
renderDetailsHeader();
fetchSummary().then(() => enableTooltips());

// No preview section (binpacking) in this view
