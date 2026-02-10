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
    retransmitHeatRow,
    utilizationHeatRow,
    MAX_HEATMAP_ROWS,
} from "./executive_heatmap_shared";

const QOO_TOOLTIP_HTML = [
    `Quality of Outcome (QoO) is IETF IPPM “Internet Quality” (draft-ietf-ippm-qoo).`,
    `https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/`,
    `LibreQoS implements a latency- and loss-based model to estimate Quality of Outcome.`,
].join("<br>");

function escapeAttr(value) {
    return String(value)
        .replace(/&/g, "&amp;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function escapeHtml(value) {
    return String(value)
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;")
        .replace(/"/g, "&quot;")
        .replace(/'/g, "&#039;");
}

function qooInfoIconHtml() {
    const title = escapeAttr(QOO_TOOLTIP_HTML);
    return `<span class="ms-1 text-muted" role="button" tabindex="0" aria-label="QoO information" data-bs-toggle="tooltip" data-bs-placement="top" data-bs-html="true" title="${title}"><i class="fas fa-info-circle"></i></span>`;
}

function initTooltipsWithin(rootEl) {
    if (!rootEl) return;
    if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) return;
    const elements = rootEl.querySelectorAll('[data-bs-toggle="tooltip"]');
    elements.forEach((element) => {
        if (bootstrap.Tooltip.getOrCreateInstance) {
            bootstrap.Tooltip.getOrCreateInstance(element);
        } else {
            new bootstrap.Tooltip(element);
        }
    });
}

function qoqHeatmapRow(blocks, colorFn) {
    const length =
        Array.isArray(blocks?.upload_total) && blocks.upload_total.length
            ? blocks.upload_total.length
            : (Array.isArray(blocks?.download_total) && blocks.download_total.length ? blocks.download_total.length : 15);
    const fmt = (v) => formatLatest(v, "", 0);
    let cells = "";
    for (let i = 0; i < length; i++) {
        const ulTotal = blocks?.upload_total?.[i];
        const dlTotal = blocks?.download_total?.[i];

        const allMissing =
            (dlTotal === null || dlTotal === undefined) &&
            (ulTotal === null || ulTotal === undefined);
        if (allMissing) {
            cells += `<div class="exec-heat-cell empty" title="No data"></div>`;
            continue;
        }

        const title = [
            `Block ${i + 1}`,
            `UL Total: ${fmt(ulTotal)}`,
            `DL Total: ${fmt(dlTotal)}`,
        ].join(" • ");

        const part = (v) => {
            if (v === null || v === undefined) {
                return `<div class="exec-split empty"></div>`;
            }
            const numeric = Number(v);
            if (!Number.isFinite(numeric)) {
                return `<div class="exec-split empty"></div>`;
            }
            const color = colorFn(numeric);
            return `<div class="exec-split" style="background:${color}"></div>`;
        };

        // Top = upload, bottom = download.
        cells += `
            <div class="exec-heat-cell split" title="${title}">
                <div class="exec-split-grid">
                    ${part(ulTotal)}
                    ${part(dlTotal)}
                </div>
            </div>
        `;
    }
    return cells;
}

function qoqRow(labelText, badge, blocks, colorFn, labelHtml = null) {
    const topValues = blocks?.upload_total || [];
    const bottomValues = blocks?.download_total || [];
    const latestTop = latestValue(topValues);
    const latestBottom = latestValue(bottomValues);
    const formattedLatest = `
        <div>${formatLatest(latestTop, "", 0)}</div>
        <div>${formatLatest(latestBottom, "", 0)}</div>
    `;
    const labelTitle = escapeAttr(labelText);
    const visibleLabel = labelHtml ?? escapeHtml(labelText);
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${labelTitle}">
                <div class="fw-semibold text-truncate">${visibleLabel}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${qoqHeatmapRow(blocks, colorFn)}</div>
            <div class="text-muted small text-end exec-latest split">${formattedLatest}</div>
        </div>
    `;
}

function qoqHeatRow(label, badge, blocks, link = null) {
    const topValues = blocks?.upload_total || [];
    const bottomValues = blocks?.download_total || [];
    const latestTop = latestValue(topValues);
    const latestBottom = latestValue(bottomValues);
    const formattedLatest = `
        <div>${formatLatest(latestTop, "", 0)}</div>
        <div>${formatLatest(latestBottom, "", 0)}</div>
    `;
    const redactClass =
        badge === "Site" || badge === "Circuit" ? " redactable" : "";
    const labelMarkup = link ? `<a href="${link}">${label}</a>` : label;
    return `
        <div class="exec-heat-row">
            <div class="exec-heat-label text-truncate" title="${label}">
                <div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells">${qoqHeatmapRow(blocks, colorByQoqScore)}</div>
            <div class="text-muted small text-end exec-latest split">${formattedLatest}</div>
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
            { kind: "qoo", label: "Overall QoO", labelHtml: `Overall QoO${qooInfoIconHtml()}`, badge: "Global", blocks: globalQoq },
            { kind: "rtt", label: "RTT (p50/p90)", badge: "Global", blocks: global },
            { kind: "retransmit", label: "TCP Retransmits", badge: "Global", blocks: global, color: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))), format: (v) => formatLatest(v, "%", 1) },
            { kind: "utilization", label: "Utilization", badge: "Global", blocks: global, color: colorByCapacity, format: (v) => formatLatest(v, "%") },
        ];
        const body = rows
            .map((row) => {
                if (row.kind === "qoo") {
                    return qoqRow(row.label, row.badge, row.blocks, colorByQoqScore, row.labelHtml);
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
                if (row.kind === "retransmit") {
                    return retransmitHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        row.color,
                        row.format,
                    );
                }
                if (row.kind === "utilization") {
                    return utilizationHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        row.color,
                        row.format,
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
        initTooltipsWithin(target);
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
                if (this.config.metricKey === "retransmit") {
                    return retransmitHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        this.config.colorFn,
                        this.config.formatFn,
                        link
                    );
                }
                if (this.config.metricKey === "utilization") {
                    return utilizationHeatRow(
                        row.label,
                        row.badge,
                        row.blocks,
                        this.config.colorFn,
                        this.config.formatFn,
                        link
                    );
                }
                if (this.config.metricKey === "qoo") {
                    return qoqHeatRow(
                        row.label,
                        row.badge,
                        row.qoq_blocks,
                        link,
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
            const isQooMetric = this.config.metricKey === "qoo";
            const titleLabel = isQooMetric
                ? `QoO${qooInfoIconHtml()} Heatmap`
                : this.config.title;
            const titleHtml = this.config.link
                ? `<a class="text-decoration-none text-secondary" href="${this.config.link}"><i class="fas ${this.config.icon} me-2 text-primary"></i>${titleLabel}${linkIcon}</a>`
                : `<i class="fas ${this.config.icon} me-2 text-primary"></i>${titleLabel}`;
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
            initTooltipsWithin(activeTarget);
        });
    }

    metricSort(a, b) {
        const metricKey = this.config.metricKey;
        const isUtilization = metricKey === "utilization";
        const isQoo = metricKey === "qoo";
        const aDownVals = a.blocks.download || [];
        const aUpVals = a.blocks.upload || [];
        const bDownVals = b.blocks.download || [];
        const bUpVals = b.blocks.upload || [];

        const aVals = isUtilization ? [] : (a.blocks[metricKey] || []);
        const bVals = isUtilization ? [] : (b.blocks[metricKey] || []);

        const latestQoo = (blocks) => {
            if (!blocks) return null;
            const vals = [
                latestValue(blocks.download_total),
                latestValue(blocks.upload_total),
            ].filter((v) => v !== null && v !== undefined);
            if (!vals.length) return null;
            const sum = vals.reduce((x, y) => x + y, 0);
            return sum / vals.length;
        };
        const countQoo = (blocks) => {
            if (!blocks) return 0;
            return Math.max(
                nonNullCount(blocks.download_total),
                nonNullCount(blocks.upload_total),
            );
        };

        const aLatest = isQoo
            ? latestQoo(a.qoq_blocks)
            : isUtilization
            ? Math.max(latestValue(aDownVals) ?? -Infinity, latestValue(aUpVals) ?? -Infinity)
            : latestValue(aVals);
        const bLatest = isQoo
            ? latestQoo(b.qoq_blocks)
            : isUtilization
            ? Math.max(latestValue(bDownVals) ?? -Infinity, latestValue(bUpVals) ?? -Infinity)
            : latestValue(bVals);
        const aCount = isQoo
            ? countQoo(a.qoq_blocks)
            : isUtilization
            ? Math.max(nonNullCount(aDownVals), nonNullCount(aUpVals))
            : nonNullCount(aVals);
        const bCount = isQoo
            ? countQoo(b.qoq_blocks)
            : isUtilization
            ? Math.max(nonNullCount(bDownVals), nonNullCount(bUpVals))
            : nonNullCount(bVals);
        const countWeight = this.config.countWeight || 0;
        const minSamples = this.config.minSamples || 0;

        const score = (latest, count) => {
            if (latest === null || latest === undefined) return -Infinity;
            let base = latest;
            if (isQoo) {
                // QoO: higher is better; show worst first by inverting for sort score.
                base = -latest;
            }
            let s = base + countWeight * (count / 15);
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
            title: "Utilization",
            icon: "fa-chart-line",
            metricKey: "utilization",
            colorFn: colorByCapacity,
            formatFn: (v) => formatLatest(v, "%"),
            link: "executive_heatmap_download.html",
            hideAsns: true,
        });
    }
    title() { return "Utilization"; }
}

export class ExecutiveUploadHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            title: "QoO Heatmap",
            icon: "fa-bullseye",
            metricKey: "qoo",
            colorFn: colorByQoqScore,
            formatFn: (v) => formatLatest(v, "", 0),
            link: null,
            hideAsns: true,
            countWeight: 100,
            minSamples: 3,
        });
    }
    title() { return "QoO Heatmap"; }
}
