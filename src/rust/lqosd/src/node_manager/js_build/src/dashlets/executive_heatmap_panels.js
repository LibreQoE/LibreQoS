import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {colorByRetransmitPct, colorByRttMs} from "../helpers/color_scales";
import {
    buildHeatmapRows,
    colorByCapacity,
    formatLatest,
    latestValue,
    nonNullCount,
    heatRow,
    MAX_HEATMAP_ROWS,
} from "./executive_heatmap_shared";

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
        if (!global) {
            target.innerHTML = this.emptyCard();
            return;
        }
        const rows = [
            { label: "Median RTT", badge: "Global", values: global.rtt || [], color: (v) => colorByRttMs(v, 200), format: (v) => formatLatest(v, "ms") },
            { label: "TCP Retransmits", badge: "Global", values: global.retransmit || [], color: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))), format: (v) => formatLatest(v, "%", 1) },
            { label: "Download Utilization", badge: "Global", values: global.download || [], color: colorByCapacity, format: (v) => formatLatest(v, "%") },
            { label: "Upload Utilization", badge: "Global", values: global.upload || [], color: colorByCapacity, format: (v) => formatLatest(v, "%") },
        ];
        const body = rows.map(row => heatRow(row.label, row.badge, row.values, row.color, row.format)).join("");
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
        if (!rows.length) {
            target.innerHTML = this.emptyCard();
            return;
        }
        const sorted = rows.slice().sort((a, b) => this.metricSort(a, b));
        const limited = sorted.slice(0, MAX_HEATMAP_ROWS);
        const metricRows = limited.map(row => heatRow(
            row.label,
            row.badge,
            row.blocks[this.config.metricKey] || [],
            this.config.colorFn,
            this.config.formatFn
        )).join("");
        const titleHtml = this.config.link
            ? `<a class="text-decoration-none text-secondary" href="${this.config.link}"><i class="fas ${this.config.icon} me-2 text-primary"></i>${this.config.title}</a>`
            : `<i class="fas ${this.config.icon} me-2 text-primary"></i>${this.config.title}`;
        target.innerHTML = `
            <div class="card shadow-sm border-0 h-100">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0">${titleHtml}</div>
                        <span class="badge bg-light text-secondary border">${rows.length} rows</span>
                    </div>
                    <div class="exec-heat-rows">${metricRows}</div>
                </div>
            </div>
        `;
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
        });
    }
    title() { return "Upload Utilization"; }
}
