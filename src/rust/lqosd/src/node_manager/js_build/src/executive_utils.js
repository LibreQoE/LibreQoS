import {get_ws_client, subscribeWS} from "./pubsub/ws";

const wsClient = get_ws_client();
const DEFAULT_EXECUTIVE_REFRESH_MS = 5000;
let executiveRequestCounter = 0;

const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

function normalizeEntityKinds(entityKinds) {
    return [...new Set((entityKinds || []).map((kind) => String(kind)))].sort();
}

function nextExecutiveRequestId(prefix) {
    executiveRequestCounter += 1;
    return `${prefix}-${Date.now()}-${executiveRequestCounter}`;
}

function matchesExecutiveHeatmapQuery(responseQuery, requestQuery) {
    if (!responseQuery || !requestQuery) return false;
    const responseRequestId = String(responseQuery.client_request_id || "");
    const requestRequestId = String(requestQuery.client_request_id || "");
    if (responseRequestId || requestRequestId) {
        return responseRequestId !== "" && responseRequestId === requestRequestId;
    }
    return String(responseQuery.metric || "") === String(requestQuery.metric || "")
        && Number(responseQuery.page ?? 0) === Number(requestQuery.page ?? 0)
        && Number(responseQuery.page_size ?? 0) === Number(requestQuery.page_size ?? 0)
        && String(responseQuery.sort || "LatestValue") === String(requestQuery.sort || "LatestValue")
        && Boolean(responseQuery.descending ?? true) === Boolean(requestQuery.descending ?? true)
        && String(responseQuery.search || "") === String(requestQuery.search || "")
        && JSON.stringify(normalizeEntityKinds(responseQuery.entity_kinds)) === JSON.stringify(normalizeEntityKinds(requestQuery.entity_kinds));
}

function matchesExecutiveLeaderboardQuery(responseQuery, requestQuery) {
    if (!responseQuery || !requestQuery) return false;
    const responseRequestId = String(responseQuery.client_request_id || "");
    const requestRequestId = String(requestQuery.client_request_id || "");
    if (responseRequestId || requestRequestId) {
        return responseRequestId !== "" && responseRequestId === requestRequestId;
    }
    return String(responseQuery.kind || "") === String(requestQuery.kind || "")
        && Number(responseQuery.page ?? 0) === Number(requestQuery.page ?? 0)
        && Number(responseQuery.page_size ?? 0) === Number(requestQuery.page_size ?? 0)
        && String(responseQuery.search || "") === String(requestQuery.search || "");
}

function requestMatchingResponse(eventName, request, matcher, onData, onError = null) {
    let disposed = false;
    const cleanup = wsClient.on(eventName, (msg) => {
        if (disposed) return;
        const data = msg?.data || {};
        if (!matcher(data)) return;
        disposed = true;
        cleanup();
        onData(data);
    });
    try {
        wsClient.send(request);
    } catch (err) {
        disposed = true;
        cleanup();
        if (onError) {
            onError(err);
        } else {
            console.error(err);
        }
    }
    return () => {
        if (disposed) return;
        disposed = true;
        cleanup();
    };
}

function pollMatchingResponse(eventName, buildRequest, matcher, onData, intervalMs = DEFAULT_EXECUTIVE_REFRESH_MS) {
    const handlerDisposer = wsClient.on(eventName, (msg) => {
        const data = msg?.data || {};
        if (!matcher(data)) return;
        onData(data);
    });
    const sendRequest = () => {
        wsClient.send(buildRequest());
    };
    sendRequest();
    const timer = window.setInterval(sendRequest, intervalMs);
    return {
        dispose() {
            window.clearInterval(timer);
            handlerDisposer();
        },
        refresh() {
            sendRequest();
        },
    };
}

export function listenExecutiveDashboardSummary(onData) {
    return subscribeWS(["ExecutiveDashboardSummary"], (msg) => {
        if (msg.event === "ExecutiveDashboardSummary") {
            onData(msg.data || {});
        }
    });
}

export function requestExecutiveHeatmapPage(query, onData, onError = null) {
    const requestQuery = {
        ...query,
        client_request_id: nextExecutiveRequestId("exec-heatmap"),
    };
    return requestMatchingResponse(
        "ExecutiveHeatmapPage",
        { ExecutiveHeatmapPage: { query: requestQuery } },
        (data) => matchesExecutiveHeatmapQuery(data.query, requestQuery),
        onData,
        onError,
    );
}

export function pollExecutiveHeatmapPage(query, onData, intervalMs = DEFAULT_EXECUTIVE_REFRESH_MS) {
    const requestQuery = { ...query };
    return pollMatchingResponse(
        "ExecutiveHeatmapPage",
        () => {
            requestQuery.client_request_id = nextExecutiveRequestId("exec-heatmap");
            return { ExecutiveHeatmapPage: { query: requestQuery } };
        },
        (data) => matchesExecutiveHeatmapQuery(data.query, requestQuery),
        onData,
        intervalMs,
    );
}

export function requestExecutiveLeaderboardPage(query, onData, onError = null) {
    const requestQuery = {
        ...query,
        client_request_id: nextExecutiveRequestId("exec-leaderboard"),
    };
    return requestMatchingResponse(
        "ExecutiveLeaderboardPage",
        { ExecutiveLeaderboardPage: { query: requestQuery } },
        (data) => matchesExecutiveLeaderboardQuery(data.query, requestQuery),
        onData,
        onError,
    );
}

export function pollExecutiveLeaderboardPage(query, onData, intervalMs = DEFAULT_EXECUTIVE_REFRESH_MS) {
    const requestQuery = { ...query };
    return pollMatchingResponse(
        "ExecutiveLeaderboardPage",
        () => {
            requestQuery.client_request_id = nextExecutiveRequestId("exec-leaderboard");
            return { ExecutiveLeaderboardPage: { query: requestQuery } };
        },
        (data) => matchesExecutiveLeaderboardQuery(data.query, requestQuery),
        onData,
        intervalMs,
    );
}

let siteIdMap = null;
let siteIdMapPromise = null;

export function getNodeIdMap() {
    if (siteIdMap) return Promise.resolve(siteIdMap);
    if (siteIdMapPromise) return siteIdMapPromise;
    siteIdMapPromise = new Promise((resolve) => {
        let resolved = false;
        const resolveOnce = (map) => {
            if (resolved) return;
            resolved = true;
            siteIdMap = map;
            resolve(map);
        };
        listenOnce("NetworkTreeLite", (msg) => {
            const data = msg && msg.data ? msg.data : [];
            const map = new Map();
            (data || []).forEach((entry) => {
                if (!Array.isArray(entry) || entry.length < 2) return;
                const [id, node] = entry;
                if (!node || !node.name) return;
                if (!map.has(node.name)) {
                    map.set(node.name, id);
                }
            });
            resolveOnce(map);
        });
        wsClient.send({ NetworkTreeLite: {} });
        setTimeout(() => {
            resolveOnce(new Map());
        }, 5000);
    });
    return siteIdMapPromise;
}

export function getSiteIdMap() {
    return getNodeIdMap();
}

export function linkToCircuit(circuitId) {
    if (!circuitId) return null;
    return `circuit.html?id=${encodeURIComponent(circuitId)}`;
}

export function linkToTreeLocator(locator) {
    if (!locator) return null;
    const params = new URLSearchParams();
    if (locator.parent_index !== undefined && locator.parent_index !== null) {
        params.set("parent", String(locator.parent_index));
    }
    if (locator.node_id) {
        params.set("nodeId", locator.node_id);
    }
    if (Array.isArray(locator.node_path) && locator.node_path.length > 0) {
        params.set("nodePath", JSON.stringify(locator.node_path));
    }
    const query = params.toString();
    return query ? `tree.html?${query}` : null;
}

export function linkToSite(siteName, siteIdLookup) {
    if (!siteName || !siteIdLookup) return null;
    const siteId = siteIdLookup.get(siteName);
    if (siteId === undefined || siteId === null) return null;
    return `tree.html?parent=${encodeURIComponent(siteId)}`;
}

export function linkToTreeNode(nodeName, nodeIdLookup) {
    if (!nodeName || !nodeIdLookup) return null;
    const nodeId = nodeIdLookup.get(nodeName);
    if (nodeId === undefined || nodeId === null) return null;
    return `tree.html?parent=${encodeURIComponent(nodeId)}`;
}

export function badgeForEntityKind(entityKind) {
    switch (String(entityKind || "")) {
    case "Site":
        return "Site";
    case "Circuit":
        return "Circuit";
    case "Asn":
        return "ASN";
    default:
        return "";
    }
}

export function linkToExecutiveMetricRow(row) {
    if (!row) return null;
    if (row.circuit_id) {
        return linkToCircuit(row.circuit_id);
    }
    return linkToTreeLocator(row.tree);
}

export function linkToExecutiveLeaderboardRow(row) {
    if (!row || !row.kind) return null;
    switch (row.kind) {
    case "WorstSiteByRtt":
    case "OversubscribedSite":
    case "SiteDueUpgrade":
        return linkToTreeLocator(row.tree);
    case "CircuitDueUpgrade":
        return linkToCircuit(row.circuit_id);
    default:
        return null;
    }
}

export function renderCircuitLink(name, circuitId) {
    const link = linkToCircuit(circuitId);
    if (!link) return `<span class="redactable">${name}</span>`;
    return `<a class="redactable" href="${link}">${name}</a>`;
}

export function renderSiteLink(name, siteIdLookup) {
    const link = linkToSite(name, siteIdLookup);
    if (!link) return `<span class="redactable">${name}</span>`;
    return `<a class="redactable" href="${link}">${name}</a>`;
}

export function median(values) {
    const vals = (values || []).filter(v => v !== null && v !== undefined && Number.isFinite(Number(v))).map(Number);
    if (!vals.length) return null;
    vals.sort((a, b) => a - b);
    const mid = Math.floor(vals.length / 2);
    if (vals.length % 2 === 1) return vals[mid];
    return (vals[mid - 1] + vals[mid]) / 2;
}

export function averageWithCount(values) {
    const vals = (values || []).filter(v => v !== null && v !== undefined && Number.isFinite(Number(v))).map(Number);
    if (!vals.length) return { avg: null, count: 0 };
    const sum = vals.reduce((a, b) => a + b, 0);
    return { avg: sum / vals.length, count: vals.length };
}

export function medianFromBlocks(blocksArray) {
    return median(blocksArray);
}

export function averageFromBlocks(blocksArray) {
    return averageWithCount(blocksArray).avg;
}

export function sumBlocks(blocksArray) {
    const vals = (blocksArray || []).filter(v => v !== null && v !== undefined && Number.isFinite(Number(v))).map(Number);
    return vals.reduce((a, b) => a + b, 0);
}

export function renderTable(targetId, columns, rows, emptyMessage) {
    const target = document.getElementById(targetId);
    if (!target) return;
    if (!rows.length) {
        target.innerHTML = `<div class="text-muted small">${emptyMessage}</div>`;
        return;
    }
    const thead = `<thead><tr>${columns.map(c => `<th scope="col">${c.header}</th>`).join("")}</tr></thead>`;
    const tbody = rows.map(row => {
        const cells = columns.map(c => `<td>${c.render(row)}</td>`).join("");
        return `<tr>${cells}</tr>`;
    }).join("");
    target.innerHTML = `
        <div class="table-responsive lqos-table-wrap">
            <table class="lqos-table lqos-table-compact align-middle mb-0">
                ${thead}
                <tbody>${tbody}</tbody>
            </table>
        </div>
    `;
}

export function colorSwatch(color) {
    return `<span class="d-inline-block me-1" style="width:12px;height:12px;border-radius:3px;background:${color};"></span>`;
}
