import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {colorByRetransmitPct, colorByRttMs} from "../helpers/color_scales";

const HELPER_LINKS = [
    { label: "Top 10 Worst Performing Sites", icon: "fa-temperature-high", href: "executive_worst_sites.html" },
    { label: "Top 10 Most Over-Subscribed Sites", icon: "fa-chart-line", href: "executive_oversubscribed_sites.html" },
    { label: "Sites Due for Upgrade", icon: "fa-arrow-up-right-dots", href: "executive_sites_due_upgrade.html" },
    { label: "Circuits Due for Upgrade", icon: "fa-wave-square", href: "executive_circuits_due_upgrade.html" },
    { label: "Top 10 ASNs by Traffic Volume", icon: "fa-globe-americas", href: "executive_top_asns.html" },
];
const MAX_SITE_ROWS = 8;
const MAX_CIRCUIT_ROWS = 20;
const MAX_ASN_ROWS = 6;
const MAX_TOTAL_ROWS = 20;

function formatCount(value) {
    if (value === undefined || value === null) return "—";
    const num = Number(value);
    if (!Number.isFinite(num)) return "—";
    return num.toLocaleString();
}

function clampPercent(value) {
    const num = Number(value) || 0;
    return Math.min(100, Math.max(0, num));
}

const colorByCapacity = (pct) => lerpGreenToRedViaOrange(100 - clampPercent(pct), 100);

function isIpLike(name) {
    if (!name) return false;
    // crude IPv4/IPv6 pattern check
    return /^[0-9a-fA-F:.]+$/.test(name) && (name.includes(".") || name.includes(":"));
}

export class ExecutiveSummaryStub extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.lastUpdate = null;
        this.latestThroughput = null;
        this._headerId = `${this.id}_header`;
        this._helpersId = `${this.id}_helpers`;
        this._heatmapId = `${this.id}_heatmaps`;
    }

    canBeSlowedDown() { return true; }
    title() { return "Executive Summary"; }
    tooltip() { return "Headline health metrics with helper shortcuts and 30s heatmap updates."; }
    subscribeTo() { return ["ExecutiveHeatmaps", "Throughput"]; }

    buildContainer() {
        const container = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("d-flex", "flex-column", "gap-3");

        const headerSection = document.createElement("div");
        headerSection.id = this._headerId;
        headerSection.innerHTML = this.#loadingRow("Collecting headline metrics…");
        wrap.appendChild(headerSection);

        const helperSection = document.createElement("div");
        helperSection.id = this._helpersId;
        helperSection.innerHTML = this.#loadingRow("Loading helper links…");
        wrap.appendChild(helperSection);

        const heatmapSection = document.createElement("div");
        heatmapSection.id = this._heatmapId;
        heatmapSection.innerHTML = this.#loadingRow("Preparing heatmap view…");
        wrap.appendChild(heatmapSection);

        container.appendChild(wrap);
        return container;
    }

    setup() {
        if (this.lastUpdate && window.executiveHeatmapData) {
            this.#render();
        }
    }

    onMessage(msg) {
        if (msg.event === "ExecutiveHeatmaps") {
            window.executiveHeatmapData = msg.data;
            this.lastUpdate = new Date();
            this.#render();
            return;
        }
        if (msg.event === "Throughput") {
            this.latestThroughput = msg.data;
            this.#render();
        }
    }

    #render() {
        const data = window.executiveHeatmapData || {};
        this.#renderHeader(data.header);
        this.#renderHelpers();
        this.#renderHeatmaps(data);
    }

    #renderHeader(header) {
        const target = document.getElementById(this._headerId);
        if (!target) return;
        if (!header) {
            target.innerHTML = this.#loadingRow("Waiting for executive summary data…");
            return;
        }

        target.innerHTML = `
            <div class="d-flex flex-wrap align-items-center justify-content-between gap-2 mb-2">
                <div class="exec-section-title mb-0 text-secondary"><i class="fas fa-chart-pie me-2 text-primary"></i>Network Snapshot</div>
            </div>
            <div class="row row-cols-1 row-cols-md-2 row-cols-lg-3 row-cols-xl-3 g-3">
                ${this.#renderInventoryCard(header)}
                ${this.#renderQueuesCard(header)}
                ${this.#renderThroughputCard()}
                ${this.#renderSimpleCard("Mapped IPs", "fa-map-marker-alt", "text-primary", formatCount(header.mapped_ip_count))}
                ${this.#renderSimpleCard("Unknown IPs", "fa-question-circle", "text-warning", formatCount(header.unmapped_ip_count))}
                ${this.#renderInsightCard(header)}
            </div>
        `;
    }

    #renderHelpers() {
        const target = document.getElementById(this._helpersId);
        if (!target) return;
        const buttons = HELPER_LINKS.map(link => `
            <a class="btn btn-outline-primary exec-helper-button" href="${link.href}" title="Open ${link.label}">
                <i class="fas ${link.icon} me-2"></i>${link.label}
            </a>
        `).join("");

        target.innerHTML = `
            <div class="card shadow-sm border-0">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0"><i class="fas fa-external-link-alt me-2 text-primary"></i>Helper Views</div>
                        <span class="badge bg-light text-secondary border">Navigation</span>
                    </div>
                    <div class="d-flex flex-wrap gap-2">${buttons}</div>
                </div>
            </div>
        `;
    }

    #renderHeatmaps(data) {
        const target = document.getElementById(this._heatmapId);
        if (!target) return;

        const rows = this.#buildHeatmapRows(data);
        const globalCard = this.#renderGlobalCard(data.global);

        const rttPanel = this.#renderMetricPanel(
            "Median RTT",
            "fa-stopwatch",
            rows,
            "rtt",
            (v) => colorByRttMs(v, 200),
            (v) => this.#formatLatest(v, "ms")
        );
        const retransPanel = this.#renderMetricPanel(
            "TCP Retransmits",
            "fa-undo-alt",
            rows,
            "retransmit",
            (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))),
            (v) => this.#formatLatest(v, "%", 1)
        );
        const downloadPanel = this.#renderMetricPanel(
            "Download Utilization",
            "fa-arrow-down",
            rows,
            "download",
            colorByCapacity,
            (v) => this.#formatLatest(v, "%")
        );
        const uploadPanel = this.#renderMetricPanel(
            "Upload Utilization",
            "fa-arrow-up",
            rows,
            "upload",
            colorByCapacity,
            (v) => this.#formatLatest(v, "%")
        );

        target.innerHTML = `
            ${globalCard}
            <div class="row g-3 mt-1">
                <div class="col-md-6">${rttPanel}</div>
                <div class="col-md-6">${retransPanel}</div>
                <div class="col-md-6">${downloadPanel}</div>
                <div class="col-md-6">${uploadPanel}</div>
            </div>
        `;
    }

    #renderGlobalCard(global) {
        if (!global) {
            return `
                <div class="card shadow-sm border-0">
                    <div class="card-body py-3 text-muted small">No heatmap data yet.</div>
                </div>
            `;
        }
        const rows = [
            { label: "Median RTT", badge: "Global", values: global.rtt || [], color: (v) => colorByRttMs(v, 200), format: (v) => this.#formatLatest(v, "ms") },
            { label: "TCP Retransmits", badge: "Global", values: global.retransmit || [], color: (v) => colorByRetransmitPct(Math.min(10, Math.max(0, v || 0))), format: (v) => this.#formatLatest(v, "%", 1) },
            { label: "Download Utilization", badge: "Global", values: global.download || [], color: colorByCapacity, format: (v) => this.#formatLatest(v, "%") },
            { label: "Upload Utilization", badge: "Global", values: global.upload || [], color: colorByCapacity, format: (v) => this.#formatLatest(v, "%") },
        ];
        const body = rows.map(row => this.#heatRow(row.label, row.badge, row.values, row.color, row.format)).join("");
        return `
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

    #renderMetricPanel(title, icon, rows, metricKey, colorFn, formatLatest) {
        if (!rows.length) {
            return `
                <div class="card shadow-sm border-0 h-100">
                    <div class="card-body py-3 text-muted small">No heatmap data yet.</div>
                </div>
            `;
        }
        const body = rows.map(row => this.#heatRow(
            row.label,
            row.badge,
            row.blocks[metricKey] || [],
            colorFn,
            formatLatest
        )).join("");
        return `
            <div class="card shadow-sm border-0 h-100">
                <div class="card-body py-3">
                    <div class="d-flex align-items-center justify-content-between flex-wrap gap-2 mb-2">
                        <div class="exec-section-title mb-0"><i class="fas ${icon} me-2 text-primary"></i>${title}</div>
                        <span class="badge bg-light text-secondary border">${rows.length} rows</span>
                    </div>
                    <div class="exec-heat-rows">${body}</div>
                </div>
            </div>
        `;
    }

    #heatRow(label, badge, values, colorFn, formatLatest) {
        const latest = this.#latestValue(values);
        const formattedLatest = formatLatest(latest);
        return `
            <div class="exec-heat-row">
                <div class="exec-heat-label text-truncate" title="${label}">
                    <div class="fw-semibold text-truncate">${label}</div>
                    ${badge ? `<span class="badge bg-light text-secondary border">${badge}</span>` : ""}
                </div>
                <div class="exec-heat-cells">${this.#heatmapRow(values, colorFn, formatLatest)}</div>
                <div class="text-muted small text-end exec-latest">${formattedLatest}</div>
            </div>
        `;
    }

    #heatmapRow(values, colorFn, formatLatest) {
        const length = Array.isArray(values) && values.length ? values.length : 15;
        let cells = "";
        for (let i = 0; i < length; i++) {
            const val = values && values[i] !== undefined ? values[i] : null;
            if (val === null || val === undefined) {
                cells += `<div class="exec-heat-cell empty" title="No data"></div>`;
                continue;
            }
            const numeric = Number(val) || 0;
            const color = colorFn(numeric);
            const title = formatLatest(numeric);
            cells += `<div class="exec-heat-cell" style="background:${color}" title="Block ${i + 1}: ${title}"></div>`;
        }
        return cells;
    }

    #latestValue(values) {
        if (!values || !values.length) return null;
        for (let i = values.length - 1; i >= 0; i--) {
            const val = values[i];
            if (val !== null && val !== undefined) {
                const num = Number(val);
                if (Number.isFinite(num)) {
                    return num;
                }
            }
        }
        return null;
    }

    #formatLatest(value, unit = "", precision = 0) {
        if (value === null || value === undefined || Number.isNaN(value)) return "—";
        const suffix = unit ? ` ${unit}` : "";
        if (precision === 0) {
            return `${Math.round(value)}${suffix}`;
        }
        return `${value.toFixed(precision)}${suffix}`;
    }

    #buildHeatmapRows(data) {
        const rows = [];
        const sites = (data.sites || [])
            .filter(site => site.blocks)
            .filter(site => !isIpLike(site.site_name))
            .filter(site => site.depth === undefined || site.depth <= 2)
            .filter(site => {
                const t = (site.node_type || "").toLowerCase();
                return t === "site" || t === "ap" || t === "";
            });
        sites.forEach(site => rows.push({
            label: site.site_name || "Site",
            badge: "Site",
            blocks: site.blocks,
        }));
        const circuits = (data.circuits || []);
        circuits.forEach(circuit => {
            const name = circuit.circuit_name || circuit.circuit_id || `Circuit ${circuit.circuit_hash}`;
            rows.push({
                label: name,
                badge: "Circuit",
                blocks: circuit.blocks,
            });
        });
        const asns = (data.asns || []);
        asns.forEach(asn => rows.push({
            label: `ASN ${asn.asn}`,
            badge: "ASN",
            blocks: asn.blocks,
        }));

        // Sort by most active based on non-empty blocks; fall back to label
        rows.sort((a, b) => {
            const aScore = this.#rowScore(a);
            const bScore = this.#rowScore(b);
            if (bScore !== aScore) return bScore - aScore;
            return (a.label || "").localeCompare(b.label || "");
        });
        return rows.slice(0, MAX_TOTAL_ROWS);
    }

    #rowScore(row) {
        if (!row || !row.blocks) return 0;
        const latest = (arr) => {
            if (!arr || !arr.length) return null;
            for (let i = arr.length - 1; i >= 0; i--) {
                const v = arr[i];
                if (v !== null && v !== undefined && Number.isFinite(Number(v))) return Number(v);
            }
            return null;
        };
        const d = latest(row.blocks.download);
        const u = latest(row.blocks.upload);
        const rtt = latest(row.blocks.rtt);
        const retr = latest(row.blocks.retransmit);
        const util = Math.max(d || 0, u || 0);
        const latencyScore = rtt !== null ? (200 - Math.min(200, rtt)) / 200 * 5 : 0;
        const retrScore = retr !== null ? retr : 0;
        // prioritize any non-empty rows; if all zero/blank, score stays low
        const hasData = (util > 0 || rtt !== null || retr !== null) ? 1 : 0;
        return util + latencyScore + retrScore * 0.5 + hasData;
    }

    #renderInventoryCard(header) {
        const items = [
            { label: "Circuits", value: formatCount(header.circuit_count) },
            { label: "Devices", value: formatCount(header.device_count) },
            { label: "Sites", value: formatCount(header.site_count) },
        ];
        return this.#groupCard("Inventory", "fa-layer-group", "text-primary", items);
    }

    #renderQueuesCard(header) {
        const items = [
            { label: "HTB", value: formatCount(header.htb_queue_count) },
            { label: "CAKE", value: formatCount(header.cake_queue_count) },
        ];
        return this.#groupCard("Queues", "fa-stream", "text-secondary", items);
    }

    #renderThroughputCard() {
        const bps = this.latestThroughput?.bps;
        const down = this.#formatBps(bps?.down);
        const up = this.#formatBps(bps?.up);
        const items = [
            { label: `<i class="fas fa-arrow-down"></i>`, value: down },
            { label: `<i class="fas fa-arrow-up"></i>`, value: up },
        ];
        return this.#groupCard("Throughput", "fa-tachometer-alt", "text-info", items, true);
    }

    #renderSimpleCard(label, icon, accent, value) {
        return `
            <div class="col">
                <div class="executive-card h-100">
                    <div class="d-flex align-items-center gap-3">
                        <span class="exec-icon ${accent}"><i class="fas ${icon}"></i></span>
                        <div>
                            <div class="text-secondary small">${label}</div>
                            <div class="exec-metric-value text-secondary">${value}</div>
                        </div>
                    </div>
                </div>
            </div>
        `;
    }

    #renderInsightCard(header) {
        const value = header.insight_connected;
        const badge = value === undefined
            ? `<span class="badge bg-light text-secondary border exec-badge">Pending</span>`
            : value
                ? `<span class="badge bg-success-subtle text-success exec-badge">Connected</span>`
                : `<span class="badge bg-danger-subtle text-danger exec-badge">Offline</span>`;
        return `
            <div class="col">
                <div class="executive-card h-100">
                    <div class="d-flex align-items-center gap-3">
                        <span class="exec-icon ${value ? "text-success" : "text-danger"}"><i class="fas fa-satellite-dish"></i></span>
                        <div>
                            <div class="text-secondary small">Insight</div>
                            <div class="exec-metric-value">${badge}</div>
                        </div>
                    </div>
                </div>
            </div>
        `;
    }

    #groupCard(title, icon, accent, items, allowHtmlLabel = false) {
        const rows = items.map(item => `
            <div class="d-flex align-items-baseline gap-1">
                <span class="text-secondary small">${allowHtmlLabel ? item.label : this.#escapeHtml(item.label)}</span>
                <span class="exec-metric-value text-secondary">${item.value}</span>
            </div>
        `).join("");
        return `
            <div class="col">
                <div class="executive-card h-100">
                    <div class="d-flex align-items-start gap-3">
                        <span class="exec-icon ${accent}"><i class="fas ${icon}"></i></span>
                        <div class="flex-grow-1">
                            <div class="text-secondary small">${title}</div>
                            <div class="d-flex flex-wrap gap-3 mt-2">${rows}</div>
                        </div>
                    </div>
                </div>
            </div>
        `;
    }

    #formatBps(bits) {
        if (bits === undefined || bits === null || Number.isNaN(bits)) return "—";
        const scaled = scaleNumber(bits, 1);
        return `${scaled}bps`;
    }

    #escapeHtml(str) {
        return String(str)
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;")
            .replace(/'/g, "&#039;");
    }

    #loadingRow(msg) {
        return `<div class="text-muted small">${msg}</div>`;
    }
}
