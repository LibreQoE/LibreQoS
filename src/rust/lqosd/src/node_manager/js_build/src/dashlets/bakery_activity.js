import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {formatUnixSecondsToLocalDateTime, mkBadge} from "./bakery_shared";

function eventBadge(entry) {
    const status = (entry?.status ?? "").toString().toLowerCase();
    if (status === "error") {
        return mkBadge("Error", "bg-danger-subtle text-danger border border-danger-subtle", entry?.event || "");
    }
    if (status === "warning") {
        return mkBadge("Warning", "bg-warning-subtle text-warning border border-warning-subtle", entry?.event || "");
    }
    return mkBadge("Info", "bg-light text-secondary border", entry?.event || "");
}

export class BakeryActivityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
    }

    title() {
        return "Recent Bakery Events";
    }

    tooltip() {
        return "<h5>Recent Bakery Events</h5><p>Recent Bakery lifecycle events, including commit receipt, apply starts/finishes, preflight blocks, and failures.</p>";
    }

    subscribeTo() {
        return ["BakeryActivity"];
    }

    buildContainer() {
        const base = super.buildContainer();
        base.classList.add("dashbox-body-scroll");

        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");

        const thead = document.createElement("thead");
        const hr = document.createElement("tr");
        ["Local Time", "Status", "Summary"].forEach((label) => {
            const th = document.createElement("th");
            th.textContent = label;
            hr.appendChild(th);
        });
        thead.appendChild(hr);

        this.tbody = document.createElement("tbody");
        table.appendChild(thead);
        table.appendChild(this.tbody);
        tableWrap.appendChild(table);
        wrap.appendChild(tableWrap);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "BakeryActivity") {
            return;
        }

        const entries = Array.isArray(msg.data) ? msg.data : [];
        this.tbody.innerHTML = "";
        if (entries.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 3;
            td.textContent = "No recent activity";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
            return;
        }

        entries.forEach((entry) => {
            const tr = document.createElement("tr");
            const tdTime = document.createElement("td");
            const tdStatus = document.createElement("td");
            const tdSummary = document.createElement("td");

            tdTime.textContent = formatUnixSecondsToLocalDateTime(entry?.ts);
            tdStatus.appendChild(eventBadge(entry));
            tdSummary.textContent = entry?.summary || "—";
            tdSummary.title = entry?.event || "";

            tr.appendChild(tdTime);
            tr.appendChild(tdStatus);
            tr.appendChild(tdSummary);
            this.tbody.appendChild(tr);
        });
    }
}
