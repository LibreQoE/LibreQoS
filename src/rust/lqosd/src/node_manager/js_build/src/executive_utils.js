import {subscribeWS} from "./pubsub/ws";

export function listenExecutiveHeatmaps(onData) {
    subscribeWS(["ExecutiveHeatmaps"], (msg) => {
        if (msg.event === "ExecutiveHeatmaps") {
            onData(msg.data || {});
        }
    });
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
            <table class="table table-sm align-middle mb-0">
                ${thead}
                <tbody>${tbody}</tbody>
            </table>
        </div>
    `;
}
