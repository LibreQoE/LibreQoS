import {
    listenExecutiveHeatmaps,
    medianFromBlocks,
    averageFromBlocks,
    renderTable,
    colorSwatch,
    getSiteIdMap,
    renderSiteLink
} from "./executive_utils";
import {colorByRttMs} from "./helpers/color_scales";
import {colorByCapacity} from "./dashlets/executive_heatmap_shared";

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
    return sites.slice(0, 20);
}

function render(data) {
    const rows = buildRows(data);
    getSiteIdMap().then((siteIdMap) => {
        renderTable("executiveWorstSitesTable", [
            { header: "Site", render: (r) => renderSiteLink(r.name, siteIdMap) },
            { header: "Median RTT (ms)", render: (r) => {
                if (r.rtt === null) return "—";
                const color = colorByRttMs(r.rtt, 200);
                return `${colorSwatch(color)}${r.rtt.toFixed(1)}`;
            }},
            { header: "Avg Down Util (%)", render: (r) => {
                if (r.avgDown === null) return "—";
                const color = colorByCapacity(r.avgDown);
                return `${colorSwatch(color)}${r.avgDown.toFixed(1)}`;
            }},
            { header: "Avg Up Util (%)", render: (r) => {
                if (r.avgUp === null) return "—";
                const color = colorByCapacity(r.avgUp);
                return `${colorSwatch(color)}${r.avgUp.toFixed(1)}`;
            }},
        ], rows, "No site heatmap data yet.");
    });
}

listenExecutiveHeatmaps(render);
