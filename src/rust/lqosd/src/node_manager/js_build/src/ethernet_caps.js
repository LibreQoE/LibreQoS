import {clearDiv, simpleRow, theading} from "./helpers/builders";
import {formatMbps} from "./helpers/scaling";
import {get_ws_client} from "./pubsub/ws";

const wsClient = get_ws_client();
const params = new Proxy(new URLSearchParams(window.location.search), {
    get: (searchParams, prop) => searchParams.get(prop),
});

let page = 0;
let pageSize = 100;
let search = "";
let tier = "";
let totalRows = 0;

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

function formatPlanPair(down, up) {
    return `${formatMbps(down)} / ${formatMbps(up)} Mbps`;
}

function tierBadgeClass(tierLabel) {
    if (tierLabel === "10M") return "text-bg-danger";
    if (tierLabel === "100M") return "text-bg-warning";
    return "text-bg-info";
}

function buildTierBadge(row) {
    const badge = document.createElement("span");
    badge.className = `badge rounded-pill ${tierBadgeClass(row?.badge?.tier_label || "")}`;
    badge.textContent = row?.badge?.tier_label || "?";
    return badge;
}

function tableRow(row) {
    const tr = document.createElement("tr");
    tr.classList.add("small");

    const circuitCell = document.createElement("td");
    const link = document.createElement("a");
    link.href = `/circuit.html?id=${encodeURIComponent(row.circuit_id)}`;
    link.classList.add("redactable");
    link.textContent = row.circuit_name || row.circuit_id;
    circuitCell.appendChild(link);
    tr.appendChild(circuitCell);

    tr.appendChild(simpleRow(row.parent_node || "-", true));

    const ethernetCell = document.createElement("td");
    ethernetCell.appendChild(buildTierBadge(row));
    tr.appendChild(ethernetCell);

    tr.appendChild(simpleRow(formatPlanPair(row.badge.requested_download_mbps, row.badge.requested_upload_mbps)));
    tr.appendChild(simpleRow(formatPlanPair(row.badge.applied_download_mbps, row.badge.applied_upload_mbps)));
    tr.appendChild(simpleRow(row.limiting_device_name || "-"));
    tr.appendChild(simpleRow(row.limiting_interface_name || "-"));

    return tr;
}

function renderTable(rows) {
    const target = document.getElementById("ethernetCapsTableWrap");
    if (!target) return;
    clearDiv(target);

    if (!Array.isArray(rows) || rows.length === 0) {
        const empty = document.createElement("div");
        empty.className = "text-muted small";
        empty.textContent = "No Ethernet-limited circuits matched this filter.";
        target.appendChild(empty);
        return;
    }

    const wrap = document.createElement("div");
    wrap.classList.add("lqos-table-wrap");
    const table = document.createElement("table");
    table.classList.add("lqos-table", "lqos-table-tight");

    const thead = document.createElement("thead");
    thead.appendChild(theading("Circuit"));
    thead.appendChild(theading("Parent"));
    thead.appendChild(theading("Ethernet"));
    thead.appendChild(theading("Requested"));
    thead.appendChild(theading("Applied"));
    thead.appendChild(theading("Limiting Device"));
    thead.appendChild(theading("Interface"));
    table.appendChild(thead);

    const tbody = document.createElement("tbody");
    rows.forEach((row) => tbody.appendChild(tableRow(row)));
    table.appendChild(tbody);
    wrap.appendChild(table);
    target.appendChild(wrap);
}

function updateSummary(query, rows) {
    const summary = document.getElementById("ethernetCapsSummary");
    const pager = document.getElementById("ethernetCapsPager");
    const prev = document.getElementById("ethernetCapsPrev");
    const next = document.getElementById("ethernetCapsNext");
    if (!summary || !pager || !prev || !next) return;

    const totalPages = Math.max(1, Math.ceil(totalRows / pageSize));
    const currentPage = (query?.page ?? 0) + 1;
    const start = totalRows === 0 ? 0 : (currentPage - 1) * pageSize + 1;
    const end = totalRows === 0 ? 0 : start + Math.max(0, rows.length - 1);

    summary.textContent = totalRows === 0
        ? "No active Ethernet caps"
        : `${totalRows} Ethernet-limited circuits`;
    pager.textContent = totalRows === 0
        ? "Page 1 / 1"
        : `Showing ${start}–${end} of ${totalRows} • Page ${currentPage} / ${totalPages}`;
    prev.disabled = currentPage <= 1;
    next.disabled = currentPage >= totalPages;
}

async function requestPage() {
    try {
        const query = {
            page,
            page_size: pageSize,
        };
        if (search.trim()) {
            query.search = search.trim();
        }
        if (tier) {
            query.tier = tier === "10M" ? "TenM" : tier === "100M" ? "HundredM" : "GigPlus";
        }
        const msg = await sendWsRequest("EthernetCapsPage", {
            EthernetCapsPage: { query },
        });
        const data = msg?.data || { rows: [], total_rows: 0, query };
        totalRows = Number.isFinite(Number(data.total_rows)) ? Number(data.total_rows) : 0;
        renderTable(data.rows || []);
        updateSummary(data.query, data.rows || []);
    } catch (_error) {
        totalRows = 0;
        renderTable([]);
        updateSummary({ page: 0 }, []);
    }
}

function bindControls() {
    const searchInput = document.getElementById("ethernetCapsSearch");
    const tierSelect = document.getElementById("ethernetCapsTier");
    const pageSizeSelect = document.getElementById("ethernetCapsPageSize");
    const prev = document.getElementById("ethernetCapsPrev");
    const next = document.getElementById("ethernetCapsNext");

    if (searchInput) {
        searchInput.value = search;
        searchInput.addEventListener("input", () => {
            search = searchInput.value || "";
            page = 0;
            requestPage();
        });
    }
    if (tierSelect) {
        tierSelect.value = tier;
        tierSelect.addEventListener("change", () => {
            tier = tierSelect.value || "";
            page = 0;
            requestPage();
        });
    }
    if (pageSizeSelect) {
        pageSizeSelect.value = String(pageSize);
        pageSizeSelect.addEventListener("change", () => {
            const nextSize = parseInt(pageSizeSelect.value, 10);
            pageSize = Number.isFinite(nextSize) && nextSize > 0 ? nextSize : 100;
            page = 0;
            requestPage();
        });
    }
    prev?.addEventListener("click", () => {
        page = Math.max(0, page - 1);
        requestPage();
    });
    next?.addEventListener("click", () => {
        const totalPages = Math.max(1, Math.ceil(totalRows / pageSize));
        page = Math.min(totalPages - 1, page + 1);
        requestPage();
    });
}

function initFromQueryString() {
    const rawTier = typeof params.tier === "string" ? params.tier.trim().toUpperCase() : "";
    tier = rawTier === "10M" || rawTier === "100M" || rawTier === "1G" ? rawTier : "";
    search = typeof params.search === "string" ? params.search : "";
}

wsClient.on("join", () => {
    requestPage();
});

initFromQueryString();
bindControls();
requestPage();
