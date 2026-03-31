import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {StormguardAdjustmentsGraph} from "../graphs/stormguard_adjustments_graph";
import {mkBadge} from "./bakery_shared";
import {
    directionReason,
    formatStormguardAgeSeconds,
    formatStormguardMbps,
    formatStormguardMs,
    formatStormguardPercent,
    stormguardSelectedSite,
    subscribeStormguardState,
    updateStormguardDebug,
    updateStormguardStatus,
} from "./stormguard_shared";

function metricRow(label, value) {
    return `
        <tr>
            <td class="table-label-cell">${label}</td>
            <td class="table-value-cell">${value}</td>
        </tr>
    `;
}

function summaryBadge(summary) {
    return mkBadge(summary.label, summary.className, summary.reason || "");
}

function renderDirectionTable(direction, fallbackRatesLabel) {
    if (!direction) {
        return `
            <div class="small text-muted">
                No live debug metrics yet. Current enforced rate: ${fallbackRatesLabel}.
            </div>
        `;
    }

    return `
        <div class="table-responsive lqos-table-wrap">
            <table class="lqos-table lqos-table-compact mb-0">
                <tbody>
                    ${metricRow("Queue", `${formatStormguardMbps(direction.queue_mbps)} Mbps`)}
                    ${metricRow("Min / Max", `${formatStormguardMbps(direction.min_mbps)} / ${formatStormguardMbps(direction.max_mbps)} Mbps`)}
                    ${metricRow("State", direction.state || "—")}
                    ${metricRow("Last Action", direction.last_action || "—")}
                    ${metricRow("Action Age", formatStormguardAgeSeconds(direction.last_action_age_secs))}
                    ${metricRow("Cooldown", direction.cooldown_remaining_secs != null ? formatStormguardAgeSeconds(direction.cooldown_remaining_secs) : "—")}
                    ${metricRow("Throughput", `${formatStormguardMbps(direction.throughput_mbps)} / ${formatStormguardMbps(direction.throughput_ma_mbps)} Mbps`)}
                    ${metricRow("Retrans", `${formatStormguardPercent(direction.retrans)} / ${formatStormguardPercent(direction.retrans_ma)}`)}
                    ${metricRow("RTT", `${formatStormguardMs(direction.rtt)} / ${formatStormguardMs(direction.rtt_ma)}`)}
                    ${metricRow("Baseline / Delay", `${formatStormguardMs(direction.baseline_rtt_ms)} / ${formatStormguardMs(direction.delay_ms)}`)}
                    ${metricRow("Saturation", `${direction.saturation_current || "—"} / ${direction.saturation_max || "—"}`)}
                    ${metricRow("Can +/-", `${direction.can_increase ? "Yes" : "No"} / ${direction.can_decrease ? "Yes" : "No"}`)}
                </tbody>
            </table>
        </div>
    `;
}

export class StormguardSiteDetailDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 8;
        this.unsubscribe = null;
        this.currentSiteName = null;
    }

    title() {
        return "StormGuard Site Detail";
    }

    tooltip() {
        return "<h5>StormGuard Site Detail</h5><p>Explains the selected site’s current enforced limits, last actions, cooldown state, and the signals StormGuard is evaluating in each direction.</p>";
    }

    subscribeTo() {
        return ["StormguardStatus", "StormguardDebug"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.header = document.createElement("div");
        this.header.classList.add("d-flex", "justify-content-between", "align-items-start", "flex-wrap", "gap-2", "mb-3");

        this.titleEl = document.createElement("div");
        this.titleEl.classList.add("fw-semibold");

        this.badgesEl = document.createElement("div");
        this.badgesEl.classList.add("d-flex", "gap-2", "flex-wrap");

        this.header.appendChild(this.titleEl);
        this.header.appendChild(this.badgesEl);
        wrap.appendChild(this.header);

        const ratesRow = document.createElement("div");
        ratesRow.classList.add("row", "g-3", "mb-3");

        this.downCard = document.createElement("div");
        this.downCard.classList.add("col-md-6");
        this.upCard = document.createElement("div");
        this.upCard.classList.add("col-md-6");

        ratesRow.appendChild(this.downCard);
        ratesRow.appendChild(this.upCard);
        wrap.appendChild(ratesRow);

        this.graphEl = document.createElement("div");
        this.graphEl.id = this.graphDivId();
        this.graphEl.classList.add("dashgraph");
        wrap.appendChild(this.graphEl);

        const detailRow = document.createElement("div");
        detailRow.classList.add("row", "g-3", "mt-1");
        this.downDetail = document.createElement("div");
        this.downDetail.classList.add("col-lg-6");
        this.upDetail = document.createElement("div");
        this.upDetail.classList.add("col-lg-6");
        detailRow.appendChild(this.downDetail);
        detailRow.appendChild(this.upDetail);
        wrap.appendChild(detailRow);

        this.emptyEl = document.createElement("div");
        this.emptyEl.classList.add("small", "text-muted", "mt-3");
        wrap.appendChild(this.emptyEl);

        base.appendChild(wrap);
        return base;
    }

    setup() {
        this.graph = new StormguardAdjustmentsGraph(this.graphDivId());
        this.unsubscribe = subscribeStormguardState((snapshot) => this.renderSnapshot(snapshot));
    }

    onMessage(msg) {
        if (msg.event === "StormguardStatus") {
            updateStormguardStatus(msg.data || []);
        }
        if (msg.event === "StormguardDebug") {
            updateStormguardDebug(msg.data || []);
        }
    }

    renderSnapshot(snapshot) {
        const site = stormguardSelectedSite(snapshot);
        if (!site) {
            this.titleEl.textContent = "No StormGuard site selected";
            this.badgesEl.replaceChildren();
            this.downCard.innerHTML = "";
            this.upCard.innerHTML = "";
            this.downDetail.innerHTML = "";
            this.upDetail.innerHTML = "";
            this.graph.update([]);
            this.emptyEl.textContent = "When StormGuard begins watching sites, this panel will explain the selected site’s live decisions.";
            return;
        }

        this.currentSiteName = site.site;
        this.titleEl.textContent = site.site;
        this.badgesEl.replaceChildren(
            summaryBadge(site.downloadSummary),
            summaryBadge(site.uploadSummary),
            mkBadge(site.inCooldown ? "Cooldown Active" : "No Cooldown", site.inCooldown ? "bg-warning-subtle text-warning border border-warning-subtle" : "bg-light text-secondary border"),
        );

        this.downCard.innerHTML = `
            <div class="border rounded p-3 h-100">
                <div class="text-muted small text-uppercase mb-1"><i class="fa fa-arrow-down text-primary me-1"></i> Download</div>
                <div class="fs-4 fw-semibold">${formatStormguardMbps(site.currentDownMbps)} Mbps</div>
                <div class="small text-muted mt-1">${directionReason(site.download)}</div>
            </div>
        `;
        this.upCard.innerHTML = `
            <div class="border rounded p-3 h-100">
                <div class="text-muted small text-uppercase mb-1"><i class="fa fa-arrow-up text-success me-1"></i> Upload</div>
                <div class="fs-4 fw-semibold">${formatStormguardMbps(site.currentUpMbps)} Mbps</div>
                <div class="small text-muted mt-1">${directionReason(site.upload)}</div>
            </div>
        `;

        this.downDetail.innerHTML = `
            <div class="border rounded p-3 h-100">
                <div class="d-flex justify-content-between align-items-center mb-2">
                    <h6 class="mb-0 text-secondary">Download evaluation</h6>
                    ${summaryBadge(site.downloadSummary).outerHTML}
                </div>
                ${renderDirectionTable(site.download, `${formatStormguardMbps(site.currentDownMbps)} Mbps`)}
            </div>
        `;
        this.upDetail.innerHTML = `
            <div class="border rounded p-3 h-100">
                <div class="d-flex justify-content-between align-items-center mb-2">
                    <h6 class="mb-0 text-secondary">Upload evaluation</h6>
                    ${summaryBadge(site.uploadSummary).outerHTML}
                </div>
                ${renderDirectionTable(site.upload, `${formatStormguardMbps(site.currentUpMbps)} Mbps`)}
            </div>
        `;

        this.graph.update([[site.site, site.currentDownMbps ?? 0, site.currentUpMbps ?? 0]]);
        this.emptyEl.textContent = snapshot.singleSite
            ? "Single-site deployment: this panel is the primary StormGuard view."
            : "Use the site list to switch focus. The graph here follows the selected site only.";
    }
}
