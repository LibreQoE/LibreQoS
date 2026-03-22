import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {bakeryPreflightBadge} from "./bakery_shared";

export class BakeryCapacityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.lastPreflight = null;
    }

    title() {
        return "Capacity / Safety";
    }

    tooltip() {
        return "<h5>Capacity / Safety</h5><p>Shows the last recorded Bakery qdisc-budget preflight, including per-interface planned qdisc counts and whether the full reload was within budget.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.badgeWrap = document.createElement("div");
        this.badgeWrap.classList.add("mb-2");
        wrap.appendChild(this.badgeWrap);

        this.summaryEl = document.createElement("div");
        this.summaryEl.classList.add("small", "text-muted", "mb-2");
        this.summaryEl.style.whiteSpace = "normal";
        this.summaryEl.style.wordBreak = "break-word";
        wrap.appendChild(this.summaryEl);

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");

        const thead = document.createElement("thead");
        const hr = document.createElement("tr");
        ["Interface", "Planned", "Budget"].forEach((label) => {
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
        if (msg.event !== "BakeryStatus") {
            return;
        }
        this.lastPreflight = msg?.data?.currentState?.preflight || null;
        this.renderPreflight();
    }

    renderPreflight() {
        this.badgeWrap.innerHTML = "";
        this.badgeWrap.appendChild(bakeryPreflightBadge(this.lastPreflight));

        this.summaryEl.textContent = this.lastPreflight?.message || "No preflight has been recorded yet.";
        this.tbody.innerHTML = "";

        const interfaces = Array.isArray(this.lastPreflight?.interfaces) ? this.lastPreflight.interfaces : [];
        if (interfaces.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 3;
            td.textContent = "No interface budget data";
            tr.appendChild(td);
            this.tbody.appendChild(tr);
            return;
        }

        interfaces.forEach((entry) => {
            const tr = document.createElement("tr");
            const tdName = document.createElement("td");
            const tdPlanned = document.createElement("td");
            const tdBudget = document.createElement("td");

            tdName.textContent = entry?.name || "—";
            tdPlanned.textContent = Number.isFinite(entry?.plannedQdiscs)
                ? entry.plannedQdiscs.toLocaleString()
                : "—";

            const hardLimit = Number.isFinite(this.lastPreflight?.hardLimit) ? this.lastPreflight.hardLimit : null;
            const safeBudget = Number.isFinite(this.lastPreflight?.safeBudget) ? this.lastPreflight.safeBudget : null;
            tdBudget.textContent = safeBudget !== null && hardLimit !== null
                ? `${safeBudget.toLocaleString()} / ${hardLimit.toLocaleString()}`
                : "—";
            if (Number.isFinite(entry?.plannedQdiscs) && safeBudget !== null && entry.plannedQdiscs > safeBudget) {
                tdPlanned.classList.add("text-danger");
            }

            tr.appendChild(tdName);
            tr.appendChild(tdPlanned);
            tr.appendChild(tdBudget);
            this.tbody.appendChild(tr);
        });
    }
}
