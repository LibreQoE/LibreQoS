import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {formatDurationMs, mkBadge} from "./bakery_shared";

function formatApplyType(kindRaw) {
    switch ((kindRaw ?? "").toString()) {
        case "FullReload":
            return mkBadge("Full Reload", "bg-warning-subtle text-warning border border-warning-subtle");
        case "LiveChange":
            return mkBadge("Live Change", "bg-info-subtle text-info border border-info-subtle");
        case "None":
        default:
            return mkBadge("None", "bg-light text-secondary border");
    }
}

export class BakeryApplySummaryDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
    }

    title() {
        return "Apply Summary";
    }

    tooltip() {
        return "<h5>Apply Summary</h5><p>Shows the most recent Bakery apply type, command counts, and recorded build/apply durations.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");
        const tbody = document.createElement("tbody");

        const mkRow = (label, valueEl) => {
            const tr = document.createElement("tr");
            const tdL = document.createElement("td");
            tdL.classList.add("table-label-cell");
            tdL.style.width = "42%";
            tdL.textContent = label;
            const tdV = document.createElement("td");
            tdV.classList.add("table-value-cell");
            tdV.appendChild(valueEl);
            tr.appendChild(tdL);
            tr.appendChild(tdV);
            return tr;
        };

        this.applyTypeEl = document.createElement("div");
        this.totalCommandsEl = document.createElement("span");
        this.classCommandsEl = document.createElement("span");
        this.qdiscCommandsEl = document.createElement("span");
        this.buildDurationEl = document.createElement("span");
        this.applyDurationEl = document.createElement("span");

        tbody.appendChild(mkRow("Last Apply Type", this.applyTypeEl));
        tbody.appendChild(mkRow("Total tc Commands", this.totalCommandsEl));
        tbody.appendChild(mkRow("Class Commands", this.classCommandsEl));
        tbody.appendChild(mkRow("Qdisc Commands", this.qdiscCommandsEl));
        tbody.appendChild(mkRow("Build Time", this.buildDurationEl));
        tbody.appendChild(mkRow("Apply Time", this.applyDurationEl));

        table.appendChild(tbody);
        tableWrap.appendChild(table);
        wrap.appendChild(tableWrap);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "BakeryStatus") {
            return;
        }
        const status = msg?.data?.currentState || {};
        this.applyTypeEl.innerHTML = "";
        this.applyTypeEl.appendChild(formatApplyType(status.lastApplyType));
        this.totalCommandsEl.textContent = Number.isFinite(status.lastTotalTcCommands)
            ? status.lastTotalTcCommands.toLocaleString()
            : "—";
        this.classCommandsEl.textContent = Number.isFinite(status.lastClassCommands)
            ? status.lastClassCommands.toLocaleString()
            : "—";
        this.qdiscCommandsEl.textContent = Number.isFinite(status.lastQdiscCommands)
            ? status.lastQdiscCommands.toLocaleString()
            : "—";
        this.buildDurationEl.textContent = formatDurationMs(status.lastBuildDurationMs);
        this.applyDurationEl.textContent = formatDurationMs(status.lastApplyDurationMs);
    }
}
