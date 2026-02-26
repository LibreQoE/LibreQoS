import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {get_ws_client} from "../pubsub/ws";

function formatUnixSecondsToLocalTime(unixSeconds) {
    const n = typeof unixSeconds === "number" ? unixSeconds : parseInt(unixSeconds, 10);
    if (!Number.isFinite(n) || n <= 0) {
        return "";
    }
    return new Date(n * 1000).toLocaleString();
}

export class TreeGuardActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.circuitNameById = new Map();
    }

    title() {
        return "TreeGuard Activity";
    }

    tooltip() {
        return "<h5>TreeGuard Activity</h5><p>Recent TreeGuard actions, including dry-run entries and persisted changes.</p>";
    }

    subscribeTo() {
        return ["TreeGuardActivity"];
    }

    setup() {
        const wsClient = get_ws_client();
        const wrapped = (msg) => {
            wsClient.off("AllShapedDevices", wrapped);
            const devices = msg && Array.isArray(msg.data) ? msg.data : [];
            devices.forEach((d) => {
                const id = (d && d.circuit_id ? String(d.circuit_id) : "").trim();
                const name = (d && d.circuit_name ? String(d.circuit_name) : "").trim();
                if (!id || !name) return;
                if (!this.circuitNameById.has(id)) {
                    this.circuitNameById.set(id, name);
                }
            });
        };
        wsClient.on("AllShapedDevices", wrapped);
        wsClient.send({ AllShapedDevices: {} });
    }

    buildContainer() {
        let base = super.buildContainer();

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const table = document.createElement("table");
        table.classList.add("table", "table-sm", "table-striped", "mb-0");

        const thead = document.createElement("thead");
        const headRow = document.createElement("tr");
        ["Local Time", "Entity", "Action", "Persisted", "Reason"].forEach((h) => {
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
        if (msg.event !== "TreeGuardActivity") {
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
            tdTime.textContent = formatUnixSecondsToLocalTime(e.time);

            const tdEntity = document.createElement("td");
            const et = e.entity_type ?? "";
            const eid = e.entity_id ?? "";
            const entityType = (et || "").toString().toLowerCase();
            const entityId = (eid || "").toString();
            let display = entityId;
            if (entityType === "circuit") {
                const name = this.circuitNameById.get(entityId.trim());
                if (name) {
                    display = name;
                    if (name !== entityId) {
                        tdEntity.title = entityId;
                    }
                }
            }
            tdEntity.textContent = et && display ? `${et}: ${display}` : (display || et);

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
