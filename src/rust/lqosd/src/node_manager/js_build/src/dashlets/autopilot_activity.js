import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class AutopilotActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
    }

    title() {
        return "Autopilot Activity";
    }

    tooltip() {
        return "<h5>Autopilot Activity</h5><p>Recent Autopilot actions, including dry-run entries and persisted changes.</p>";
    }

    subscribeTo() {
        return ["AutopilotActivity"];
    }

    buildContainer() {
        let base = super.buildContainer();

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const table = document.createElement("table");
        table.classList.add("table", "table-sm", "table-striped", "mb-0");

        const thead = document.createElement("thead");
        const headRow = document.createElement("tr");
        ["Time", "Entity", "Action", "Persisted", "Reason"].forEach((h) => {
            const th = document.createElement("th");
            th.textContent = h;
            headRow.appendChild(th);
        });
        thead.appendChild(headRow);

        this.tbody = document.createElement("tbody");

        table.appendChild(thead);
        table.appendChild(this.tbody);
        wrap.appendChild(table);

        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "AutopilotActivity") {
            return;
        }

        const entries = Array.isArray(msg.data) ? msg.data : [];
        this.tbody.innerHTML = "";

        if (entries.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 5;
            td.classList.add("text-muted");
            td.textContent = "No recent activity";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
            return;
        }

        entries.slice(0, 50).forEach((e) => {
            const tr = document.createElement("tr");

            const tdTime = document.createElement("td");
            tdTime.textContent = e.time ?? "";

            const tdEntity = document.createElement("td");
            const et = e.entity_type ?? "";
            const eid = e.entity_id ?? "";
            tdEntity.textContent = et && eid ? `${et}: ${eid}` : (eid || et);

            const tdAction = document.createElement("td");
            tdAction.textContent = e.action ?? "";

            const tdPersisted = document.createElement("td");
            tdPersisted.textContent = e.persisted ? "Yes" : "No";

            const tdReason = document.createElement("td");
            tdReason.textContent = e.reason ?? "";

            tr.appendChild(tdTime);
            tr.appendChild(tdEntity);
            tr.appendChild(tdAction);
            tr.appendChild(tdPersisted);
            tr.appendChild(tdReason);
            this.tbody.appendChild(tr);
        });
    }
}

