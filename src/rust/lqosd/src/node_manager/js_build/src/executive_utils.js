import {get_ws_client, subscribeWS} from "./pubsub/ws";

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export function listenExecutiveHeatmaps(onData) {
    subscribeWS(["ExecutiveHeatmaps"], (msg) => {
        if (msg.event === "ExecutiveHeatmaps") {
            onData(msg.data || {});
        }
    });
}

let siteIdMap = null;
let siteIdMapPromise = null;

export function getSiteIdMap() {
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
        listenOnce("NetworkTree", (msg) => {
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
        });
        wsClient.send({ NetworkTree: {} });
        setTimeout(() => {
            resolveOnce(new Map());
        }, 5000);
    });
    return siteIdMapPromise;
}

export function linkToCircuit(circuitId) {
    if (!circuitId) return null;
    return `circuit.html?id=${encodeURIComponent(circuitId)}`;
}

export function linkToSite(siteName, siteIdLookup) {
    if (!siteName || !siteIdLookup) return null;
    const siteId = siteIdLookup.get(siteName);
    if (siteId === undefined || siteId === null) return null;
    return `tree.html?parent=${encodeURIComponent(siteId)}`;
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
        <div class="table-responsive">
            <table class="table table-sm table-striped align-middle mb-0">
                ${thead}
                <tbody>${tbody}</tbody>
            </table>
        </div>
    `;
}

export function colorSwatch(color) {
    return `<span class="d-inline-block me-1" style="width:12px;height:12px;border-radius:3px;background:${color};"></span>`;
}
