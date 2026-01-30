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
        colorFn: (v) => colorByRttMs(v, 200),
        formatFn: (v) => formatLatest(v, "ms"),
        icon: "fa-stopwatch",
        description: "Full list of sites, circuits, and ASNs sorted by latest RTT (p50/p90 quadrants).",
    },
    retransmit: {
        title: "TCP Retransmits Heatmap",
        metricKey: "retransmit",
        colorFn: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))),
        formatFn: (v) => formatLatest(v, "%", 1),
        icon: "fa-undo-alt",
        description: "Full list sorted by latest retransmit percentage.",
    },
    download: {
        title: "Download Utilization Heatmap",
        metricKey: "download",
        colorFn: colorByCapacity,
        formatFn: (v) => formatLatest(v, "%"),
        icon: "fa-arrow-down",
        description: "Full list sorted by latest download utilization.",
    },
    upload: {
        title: "Upload Utilization Heatmap",
        metricKey: "upload",
        colorFn: colorByCapacity,
        formatFn: (v) => formatLatest(v, "%"),
        icon: "fa-arrow-up",
        description: "Full list sorted by latest upload utilization.",
    },
};

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
    const renderRows = (data) => {
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
                    );
                }
                return heatRow(
                    row.label,
                    row.badge,
                    row.blocks[metricKey] || [],
                    cfg.colorFn,
                    cfg.formatFn,
                    link,
                );
            }).join("");
            activeTarget.innerHTML = `
                <div class="card shadow-sm">
                    <div class="card-body">
                        <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                            <div class="exec-section-title mb-0"><i class="fas ${cfg.icon} me-2 text-primary"></i>${cfg.title}</div>
                        </div>
                        <p class="text-muted small mb-3">${cfg.description}</p>
                        <div class="exec-heat-rows">${body}</div>
                    </div>
                </div>
            `;
        });
    };
    listenExecutiveHeatmaps(renderRows);
    // initial placeholder
    target.innerHTML = `<div class="text-muted small">Waiting for heatmap data…</div>`;
}

export function renderHeatmapPage(metricKey) {
    renderHeatmapTable("executiveHeatmapFull", metricKey);
}
