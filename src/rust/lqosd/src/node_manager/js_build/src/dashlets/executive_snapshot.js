import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {formatCount} from "./executive_heatmap_shared";
import {t} from "../i18n/index.js";

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
    title() { return t("dashlet.name.network_snapshot"); }
    tooltip() { return t("executive.snapshot.tooltip"); }
    // translatable: false — "Throughput" is the websocket event name this dashlet subscribes to and matches, not a label
    subscribeTo() { return ["ExecutiveDashboardSummary", "Throughput"]; }

    // translatable: false — "_snapshot" is the element-id suffix this dashlet appends, not a label
    buildContainer() {
        const container = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("d-flex", "flex-column", "gap-3");

        this._contentId = `${this.id}_snapshot`;
        const snapshot = document.createElement("div");
        snapshot.id = this._contentId;
        snapshot.innerHTML = SNAPSHOT_HELPER.loading(t("executive.snapshot.collecting"));
        wrap.appendChild(snapshot);

        container.appendChild(wrap);
        return container;
    }

    setup() {
        if (window.executiveDashboardSummary) {
            this.render();
        }
    }

    onMessage(msg) {
        if (msg.event === "ExecutiveDashboardSummary") {
            window.executiveDashboardSummary = msg.data;
            this.render();
            return;
        }
        if (msg.event === "Throughput") {
            this.latestThroughput = msg.data;
            this.render();
        }
    }

    render() {
        const header = window.executiveDashboardSummary?.header;
        const target = document.getElementById(this._contentId);
        if (!target) return;
        if (!header) {
            target.innerHTML = SNAPSHOT_HELPER.loading(t("executive.page.waiting_executive"));
            return;
        }

        target.innerHTML = `
            <div class="row row-cols-1 row-cols-md-2 row-cols-lg-3 row-cols-xl-3 g-3">
                ${this.throughputCard()}
                ${this.inventoryCard(header)}
                ${this.insightCard(header)}
            </div>
        `;
    }

    inventoryCard(header) {
        const items = [
            { label: t("common.table.circuits"), value: formatCount(header.circuit_count) },
            { label: t("common.table.devices"), value: formatCount(header.device_count) },
            { label: t("common.table.sites"), value: formatCount(header.site_count) },
        ];
        return this.groupCard(t("executive.snapshot.inventory"), "fa-layer-group", "text-primary", items);
    }

    throughputCard() {
        const bps = this.latestThroughput?.bps;
        const items = [
            { label: `<i class="fas fa-arrow-down"></i>`, value: this.formatBps(bps?.down) },
            { label: `<i class="fas fa-arrow-up"></i>`, value: this.formatBps(bps?.up) },
        ];
        return this.groupCard(t("common.table.throughput"), "fa-tachometer-alt", "text-info", items, true);
    }

    insightCard(header) {
        const value = header.insight_connected;
        const badge = value === undefined
            ? `<span class="badge bg-light text-secondary border exec-badge">${t("common.state.pending")}</span>`
            : value
                ? `<span class="badge bg-success-subtle text-success exec-badge">${t("executive.snapshot.connected")}</span>`
                : `<span class="badge bg-danger-subtle text-danger exec-badge">${t("executive.snapshot.offline")}</span>`;
        return `
            <div class="col">
                <div class="executive-card h-100">
                    <div class="d-flex align-items-center gap-3">
                        <span class="exec-icon ${value ? "text-success" : "text-danger"}"><i class="fas fa-satellite-dish"></i></span>
                        <div>
                            <div class="text-secondary small">${t("config.lts.status.control_service")}</div>
                            <div class="exec-metric-value text-secondary">${badge}</div>
                        </div>
                    </div>
                </div>
            </div>
        `;
    }

    groupCard(title, icon, accent, items, allowHtmlLabel = false, allowAlerts = false, statusBadge = "", footer = "") {
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
                            <div class="d-flex flex-wrap align-items-center gap-2">
                                <div class="text-secondary small">${title}</div>
                                ${statusBadge}
                            </div>
                            <div class="d-flex flex-wrap gap-3 mt-2">${rows}</div>
                            ${footer}
                        </div>
                    </div>
                </div>
            </div>
        `;
    }

    renderValueWithAlert(value, showAlert) {
        const isZero = this.isZero(value);
        const valueClass = isZero && showAlert ? "text-danger" : "text-secondary";
        if (!showAlert || !isZero) {
            return `<span class="exec-metric-value ${valueClass}">${value}</span>`;
        }
        const alertTitle = t("executive.snapshot.no_queues");
        return `
            <span class="exec-metric-value ${valueClass}">${value}</span>
            <span class="ms-2 text-danger small d-inline-flex align-items-center" title="${alertTitle}" aria-label="${alertTitle}">
                <i class="fas fa-exclamation-triangle me-1"></i>${t("common.action.reload")}
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
