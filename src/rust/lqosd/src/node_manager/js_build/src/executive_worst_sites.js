import {listenExecutiveHeatmaps, medianFromBlocks, averageFromBlocks, renderTable} from "./executive_utils";

function buildRows(data) {
    const sites = (data.sites || []).map(site => {
        const rtt = medianFromBlocks(site.blocks?.rtt || []);
        const avgDown = averageFromBlocks(site.blocks?.download || []);
        const avgUp = averageFromBlocks(site.blocks?.upload || []);
        return {
            name: site.site_name || "Site",
            rtt,
            avgDown,
            avgUp,
        };
    }).filter(row => row.rtt !== null);

    sites.sort((a, b) => (b.rtt || 0) - (a.rtt || 0));
    return sites.slice(0, 10);
}

function render(data) {
    const rows = buildRows(data);
    renderTable("executiveWorstSitesTable", [
        { header: "Site", render: (r) => r.name },
        { header: "Median RTT (ms)", render: (r) => r.rtt !== null ? r.rtt.toFixed(1) : "—" },
        { header: "Avg Down Util (%)", render: (r) => r.avgDown !== null ? r.avgDown.toFixed(1) : "—" },
        { header: "Avg Up Util (%)", render: (r) => r.avgUp !== null ? r.avgUp.toFixed(1) : "—" },
    ], rows, "No site heatmap data yet.");
}

listenExecutiveHeatmaps(render);
