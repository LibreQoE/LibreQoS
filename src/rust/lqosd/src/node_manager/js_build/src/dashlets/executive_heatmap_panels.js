import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {colorByQoqScore, colorByRetransmitPct, colorByRttMs} from "../helpers/color_scales";
import {getSiteIdMap, linkToCircuit, linkToSite} from "../executive_utils";
import {
    buildHeatmapRows,
    colorByCapacity,
    formatLatest,
    latestValue,
    nonNullCount,
    heatRow,
    rttHeatRow,
    MAX_HEATMAP_ROWS,
} from "./executive_heatmap_shared";

function qoqHeatmapRow(blocks, colorFn) {
    const length = Array.isArray(blocks?.download_total) ? blocks.download_total.length : 15;
    const fmt = (v) => formatLatest(v, "", 0);
    let cells = "";
    for (let i = 0; i < length; i++) {
        const dlTotal = blocks?.download_total?.[i];
        const ulTotal = blocks?.upload_total?.[i];
        const dlCurrent = blocks?.download_current?.[i];
        const ulCurrent = blocks?.upload_current?.[i];

        const allMissing =
            (dlTotal === null || dlTotal === undefined) &&
            (ulTotal === null || ulTotal === undefined) &&
            (dlCurrent === null || dlCurrent === undefined) &&
            (ulCurrent === null || ulCurrent === undefined);
        if (allMissing) {
            cells += `<div class="exec-heat-cell empty" title="No data"></div>`;
            continue;
        }

        const title = [
            `Block ${i + 1}`,
            `UL Total: ${fmt(ulTotal)}`,
            `UL Current: ${fmt(ulCurrent)}`,
            `DL Total: ${fmt(dlTotal)}`,
            `DL Current: ${fmt(dlCurrent)}`,
        ].join(" • ");

        const quad = (v) => {
            if (v === null || v === undefined) {
                return `<div class="exec-quad empty"></div>`;
            }
            const numeric = Number(v);
            if (!Number.isFinite(numeric)) {
                return `<div class="exec-quad empty"></div>`;
            }
            const color = colorFn(numeric);
            return `<div class="exec-quad" style="background:${color}"></div>`;
        };

        // Quadrants: upload at top, "total" (global) on the left.
        //  TL: upload_total   TR: upload_current
        //  BL: download_total BR: download_current
        cells += `
            <div class="exec-heat-cell quad" title="${title}">
                <div class="exec-quad-grid">
                    ${quad(ulTotal)}
                    ${quad(ulCurrent)}
                    ${quad(dlTotal)}
                    ${quad(dlCurrent)}
                </div>
            </div>
        `;
    }
    return cells;
}

function qoqLatest(blocks) {
    if (!blocks) return null;
    const values = [
        latestValue(blocks.download_total),
        latestValue(blocks.upload_total),
        latestValue(blocks.download_current),
        latestValue(blocks.upload_current),
    ].filter((v) => v !== null && v !== undefined);
    if (!values.length) return null;
    const sum = values.reduce((a, b) => a + b, 0);
    return sum / values.length;
}

function qoqRow(label, badge, blocks, colorFn) {
    const latest = qoqLatest(blocks);
    const formattedLatest = formatLatest(latest, "", 0);
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${label}">
                <div class="fw-semibold text-truncate">${label}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${qoqHeatmapRow(blocks, colorFn)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
        </div>
    `;
}

class ExecutiveHeatmapBase extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 6;
        this.lastData = null;
    }

    canBeSlowedDown() { return true; }
    subscribeTo() { return ["ExecutiveHeatmaps"]; }

    onMessage(msg) {
        if (msg.event === "ExecutiveHeatmaps") {
            this.lastData = msg.data;
            window.executiveHeatmapData = msg.data;
            this.render();
        }
    }

    setup() {
        if (window.executiveHeatmapData) {
            this.lastData = window.executiveHeatmapData;
            this.render();
        }
    }

    emptyCard() {
        return `
            <div class="card shadow-sm border-0 h-100">
                <div class="card-body py-3 text-muted small">No heatmap data yet.</div>
            </div>
        `;
    }
}

export class ExecutiveGlobalHeatmapDashlet extends ExecutiveHeatmapBase {
    title() { return "Global Heatmap"; }
    tooltip() { return "15 minutes of global metrics, newest on the right."; }

    buildContainer() {
        const container = super.buildContainer();
        this._contentId = `${this.id}_global`;
        const body = document.createElement("div");
        body.id = this._contentId;
        container.appendChild(body);
        return container;
    }

    render() {
        const target = document.getElementById(this._contentId);
        if (!target) return;
        const global = this.lastData?.global;
        const globalQoq = this.lastData?.global_qoq;
        if (!global) {
            target.innerHTML = this.emptyCard();
            return;
        }
        const rows = [
            { kind: "qoo", label: "Overall QoO", badge: "Global", blocks: globalQoq },
            { kind: "rtt", label: "RTT (p50/p90)", badge: "Global", blocks: global },
            { label: "TCP Retransmits", badge: "Global", values: global.retransmit || [], color: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))), format: (v) => formatLatest(v, "%", 1) },
            { label: "Download Utilization", badge: "Global", values: global.download || [], color: colorByCapacity, format: (v) => formatLatest(v, "%") },
            { label: "Upload Utilization", badge: "Global", values: global.upload || [], color: colorByCapacity, format: (v) => formatLatest(v, "%") },
        ];
        const body = rows
            .map((row) => {
                if (row.kind === "qoo") {
                    return qoqRow(row.label, row.badge, row.blocks, colorByQoqScore);
                }
                if (row.kind === "rtt") {
                    return rttHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        (v) => colorByRttMs(v, 200),
                        (v) => formatLatest(v, "ms"),
                    );
                }
                return heatRow(row.label, row.badge, row.values, row.color, row.format);
            })
            .join("");
        target.innerHTML = `
            <div class="card shadow-sm border-0">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0"><i class="fas fa-thermometer-half me-2 text-warning"></i>Global Heatmap</div>
                        <span class="text-muted small">15 minutes, newest on the right</span>
                    </div>
                    <div class="exec-heat-rows">${body}</div>
                </div>
            </div>
        `;
    }
}

class ExecutiveMetricHeatmapBase extends ExecutiveHeatmapBase {
    constructor(slot, config) {
        super(slot);
        this.config = config;
    }

    buildContainer() {
        const container = super.buildContainer();
        this._contentId = `${this.id}_metric`;
        const body = document.createElement("div");
        body.id = this._contentId;
        container.appendChild(body);
        return container;
    }

    render() {
        const target = document.getElementById(this._contentId);
        if (!target) return;
        const rows = buildHeatmapRows(this.lastData || {});
        const filteredRows = this.config.hideAsns ? rows.filter(row => row.badge !== "ASN") : rows;
        if (!filteredRows.length) {
            target.innerHTML = this.emptyCard();
            return;
        }
        const sorted = filteredRows.slice().sort((a, b) => this.metricSort(a, b));
        const limited = sorted.slice(0, MAX_HEATMAP_ROWS);
        getSiteIdMap().then((siteIdMap) => {
            const activeTarget = document.getElementById(this._contentId);
            if (!activeTarget) return;
            const metricRows = limited.map(row => {
                const link = row.badge === "Circuit"
                    ? linkToCircuit(row.circuit_id)
                    : row.badge === "Site"
                        ? linkToSite(row.site_name || row.label, siteIdMap)
                        : null;
                if (this.config.metricKey === "rtt") {
                    return rttHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        this.config.colorFn,
                        this.config.formatFn,
                        link
                    );
                }
                return heatRow(
                    row.label,
                    row.badge,
                    row.blocks[this.config.metricKey] || [],
                    this.config.colorFn,
                    this.config.formatFn,
                    link
                );
            }).join("");
            const linkIcon = this.config.link
                ? `<i class="fas fa-external-link-alt ms-2 small text-muted"></i>`
                : "";
            const titleHtml = this.config.link
                ? `<a class="text-decoration-none text-secondary" href="${this.config.link}"><i class="fas ${this.config.icon} me-2 text-primary"></i>${this.config.title}${linkIcon}</a>`
                : `<i class="fas ${this.config.icon} me-2 text-primary"></i>${this.config.title}`;
            activeTarget.innerHTML = `
                <div class="card shadow-sm border-0 h-100">
                    <div class="card-body py-3">
                        <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                            <div class="exec-section-title mb-0">${titleHtml}</div>
                        </div>
                        <div class="exec-heat-rows">${metricRows}</div>
                    </div>
                </div>
            `;
        });
    }

    metricSort(a, b) {
        const aVals = a.blocks[this.config.metricKey] || [];
        const bVals = b.blocks[this.config.metricKey] || [];
        const aLatest = latestValue(aVals);
        const bLatest = latestValue(bVals);
        const aCount = nonNullCount(aVals);
        const bCount = nonNullCount(bVals);
        const countWeight = this.config.countWeight || 0;
        const minSamples = this.config.minSamples || 0;

        const score = (latest, count) => {
            if (latest === null || latest === undefined) return -Infinity;
            let s = latest + countWeight * (count / 15);
            if (count < minSamples) {
                s -= 1000; // Heavily de-prioritize sparse data
            }
            return s;
        };

        const aScore = score(aLatest, aCount);
        const bScore = score(bLatest, bCount);
        if (bScore !== aScore) return bScore - aScore;
        if (bCount !== aCount) return bCount - aCount;
        return (a.label || "").localeCompare(b.label || "");
    }
}

export class ExecutiveRttHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            title: "Median RTT",
            icon: "fa-stopwatch",
            metricKey: "rtt",
            colorFn: (v) => colorByRttMs(v, 200),
            formatFn: (v) => formatLatest(v, "ms"),
            link: "executive_heatmap_rtt.html",
            countWeight: 100,
            minSamples: 3,
            hideAsns: true,
        });
    }
    title() { return "Median RTT"; }
}

export class ExecutiveRetransmitsHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            title: "TCP Retransmits",
            icon: "fa-undo-alt",
            metricKey: "retransmit",
            colorFn: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))),
            formatFn: (v) => formatLatest(v, "%", 1),
            link: "executive_heatmap_retransmit.html",
            countWeight: 100,
            minSamples: 3,
            hideAsns: true,
        });
    }
    title() { return "TCP Retransmits"; }
}

export class ExecutiveDownloadHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            title: "Download Utilization",
            icon: "fa-arrow-down",
            metricKey: "download",
            colorFn: colorByCapacity,
            formatFn: (v) => formatLatest(v, "%"),
            link: "executive_heatmap_download.html",
            hideAsns: true,
        });
    }
    title() { return "Download Utilization"; }
}

export class ExecutiveUploadHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            title: "Upload Utilization",
            icon: "fa-arrow-up",
            metricKey: "upload",
            colorFn: colorByCapacity,
            formatFn: (v) => formatLatest(v, "%"),
            link: "executive_heatmap_upload.html",
            hideAsns: true,
        });
    }
    title() { return "Upload Utilization"; }
}
