import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {formatCount} from "./executive_heatmap_shared";

const SNAPSHOT_HELPER = {
    loading(msg) {
        return `<div class="text-muted small">${msg}</div>`;
    },
};

export class ExecutiveSnapshotDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.latestThroughput = null;
    }

    canBeSlowedDown() { return true; }
    title() { return "Network Snapshot"; }
    tooltip() { return "Headline health metrics for the executive view."; }
    subscribeTo() { return ["ExecutiveHeatmaps", "Throughput"]; }

    buildContainer() {
        const container = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("d-flex", "flex-column", "gap-3");

        this._contentId = `${this.id}_snapshot`;
        const snapshot = document.createElement("div");
        snapshot.id = this._contentId;
        snapshot.innerHTML = SNAPSHOT_HELPER.loading("Collecting headline metrics…");
        wrap.appendChild(snapshot);

        container.appendChild(wrap);
        return container;
    }

    setup() {
        if (window.executiveHeatmapData) {
            this.render();
        }
    }

    onMessage(msg) {
        if (msg.event === "ExecutiveHeatmaps") {
            window.executiveHeatmapData = msg.data;
            this.render();
            return;
        }
        if (msg.event === "Throughput") {
            this.latestThroughput = msg.data;
            this.render();
        }
    }

    render() {
        const header = window.executiveHeatmapData?.header;
        const target = document.getElementById(this._contentId);
        if (!target) return;
        if (!header) {
            target.innerHTML = SNAPSHOT_HELPER.loading("Waiting for executive summary data…");
            return;
        }

        target.innerHTML = `
            <div class="d-flex flex-wrap align-items-center justify-content-between gap-2 mb-2">
                <div class="exec-section-title mb-0 text-secondary"><i class="fas fa-chart-pie me-2 text-primary"></i>Network Snapshot</div>
            </div>
            <div class="row row-cols-1 row-cols-md-2 row-cols-lg-3 row-cols-xl-3 g-3">
                ${this.inventoryCard(header)}
                ${this.queuesCard(header)}
                ${this.throughputCard()}
                ${this.simpleCard("Mapped IPs", "fa-map-marker-alt", "text-primary", formatCount(header.mapped_ip_count))}
                ${this.simpleCard("Unknown IPs", "fa-question-circle", "text-warning", formatCount(header.unmapped_ip_count))}
                ${this.insightCard(header)}
            </div>
        `;
    }

    inventoryCard(header) {
        const items = [
            { label: "Circuits", value: formatCount(header.circuit_count) },
            { label: "Devices", value: formatCount(header.device_count) },
            { label: "Sites", value: formatCount(header.site_count) },
        ];
        return this.groupCard("Inventory", "fa-layer-group", "text-primary", items);
    }

    queuesCard(header) {
        const items = [
            { label: "HTB", value: formatCount(header.htb_queue_count) },
            { label: "CAKE", value: formatCount(header.cake_queue_count) },
        ];
        return this.groupCard("Queues", "fa-stream", "text-secondary", items, false, true);
    }

    throughputCard() {
        const bps = this.latestThroughput?.bps;
        const items = [
            { label: `<i class="fas fa-arrow-down"></i>`, value: this.formatBps(bps?.down) },
            { label: `<i class="fas fa-arrow-up"></i>`, value: this.formatBps(bps?.up) },
        ];
        return this.groupCard("Throughput", "fa-tachometer-alt", "text-info", items, true);
    }

    simpleCard(label, icon, accent, value) {
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

    insightCard(header) {
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
                            <div class="exec-metric-value text-secondary">${badge}</div>
                        </div>
                    </div>
                </div>
            </div>
        `;
    }

    groupCard(title, icon, accent, items, allowHtmlLabel = false, allowAlerts = false) {
        const hasZero = allowAlerts && items.some(item => this.isZero(item.value));
        const rows = items.map((item, idx) => `
            <div class="d-flex align-items-baseline gap-1">
                <span class="text-secondary small">${allowHtmlLabel ? item.label : this.escapeHtml(item.label)}</span>
                ${this.renderValueWithAlert(item.value, hasZero && idx === 0)}
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

    renderValueWithAlert(value, showAlert) {
        const isZero = this.isZero(value);
        const valueClass = isZero ? "text-danger" : "text-secondary";
        if (!showAlert || !isZero) {
            return `<span class="exec-metric-value ${valueClass}">${value}</span>`;
        }
        const alertTitle = "No active queues - click 'Reload LibreQoS'";
        return `
            <span class="exec-metric-value ${valueClass}">${value}</span>
            <span class="ms-2 text-danger small d-inline-flex align-items-center" title="${alertTitle}" aria-label="${alertTitle}">
                <i class="fas fa-exclamation-triangle me-1"></i>Reload LibreQoS
            </span>
        `;
    }

    isZero(value) {
        if (value === "0" || value === 0) return true;
        const num = Number(value);
        return Number.isFinite(num) && num === 0;
    }

    formatBps(bits) {
        if (bits === undefined || bits === null || Number.isNaN(bits)) return "—";
        const scaled = scaleNumber(bits, 1);
        return `${scaled}bps`;
    }

    escapeHtml(str) {
        return String(str)
            .replace(/&/g, "&amp;")
            .replace(/</g, "&lt;")
            .replace(/>/g, "&gt;")
            .replace(/"/g, "&quot;")
            .replace(/'/g, "&#039;");
    }
}
