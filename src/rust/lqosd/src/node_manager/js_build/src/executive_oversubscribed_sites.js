import {listenExecutiveHeatmaps, averageWithCount, renderTable} from "./executive_utils";

const MIN_SAMPLES = 6;

function buildRows(data) {
    const rows = (data.sites || []).map(site => {
        const down = averageWithCount(site.blocks?.download || []);
        const up = averageWithCount(site.blocks?.upload || []);
        const combined = ((down.avg || 0) + (up.avg || 0)) / 2;
        return {
            name: site.site_name || "Site",
            down,
            up,
            combined,
        };
    }).filter(r => r.down.count >= MIN_SAMPLES || r.up.count >= MIN_SAMPLES);

    rows.sort((a, b) => (b.combined || 0) - (a.combined || 0));
    return rows.slice(0, 10);
}

function render(data) {
    const rows = buildRows(data);
    renderTable("executiveOversubscribedTable", [
        { header: "Site", render: (r) => r.name },
        { header: "Avg Down Util (%)", render: (r) => r.down.avg !== null ? r.down.avg.toFixed(1) : "—" },
        { header: "Avg Up Util (%)", render: (r) => r.up.avg !== null ? r.up.avg.toFixed(1) : "—" },
        { header: "Combined (%)", render: (r) => r.combined ? r.combined.toFixed(1) : "—" },
    ], rows, "No site utilization data yet.");
}

listenExecutiveHeatmaps(render);
