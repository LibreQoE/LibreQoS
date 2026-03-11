import {listenExecutiveHeatmaps, averageWithCount, renderTable, getSiteIdMap, renderSiteLink} from "./executive_utils";

const THRESHOLD = 80;
const MIN_SAMPLES = 10;

function buildRows(data) {
    const rows = (data.sites || []).map(site => {
        const down = averageWithCount(site.blocks?.download || []);
        const up = averageWithCount(site.blocks?.upload || []);
        return {
            name: site.site_name || "Site",
            down,
            up,
        };
    }).filter(row =>
        row.down.count >= MIN_SAMPLES &&
        row.up.count >= MIN_SAMPLES &&
        (row.down.avg || 0) >= THRESHOLD &&
        (row.up.avg || 0) >= THRESHOLD
    );

    rows.sort((a, b) => ((b.down.avg || 0) + (b.up.avg || 0)) - ((a.down.avg || 0) + (a.up.avg || 0)));
    return rows;
}

function render(data) {
    const rows = buildRows(data);
    getSiteIdMap().then((siteIdMap) => {
        renderTable("executiveSitesDueTable", [
            { header: "Site", render: (r) => renderSiteLink(r.name, siteIdMap) },
            { header: "Avg Down Util (%)", render: (r) => r.down.avg !== null ? r.down.avg.toFixed(1) : "—" },
            { header: "Avg Up Util (%)", render: (r) => r.up.avg !== null ? r.up.avg.toFixed(1) : "—" },
        ], rows, "No sites meet the 80%+ utilization threshold yet.");
    });
}

listenExecutiveHeatmaps(render);
