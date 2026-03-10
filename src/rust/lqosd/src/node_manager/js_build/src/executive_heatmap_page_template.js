import {listenExecutiveHeatmaps, renderTable, getSiteIdMap, linkToCircuit, linkToSite} from "./executive_utils";
import {
    buildHeatmapRows,
    heatRow,
    rttHeatRow,
    latestValue,
    nonNullCount,
    formatLatest,
    colorByCapacity,
} from "./dashlets/executive_heatmap_shared";
import {colorByRttMs, colorByRetransmitPct} from "./helpers/color_scales";

const metricConfigs = {
    rtt: {
        title: "RTT (p50/p90) Heatmap",
        metricKey: "rtt",
        colorFn: (v) => colorByRttMs(v),
        formatFn: (v) => formatLatest(v, "ms"),
        describeFn: (v) => {
            if (v === null || v === undefined || Number.isNaN(v)) return "";
            if (v < 20) return "Good";
            if (v < 60) return "Moderate";
            if (v < 120) return "Elevated";
            return "Severe";
        },
        icon: "fa-stopwatch",
        description: "Full list of sites, circuits, and ASNs sorted by latest RTT (p50/p90 quadrants).",
        legend: [
            { label: "Good", sample: 10 },
            { label: "Moderate", sample: 40 },
            { label: "Elevated", sample: 90 },
            { label: "Severe", sample: 180 },
        ],
    },
    retransmit: {
        title: "TCP Retransmits Heatmap",
        metricKey: "retransmit",
        colorFn: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))),
        formatFn: (v) => formatLatest(v, "%", 1),
        describeFn: (v) => {
            if (v === null || v === undefined || Number.isNaN(v)) return "";
            if (v < 1) return "Low";
            if (v < 2.5) return "Watch";
            if (v < 5) return "Elevated";
            return "Severe";
        },
        icon: "fa-undo-alt",
        description: "Full list sorted by latest retransmit percentage.",
        legend: [
            { label: "Low", sample: 0.5 },
            { label: "Watch", sample: 1.5 },
            { label: "Elevated", sample: 3.5 },
            { label: "Severe", sample: 7.5 },
        ],
    },
    download: {
        title: "Download Utilization Heatmap",
        metricKey: "download",
        colorFn: colorByCapacity,
        formatFn: (v) => formatLatest(v, "%"),
        describeFn: (v) => {
            if (v === null || v === undefined || Number.isNaN(v)) return "";
            if (v < 60) return "Comfortable";
            if (v < 80) return "Busy";
            if (v < 95) return "Near capacity";
            return "Saturated";
        },
        icon: "fa-arrow-down",
        description: "Full list sorted by latest download utilization.",
        legend: [
            { label: "Comfortable", sample: 30 },
            { label: "Busy", sample: 70 },
            { label: "Near capacity", sample: 90 },
            { label: "Saturated", sample: 99 },
        ],
    },
    upload: {
        title: "Upload Utilization Heatmap",
        metricKey: "upload",
        colorFn: colorByCapacity,
        formatFn: (v) => formatLatest(v, "%"),
        describeFn: (v) => {
            if (v === null || v === undefined || Number.isNaN(v)) return "";
            if (v < 60) return "Comfortable";
            if (v < 80) return "Busy";
            if (v < 95) return "Near capacity";
            return "Saturated";
        },
        icon: "fa-arrow-up",
        description: "Full list sorted by latest upload utilization.",
        legend: [
            { label: "Comfortable", sample: 30 },
            { label: "Busy", sample: 70 },
            { label: "Near capacity", sample: 90 },
            { label: "Saturated", sample: 99 },
        ],
    },
};

function renderLegend(cfg) {
    const items = (cfg.legend || []).map((item) => `
        <span class="badge rounded-pill text-bg-light border d-inline-flex align-items-center gap-2">
            <span class="rounded-circle border" style="display:inline-block;width:0.8rem;height:0.8rem;background:${cfg.colorFn(item.sample)}"></span>
            <span>${item.label}</span>
        </span>
    `).join("");
    return `
        <div class="d-flex flex-wrap gap-2 align-items-center mb-3" role="note" aria-label="Heatmap legend">
            <span class="small text-muted">Legend:</span>
            ${items}
            <span class="small text-muted">Latest value column includes text labels so the page is readable without relying on color alone.</span>
        </div>
    `;
}

function metricSort(metricKey) {
    return (a, b) => {
        const aVals = a.blocks[metricKey] || [];
        const bVals = b.blocks[metricKey] || [];
        const aLatest = latestValue(aVals);
        const bLatest = latestValue(bVals);
        if (bLatest !== aLatest) {
            if (aLatest === null) return 1;
            if (bLatest === null) return -1;
            return bLatest - aLatest;
        }
        const aCount = nonNullCount(aVals);
        const bCount = nonNullCount(bVals);
        if (bCount !== aCount) return bCount - aCount;
        return (a.label || "").localeCompare(b.label || "");
    };
}

function renderHeatmapTable(targetId, metricKey) {
    const cfg = metricConfigs[metricKey];
    if (!cfg) return;
    const target = document.getElementById(targetId);
    if (!target) return;
    let lastData = null;
    const renderRows = (data) => {
        lastData = data;
        getSiteIdMap().then((siteIdMap) => {
            const activeTarget = document.getElementById(targetId);
            if (!activeTarget) return;
            const rows = buildHeatmapRows(data).sort(metricSort(metricKey));
            const body = rows.map(row => {
                const link = row.badge === "Circuit"
                    ? linkToCircuit(row.circuit_id)
                    : row.badge === "Site"
                        ? linkToSite(row.site_name || row.label, siteIdMap)
                        : null;
                if (metricKey === "rtt") {
                    return rttHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        cfg.colorFn,
                        cfg.formatFn,
                        link,
                        cfg.describeFn,
                    );
                }
                return heatRow(
                    row.label,
                    row.badge,
                    row.blocks[metricKey] || [],
                    cfg.colorFn,
                    cfg.formatFn,
                    link,
                    cfg.describeFn,
                );
            }).join("");
            activeTarget.innerHTML = `
                <div class="card shadow-sm">
                    <div class="card-body">
                        <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                            <div class="exec-section-title mb-0"><i class="fas ${cfg.icon} me-2 text-primary"></i>${cfg.title}</div>
                        </div>
                        <p class="text-muted small mb-3">${cfg.description}</p>
                        ${renderLegend(cfg)}
                        <div class="exec-heat-rows" role="list">${body}</div>
                    </div>
                </div>
            `;
        });
    };
    window.addEventListener("colorBlindModeChanged", () => {
        if (lastData) {
            renderRows(lastData);
        }
    });
    listenExecutiveHeatmaps(renderRows);
    // initial placeholder
    target.innerHTML = `<div class="text-muted small">Waiting for heatmap data…</div>`;
}

export function renderHeatmapPage(metricKey) {
    renderHeatmapTable("executiveHeatmapFull", metricKey);
}
