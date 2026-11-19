import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {mkBadge} from "./bakery_shared";
import {t} from "../i18n/index.js";
import {
    formatStormguardAgeSeconds,
    stormguardActivityRows,
    subscribeStormguardState,
    updateStormguardDebug,
    updateStormguardStatus,
} from "./stormguard_shared";

function activityBadge(row) {
    return mkBadge(row.summary.label, row.summary.className, row.summary.reason || "");
}

export class StormguardRecentActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.unsubscribe = null;
    }

    title() {
        return t("dashlet.name.stormguard_recent_activity");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.stormguard_activity.tooltip")}</p>`;
    }

    subscribeTo() {
        return ["StormguardStatus", "StormguardDebug"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("table-responsive", "lqos-table-wrap");
        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "align-middle", "mb-0");
        table.innerHTML = `
            <thead>
                <tr>
                    <th>${t("topology.node_kind.site")}</th>
                    <th>${t("stormguard_debug.column.dir")}</th>
                    <th>${t("topology_probes.status")}</th>
                    <th>${t("config.lts.api_keys.table.action")}</th>
                    <th>${t("common.table.age")}</th>
                    <th>${t("common.table.why")}</th>
                </tr>
            </thead>
        `;
        this.tbody = document.createElement("tbody");
        table.appendChild(this.tbody);
        tableWrap.appendChild(table);
        wrap.appendChild(tableWrap);

        base.appendChild(wrap);
        return base;
    }

    setup() {
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
        const rows = stormguardActivityRows(snapshot, 12);
        if (rows.length === 0) {
            this.tbody.innerHTML = `<tr><td colspan="6" class="text-muted">${t("dashlet.stormguard_activity.empty")}</td></tr>`;
            return;
        }

        this.tbody.innerHTML = "";
        rows.forEach((row) => {
            const tr = document.createElement("tr");
            tr.innerHTML = `
                <td class="fw-semibold">${row.site}</td>
                <td>${row.direction === "download" ? `<i class="fa fa-arrow-down text-primary"></i> ${t("common.table.down")}` : `<i class="fa fa-arrow-up text-success"></i> ${t("common.table.up")}`}</td>
                <td></td>
                <td>${row.action || t("dashlet.stormguard.state.holding")}</td>
                <td>${row.ageSeconds != null ? formatStormguardAgeSeconds(row.ageSeconds) : "—"}</td>
                <td class="text-muted">${row.reason}</td>
            `;
            tr.children[2].appendChild(activityBadge(row));
            this.tbody.appendChild(tr);
        });
    }
}
