import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {disposeTooltipsWithin, enableTooltipsWithin} from "../lq_js_common/helpers/tooltips";
import {colorByQoqScore, colorByRetransmitPct, colorByRttMs} from "../helpers/color_scales";
import {
    badgeForEntityKind,
    heatRowAriaLabel,
    heatRowsCaption,
    linkToExecutiveMetricRow,
    typeMonoMarkup,
} from "../executive_utils";
import {
    colorByCapacity,
    formatLatest,
    latestValue,
    heatRow,
    rttHeatRow,
    retransmitHeatRow,
    utilizationHeatRow,
    MAX_HEATMAP_ROWS,
    replaceHeatmapHtml,
    tooltipAttrsFromLines,
} from "./executive_heatmap_shared";
import {t, tAttr} from "../i18n/index.js";

// The QoO definition ships as one tooltip: the standard's own sentence (its
// URL stays literal), then what LibreQoS computes.
function qooTooltipHtml() {
    return [
        t("shaped_devices.qoo.intro"),
        `https://datatracker.ietf.org/doc/draft-ietf-ippm-qoo/`,
        t("executive.panel.qoo_line"),
    ].join("<br>");
}

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
    const title = escapeAttr(qooTooltipHtml());
    return `<span class="ms-1 text-muted" role="button" tabindex="0" aria-label="${tAttr("executive.panel.qoo_aria")}" data-bs-toggle="tooltip" data-bs-placement="top" data-bs-html="true" title="${title}"><i class="fas fa-info-circle"></i></span>`;
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
            cells += `<div class="exec-heat-cell empty" ${tooltipAttrsFromLines([
                t("executive.heat.block", {index: i + 1}),
                t("common.state.no_data"),
                t("executive.heat.qoq_no_totals"),
            ])}></div>`;
            continue;
        }

        const tooltipLines = [
            t("executive.heat.block", {index: i + 1}),
            `${t("executive.heat.label.upload_score")}: ${fmt(ulTotal)}`,
            `${t("executive.heat.label.download_score")}: ${fmt(dlTotal)}`,
            t("executive.heat.qoq_meaning"),
        ];

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
            <div class="exec-heat-cell split" ${tooltipAttrsFromLines(tooltipLines)}>
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
        <span class="exec-latest-pair" title="${escapeAttr(`${t("common.table.upload")} / ${t("common.table.download")}`)}">
            <span>${formatLatest(latestTop, "", 0)}</span>
            <span class="exec-latest-sep">/</span>
            <span>${formatLatest(latestBottom, "", 0)}</span>
        </span>
    `;
    const labelTitle = escapeAttr(labelText);
    const visibleLabel = labelHtml ?? escapeHtml(labelText);
    const latestLabel = `${formatLatest(latestTop, "", 0)} / ${formatLatest(latestBottom, "", 0)}`;
    return `
        <div class="exec-heat-row" role="listitem" aria-label="${escapeAttr(heatRowAriaLabel(labelText, badge, latestLabel))}">
            <div class="exec-heat-label text-truncate" title="${labelTitle}">
                ${typeMonoMarkup(badge)}<div class="fw-semibold text-truncate">${visibleLabel}</div>
            </div>
            <div class="exec-heat-cells" role="img" aria-label="${escapeAttr(t("executive.heat.aria.history", {label: labelText}))}">${qoqHeatmapRow(blocks, colorFn)}</div>
            <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
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

function scopedUtilizationLists(summary, scope) {
    // `.64.3.2`: the dashboard has a Site/AP widget (entity_kind=Site) and a
    // Circuit widget (entity_kind=Circuit). The global lists are kept for other
    // consumers; the widgets read their kind-scoped lists from the summary.
    if (scope === "site") {
        return [summary?.top_download_sites || [], summary?.top_upload_sites || []];
    }
    if (scope === "circuit") {
        return [summary?.top_download_circuits || [], summary?.top_upload_circuits || []];
    }
    return [summary?.top_download || [], summary?.top_upload || []];
}

function mergedUtilizationRows(summary, scope) {
    const byKey = new Map();
    const [downloadRows, uploadRows] = scopedUtilizationLists(summary, scope);
    [...downloadRows, ...uploadRows].forEach((row) => {
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
    // translatable: false — "Global" "_global" "_metric" — the global rows' badge token and the element-id suffixes the dashlets append; the badge word is rendered as a label only where a kind names one
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
                <div class="card-body py-3 text-muted small">${t("executive.panel.no_data")}</div>
            </div>
        `;
    }
}

export class ExecutiveGlobalHeatmapDashlet extends ExecutiveHeatmapBase {
    title() { return t("dashlet.name.global_heatmap"); }
    tooltip() { return t("executive.panel.global_tooltip"); }

    // `.64.3.1` (A2): the Network-Snapshot strip (throughput counters + Nc/Nd/Ns +
    // the Insight badge) is REMOVED from the Global Heatmap; its down/up counters
    // move to the Last-30-Seconds-Traffic handle (A3). The Global Heatmap now
    // subscribes only to the dashboard summary.
    subscribeTo() { return ["ExecutiveDashboardSummary"]; }

    buildContainer() {
        const container = super.buildContainer();
        const handle = container.querySelector(".dashbox-title");
        if (handle) {
            const caption = document.createElement("span");
            caption.className = "exec-heat-caption ms-auto";
            caption.textContent = t("executive.panel.global_caption");
            handle.appendChild(caption);
        }
        this._contentId = `${this.id}_global`;
        const body = document.createElement("div");
        body.id = this._contentId;
        // `.64.3.1` (A4): a SHARED explicit body height so the Global dashbox and
        // the 30s Traffic dashbox have equal TOTAL heights (both = title + body).
        body.classList.add("exec-widget-body", "global-heat-body");
        container.appendChild(body);
        return container;
    }

    render() {
        const target = document.getElementById(this._contentId);
        if (!target) return;
        const global = this.lastData?.global;
        const globalQoq = this.lastData?.global_qoq;
        if (!global) {
            replaceHeatmapHtml(target, this.emptyCard(), (html) => {
                disposeTooltipsWithin(target);
                target.innerHTML = html;
                target.__lqosLastHeatmapHtml = html;
                enableTooltipsWithin(target);
            });
            return;
        }
        const rows = [
            { kind: "qoo", label: t("executive.panel.overall_qoo"), labelHtml: `${t("executive.panel.overall_qoo")}${qooInfoIconHtml()}`, badge: "Global", blocks: globalQoq },
            { kind: "rtt", label: t("executive.panel.rtt_label"), badge: "Global", blocks: global },
            { kind: "retransmit", label: t("common.table.tcp_retransmits"), badge: "Global", blocks: global, color: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))), format: (v) => formatLatest(v, "%", 1) },
            { kind: "utilization", label: t("circuit.queuing.utilization"), badge: "Global", blocks: global, color: colorByCapacity, format: (v) => formatLatest(v, "%") },
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
        const html = `
                <div class="card shadow-sm border-0">
                <div class="card-body py-2">
                    <div class="exec-heat-rows" role="list">${body}</div>
                </div>
            </div>
        `;
        replaceHeatmapHtml(target, html, (nextHtml) => {
            disposeTooltipsWithin(target);
            target.innerHTML = nextHtml;
            target.__lqosLastHeatmapHtml = nextHtml;
            enableTooltipsWithin(target);
        });
    }
}

class ExecutiveMetricHeatmapBase extends ExecutiveHeatmapBase {
    constructor(slot, config) {
        super(slot);
        this.config = config;
    }

    // `.64.2` (A/C, REQUIRED-1): the handle's info button is driven by
    // `tooltip()`; the metric panels previously rendered their info in the body
    // title. Own it here so the handle is the single chrome row.
    tooltip() {
        return this.config.tooltip || null;
    }

    buildContainer() {
        const container = super.buildContainer();
        this._contentId = `${this.id}_metric`;
        const body = document.createElement("div");
        body.id = this._contentId;
        body.className = "exec-metric-body";
        container.appendChild(body);
        // `.64.2` (A/C): the handle (`.dashbox-title`) is the SINGLE title row —
        // carry the icon + DERIVED caption + the info/zoom + pop-out there.
        // `.64.2.1` (R2.3/R2.4): restore the per-panel icon and order it
        // title (+icon) → caption adjacent, with the info/zoom/pop-out controls
        // grouped on the right. The controls group carries the `margin-left:auto`
        // (moved off `> span:first-of-type` in the CSS) so inserting the caption
        // can never make IT the floated element.
        const handle = container.querySelector(".dashbox-title");
        if (handle) {
            const titleText = handle.firstChild;
            if (this.config.icon) {
                const icon = document.createElement("i");
                icon.className = `fas ${this.config.icon} me-1 text-primary`;
                icon.setAttribute("aria-hidden", "true");
                handle.insertBefore(icon, handle.firstChild);
            }
            // Group base_dashlet's direct-child control spans (info, and zoom when
            // present) so the whole control group floats right together.
            const controls = document.createElement("span");
            controls.className = "dashbox-title-controls";
            Array.from(handle.children).forEach((child) => {
                if (child.tagName === "SPAN") {
                    controls.appendChild(child);
                }
            });
            this._captionEl = document.createElement("span");
            this._captionEl.className = "exec-heat-caption";
            if (titleText) {
                handle.insertBefore(this._captionEl, titleText.nextSibling);
            } else {
                handle.appendChild(this._captionEl);
            }
            if (this.config.link) {
                const link = document.createElement("a");
                link.href = this.config.link;
                link.className = "text-decoration-none";
                link.title = t("executive.panel.detail_page_link");
                link.setAttribute("aria-label", t("executive.panel.detail_page_aria", {title: this.title()}));
                link.innerHTML = `${t("executive.panel.view_all")} <i class="fas fa-chevron-right ms-1"></i>`;
                link.classList.add("exec-view-all");
                controls.appendChild(link);
            }
            handle.appendChild(controls);
        }
        return container;
    }

    render() {
        const target = document.getElementById(this._contentId);
        if (!target) return;
        const summary = this.lastData || {};
        const rows = this.summaryRows(summary);
        if (!rows.length) {
            replaceHeatmapHtml(target, this.emptyCard(), (html) => {
                disposeTooltipsWithin(target);
                target.innerHTML = html;
                target.__lqosLastHeatmapHtml = html;
                enableTooltipsWithin(target);
            });
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
                    null,
                    { p50Only: true },
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
            return heatRow(
                row.label,
                badge,
                row?.scalar_blocks?.values || [],
                this.config.colorFn,
                this.config.formatFn,
                link,
            );
        }).join("");
        const derivedCaption = heatRowsCaption(rows);
        if (this._captionEl) {
            const caption = this.config.caption || derivedCaption;
            this._captionEl.textContent = caption ? `— ${caption}` : "";
        }
        const html = `
            <div class="card shadow-sm border-0 h-100 exec-card-${this.config.metricKey}">
                <div class="card-body py-2">
                    <div class="exec-heat-rows" role="list">${metricRows}</div>
                </div>
            </div>
        `;
        replaceHeatmapHtml(activeTarget, html, (nextHtml) => {
            disposeTooltipsWithin(activeTarget);
            activeTarget.innerHTML = nextHtml;
            activeTarget.__lqosLastHeatmapHtml = nextHtml;
            enableTooltipsWithin(activeTarget);
        });
    }

    summaryRows(summary) {
        switch (this.config.metricKey) {
        case "rtt":
            return (summary?.top_rtt_sites || summary?.top_rtt || []).slice(0, MAX_HEATMAP_ROWS);
        case "retransmit": {
            const rows = (summary?.top_retransmit_sites || summary?.top_retransmit || []).slice();
            const maxLatest = (row) => {
                const up = latestValue(row?.split_blocks?.upload || []);
                const down = latestValue(row?.split_blocks?.download || []);
                return Math.max(
                    up ?? Number.NEGATIVE_INFINITY,
                    down ?? Number.NEGATIVE_INFINITY,
                );
            };
            rows.sort((a, b) => maxLatest(b) - maxLatest(a));
            return rows.slice(0, MAX_HEATMAP_ROWS);
        }
        case "utilization":
            return mergedUtilizationRows(summary, this.config.scope);
        default:
            return [];
        }
    }
}

export class ExecutiveRttHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            tooltip: t("executive.dashlet.rtt_tooltip"),
            icon: "fa-stopwatch",
            metricKey: "rtt",
            colorFn: (v) => colorByRttMs(v),
            formatFn: (v) => formatLatest(v, "ms"),
            link: "executive_heatmap_rtt.html",
            caption: t("executive.caption.sites_aps"),
        });
    }
    title() { return t("asn_analysis.metric.median_rtt"); }
}

export class ExecutiveRetransmitsHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            tooltip: t("executive.dashlet.retransmit_tooltip"),
            icon: "fa-undo-alt",
            metricKey: "retransmit",
            colorFn: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))),
            formatFn: (v) => formatLatest(v, "%", 1),
            link: "executive_heatmap_retransmit.html",
            caption: t("executive.caption.sites_aps"),
        });
    }
    title() { return t("common.table.tcp_retransmits"); }
}

export class ExecutiveDownloadHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            tooltip: t("executive.dashlet.circuit_util_tooltip"),
            icon: "fa-chart-line",
            metricKey: "utilization",
            scope: "circuit",
            colorFn: colorByCapacity,
            formatFn: (v) => formatLatest(v, "%"),
            link: "executive_heatmap_download.html",
            caption: t("executive.heat.kind.circuits"),
        });
    }
    title() { return t("executive.dashlet.title.circuit_utilization"); }
}

export class ExecutiveUploadHeatmapDashlet extends ExecutiveMetricHeatmapBase {
    constructor(slot) {
        super(slot, {
            tooltip: t("executive.dashlet.site_util_tooltip"),
            icon: "fa-tower-broadcast",
            metricKey: "utilization",
            scope: "site",
            colorFn: colorByCapacity,
            formatFn: (v) => formatLatest(v, "%"),
            link: "executive_heatmap_download.html",
            caption: t("executive.caption.sites_aps"),
        });
    }
    title() { return t("executive.dashlet.title.site_ap_utilization"); }
}
