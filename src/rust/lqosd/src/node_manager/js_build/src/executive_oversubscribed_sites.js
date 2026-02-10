import {
    listenExecutiveHeatmaps,
    averageWithCount,
    renderTable,
    medianFromBlocks,
    getSiteIdMap,
    renderSiteLink
} from "./executive_utils";
import {colorSwatch} from "./executive_utils";
import {colorByCapacity, formatLatest} from "./dashlets/executive_heatmap_shared";
import {colorByRttMs} from "./helpers/color_scales";

const MIN_SAMPLES = 6;
const MAX_ROWS = 20;

function buildRows(data) {
    const oversub = data.oversubscribed_sites || [];
    if (!oversub.length) return [];

    // Build a lookup for heatmap stats by site
    const siteStats = new Map();
    (data.sites || []).forEach(site => {
        const down = averageWithCount(site.blocks?.download || []);
        const up = averageWithCount(site.blocks?.upload || []);
        const rtt = medianFromBlocks(site.blocks?.rtt || []);
        siteStats.set(site.site_name || site.name || "", {
            down,
            up,
            rtt,
        });
    });

    const rows = oversub.map(item => {
        const name = item.site_name || "Site";
        const stats = siteStats.get(name) || {};
        const ratio = item.ratio_max ?? Math.max(item.ratio_down || 0, item.ratio_up || 0);
        return {
            name,
            ratio_down: item.ratio_down,
            ratio_up: item.ratio_up,
            ratio,
            cap_down: item.cap_down,
            cap_up: item.cap_up,
            sub_down: item.sub_down,
            sub_up: item.sub_up,
            perf_down: stats.down || { avg: null, count: 0 },
            perf_up: stats.up || { avg: null, count: 0 },
            rtt: stats.rtt ?? null,
        };
    });

    rows.sort((a, b) => (b.ratio || 0) - (a.ratio || 0));
    return rows.slice(0, MAX_ROWS);
}

function fmtRatio(r) {
    if (r === undefined || r === null || Number.isNaN(r)) return "—";
    return `${r.toFixed(2)}x`;
}

function fmtMbps(val) {
    if (val === undefined || val === null || Number.isNaN(val)) return "—";
    return `${val.toFixed(val >= 10 ? 1 : 2)} Mbps`;
}

function render(data) {
    const rows = buildRows(data);
    getSiteIdMap().then((siteIdMap) => {
        renderTable("executiveOversubscribedTable", [
            { header: "Site", render: (r) => renderSiteLink(r.name, siteIdMap) },
            { header: "Oversub Down", render: (r) => fmtRatio(r.ratio_down) },
            { header: "Oversub Up", render: (r) => fmtRatio(r.ratio_up) },
            { header: "Cap (D/U)", render: (r) => `${fmtMbps(r.cap_down)} / ${fmtMbps(r.cap_up)}` },
            { header: "Subscribed (D/U)", render: (r) => `${fmtMbps(r.sub_down)} / ${fmtMbps(r.sub_up)}` },
            { header: "Median RTT (ms)", render: (r) => {
                if (r.rtt === null || r.rtt === undefined) return "—";
                const color = colorByRttMs(r.rtt, 200);
                return `${colorSwatch(color)}${formatLatest(r.rtt, "ms", 1)}`;
            }},
            { header: "Avg Down Util (%)", render: (r) => {
                if (r.perf_down.avg === null || r.perf_down.avg === undefined) return "—";
                const color = colorByCapacity(r.perf_down.avg);
                return `${colorSwatch(color)}${r.perf_down.avg.toFixed(1)}`;
            }},
            { header: "Avg Up Util (%)", render: (r) => {
                if (r.perf_up.avg === null || r.perf_up.avg === undefined) return "—";
                const color = colorByCapacity(r.perf_up.avg);
                return `${colorSwatch(color)}${r.perf_up.avg.toFixed(1)}`;
            }},
        ], rows, "No oversubscription data available yet.");
    });
}

listenExecutiveHeatmaps(render);
