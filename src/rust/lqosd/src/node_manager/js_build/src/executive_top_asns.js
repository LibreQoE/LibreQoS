import {listenExecutiveHeatmaps, medianFromBlocks, sumBlocks, renderTable} from "./executive_utils";

const SECONDS_PER_BLOCK = 60;
const BITS_PER_MEGABIT = 1_000_000;

function buildRows(data) {
    const rows = (data.asns || []).map(asn => {
        const downSum = sumBlocks(asn.blocks?.download || []);
        const upSum = sumBlocks(asn.blocks?.upload || []);
        const totalMbps = downSum + upSum;
        const totalBytes = (totalMbps * BITS_PER_MEGABIT / 8) * SECONDS_PER_BLOCK;
        const medianRtt = medianFromBlocks(asn.blocks?.rtt || []);
        const medianRetrans = medianFromBlocks(asn.blocks?.retransmit || []);
        return {
            asn: asn.asn,
            totalBytes,
            medianRtt,
            medianRetrans,
        };
    });
    rows.sort((a, b) => b.totalBytes - a.totalBytes);
    return rows.slice(0, 10);
}

function formatBytes(bytes) {
    if (!Number.isFinite(bytes) || bytes <= 0) return "—";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let val = bytes;
    while (val >= 1024 && i < units.length - 1) {
        val /= 1024;
        i++;
    }
    return `${val.toFixed(val >= 10 ? 1 : 2)} ${units[i]}`;
}

function render(data) {
    const rows = buildRows(data);
    renderTable("executiveTopAsnsTable", [
        { header: "ASN", render: (r) => r.asn },
        { header: "Total Traffic (15m)", render: (r) => formatBytes(r.totalBytes) },
        { header: "Median RTT (ms)", render: (r) => r.medianRtt !== null ? r.medianRtt.toFixed(1) : "—" },
        { header: "Median Retrans (%)", render: (r) => r.medianRetrans !== null ? r.medianRetrans.toFixed(2) : "—" },
    ], rows, "No ASN heatmap data yet.");
}

listenExecutiveHeatmaps(render);
