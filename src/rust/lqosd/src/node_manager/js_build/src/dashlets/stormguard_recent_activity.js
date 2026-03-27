import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {mkBadge} from "./bakery_shared";
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
        return "StormGuard Recent Activity";
    }

    tooltip() {
        return "<h5>StormGuard Recent Activity</h5><p>Shows the freshest StormGuard actions and holding states so operators can quickly answer what the control loop is doing right now.</p>";
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
                    <th>Site</th>
                    <th>Dir</th>
                    <th>Status</th>
                    <th>Action</th>
                    <th>Age</th>
                    <th>Why</th>
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
            this.tbody.innerHTML = "<tr><td colspan='6' class='text-muted'>No recent StormGuard activity yet.</td></tr>";
            return;
        }

        this.tbody.innerHTML = "";
        rows.forEach((row) => {
            const tr = document.createElement("tr");
            tr.innerHTML = `
                <td class="fw-semibold">${row.site}</td>
                <td>${row.direction === "download" ? "<i class='fa fa-arrow-down text-primary'></i> Down" : "<i class='fa fa-arrow-up text-success'></i> Up"}</td>
                <td></td>
                <td>${row.action || "Holding"}</td>
                <td>${row.ageSeconds != null ? formatStormguardAgeSeconds(row.ageSeconds) : "—"}</td>
                <td class="text-muted">${row.reason}</td>
            `;
            tr.children[2].appendChild(activityBadge(row));
            this.tbody.appendChild(tr);
        });
    }
}
