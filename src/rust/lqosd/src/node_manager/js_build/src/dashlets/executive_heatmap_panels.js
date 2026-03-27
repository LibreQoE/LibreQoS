import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {disposeTooltipsWithin, enableTooltipsWithin} from "../lq_js_common/helpers/tooltips";
import {colorByQoqScore, colorByRetransmitPct, colorByRttMs} from "../helpers/color_scales";
import {badgeForEntityKind, linkToExecutiveMetricRow} from "../executive_utils";
import {
    colorByCapacity,
    formatLatest,
    latestValue,
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
            cells += `<div class="exec-heat-cell empty" title="No data" aria-label="Block ${i + 1}: no data"></div>`;
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
            <div class="exec-heat-cell split" title="${escapeAttr(title)}" aria-label="${escapeAttr(title)}">
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
    const latestLabel = `${formatLatest(latestTop, "", 0)} / ${formatLatest(latestBottom, "", 0)}`;
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeAttr(`${labelText} ${badge || ""} latest ${latestLabel}`)}">
            <div class="exec-heat-label text-truncate" title="${labelTitle}">
                <div class="fw-semibold text-truncate">${visibleLabel}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeAttr(`${labelText} heatmap history`)}">${qoqHeatmapRow(blocks, colorFn)}</div>
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
    const latestLabel = `${formatLatest(latestTop, "", 0)} / ${formatLatest(latestBottom, "", 0)}`;
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeAttr(`${label} ${badge || ""} latest ${latestLabel}`)}">
            <div class="exec-heat-label text-truncate" title="${escapeAttr(label)}">
                <div class="fw-semibold text-truncate${redactClass}">${labelMarkup}</div>
                ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeAttr(`${label} heatmap history`)}">${qoqHeatmapRow(blocks, colorByQoqScore)}</div>
            <div class="text-muted small text-end exec-latest split">${formattedLatest}</div>
        </div>
    `;
}

function executiveBadge(row) {
    return badgeForEntityKind(row?.entity_kind);
}

function executiveSplitBlocks(row) {
    return {
        download: row?.split_blocks?.download || [],
        upload: row?.split_blocks?.upload || [],
    };
}

function executiveQoqBlocks(row) {
    return {
        download_total: row?.split_blocks?.download || [],
        upload_total: row?.split_blocks?.upload || [],
    };
}

function executiveRttBlocks(row) {
    return {
        rtt: row?.rtt_blocks?.rtt || [],
        rtt_p50_down: row?.rtt_blocks?.dl_p50 || [],
        rtt_p90_down: row?.rtt_blocks?.dl_p90 || [],
        rtt_p50_up: row?.rtt_blocks?.ul_p50 || [],
        rtt_p90_up: row?.rtt_blocks?.ul_p90 || [],
    };
}

function executiveRetransmitBlocks(row) {
    return {
        retransmit: row?.scalar_blocks?.values || [],
        retransmit_down: row?.split_blocks?.download || [],
        retransmit_up: row?.split_blocks?.upload || [],
    };
}

function latestUtilization(row) {
    const split = executiveSplitBlocks(row);
    return Math.max(
        latestValue(split.download) ?? Number.NEGATIVE_INFINITY,
        latestValue(split.upload) ?? Number.NEGATIVE_INFINITY,
    );
}

function mergedUtilizationRows(summary) {
    const byKey = new Map();
    [...(summary?.top_download || []), ...(summary?.top_upload || [])].forEach((row) => {
        if (!row?.row_key || byKey.has(row.row_key)) {
            return;
        }
        byKey.set(row.row_key, row);
    });
    return [...byKey.values()]
        .sort((left, right) => {
            const latestDiff = latestUtilization(right) - latestUtilization(left);
            if (Number.isFinite(latestDiff) && latestDiff !== 0) {
                return latestDiff;
            }
            return String(left?.label || "").localeCompare(String(right?.label || ""));
        })
        .slice(0, MAX_HEATMAP_ROWS);
}

class ExecutiveHeatmapBase extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 6;
        this.lastData = null;
        this._colorBlindListener = () => {
            if (this.lastData) {
                this.render();
            }
        };
        this._colorBlindBound = false;
    }

    canBeSlowedDown() { return true; }
    subscribeTo() { return ["ExecutiveDashboardSummary"]; }

    onMessage(msg) {
        if (msg.event === "ExecutiveDashboardSummary") {
            this.lastData = msg.data;
            window.executiveDashboardSummary = msg.data;
            this.render();
        }
    }

    setup() {
        if (!this._colorBlindBound) {
            window.addEventListener("colorBlindModeChanged", this._colorBlindListener);
            this._colorBlindBound = true;
        }
        if (window.executiveDashboardSummary) {
            this.lastData = window.executiveDashboardSummary;
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
                        (v) => colorByRttMs(v),
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
        disposeTooltipsWithin(target);
        target.innerHTML = `
                <div class="card shadow-sm border-0">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0"><i class="fas fa-thermometer-half me-2 text-warning"></i>Global Heatmap</div>
                        <span class="text-muted small">15 minutes, newest on the right</span>
                    </div>
                    <div class="exec-heat-rows" role="list">${body}</div>
                </div>
            </div>
        `;
        enableTooltipsWithin(target);
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
        const summary = this.lastData || {};
        const rows = this.summaryRows(summary);
        if (!rows.length) {
            target.innerHTML = this.emptyCard();
            return;
        }
        const activeTarget = document.getElementById(this._contentId);
        if (!activeTarget) return;
        const metricRows = rows.map((row) => {
            const badge = executiveBadge(row);
            const link = linkToExecutiveMetricRow(row);
            if (this.config.metricKey === "rtt") {
                return rttHeatRow(
                    row.label,
                    badge,
                    executiveRttBlocks(row),
                    this.config.colorFn,
                    this.config.formatFn,
                    link,
                );
            }
            if (this.config.metricKey === "retransmit") {
                return retransmitHeatRow(
                    row.label,
                    badge,
                    executiveRetransmitBlocks(row),
                    this.config.colorFn,
                    this.config.formatFn,
                    link,
                );
            }
            if (this.config.metricKey === "utilization") {
                return utilizationHeatRow(
                    row.label,
                    badge,
                    executiveSplitBlocks(row),
                    this.config.colorFn,
                    this.config.formatFn,
                    link,
                );
            }
            if (this.config.metricKey === "qoo") {
                return qoqHeatRow(
                    row.label,
                    badge,
                    executiveQoqBlocks(row),
                    link,
                );
            }
            return heatRow(
                row.label,
                badge,
                row?.scalar_blocks?.values || [],
                this.config.colorFn,
                this.config.formatFn,
                link,
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
        disposeTooltipsWithin(activeTarget);
        activeTarget.innerHTML = `
            <div class="card shadow-sm border-0 h-100">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0">${titleHtml}</div>
                    </div>
                    <div class="exec-heat-rows" role="list">${metricRows}</div>
                </div>
            </div>
        `;
        enableTooltipsWithin(activeTarget);
    }

    summaryRows(summary) {
        switch (this.config.metricKey) {
        case "rtt":
            return (summary?.top_rtt || []).slice(0, MAX_HEATMAP_ROWS);
        case "retransmit":
            return (summary?.top_retransmit || []).slice(0, MAX_HEATMAP_ROWS);
        case "utilization":
            return mergedUtilizationRows(summary);
        case "qoo":
            return (summary?.top_qoo || []).slice(0, MAX_HEATMAP_ROWS);
        default:
            return [];
        }
    }
}

export class ExecutiveRttHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            title: "Median RTT",
            icon: "fa-stopwatch",
            metricKey: "rtt",
            colorFn: (v) => colorByRttMs(v),
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
