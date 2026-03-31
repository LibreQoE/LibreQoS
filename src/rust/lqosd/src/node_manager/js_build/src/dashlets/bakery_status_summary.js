import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {
    bakeryModeBadge,
    formatElapsedSince,
    formatUnixSecondsToLocalDateTime,
    mkBadge,
} from "./bakery_shared";

export class BakeryStatusSummaryDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.lastStatus = null;
    }

    title() {
        return "Bakery Status";
    }

    tooltip() {
        return "<h5>Bakery Status</h5><p>Current Bakery mode, how long the current action has been running, and the last recorded success/failure timestamps.</p>";
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

        this.modeEl = document.createElement("div");
        this.durationEl = document.createElement("span");
        this.lastSuccessEl = document.createElement("span");
        this.lastFailureEl = document.createElement("span");
        this.lastFailureSummaryEl = document.createElement("span");
        this.lastFailureSummaryEl.style.whiteSpace = "normal";
        this.lastFailureSummaryEl.style.wordBreak = "break-word";
        this.reloadRequiredEl = document.createElement("div");
        this.reloadReasonEl = document.createElement("span");
        this.reloadReasonEl.style.whiteSpace = "normal";
        this.reloadReasonEl.style.wordBreak = "break-word";

        tbody.appendChild(mkRow("Current Mode", this.modeEl));
        tbody.appendChild(mkRow("Current Duration", this.durationEl));
        tbody.appendChild(mkRow("Runtime Drift", this.reloadRequiredEl));
        tbody.appendChild(mkRow("Last Success", this.lastSuccessEl));
        tbody.appendChild(mkRow("Last Failure", this.lastFailureEl));
        tbody.appendChild(mkRow("Failure Summary", this.lastFailureSummaryEl));
        tbody.appendChild(mkRow("Reload Reason", this.reloadReasonEl));

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
        this.lastStatus = msg?.data?.currentState || null;
        this.renderStatus();
    }

    renderStatus() {
        const status = this.lastStatus || {};

        this.modeEl.innerHTML = "";
        this.modeEl.appendChild(bakeryModeBadge(status.mode));
        this.reloadRequiredEl.innerHTML = "";
        this.reloadRequiredEl.appendChild(
            status.reloadRequired
                ? mkBadge("Full Reload Required", "bg-danger-subtle text-danger border border-danger-subtle")
                : mkBadge("No Drift", "bg-success-subtle text-success border border-success-subtle"),
        );

        this.durationEl.textContent = status.currentActionStartedUnix
            ? formatElapsedSince(status.currentActionStartedUnix)
            : "—";
        this.lastSuccessEl.textContent = formatUnixSecondsToLocalDateTime(status.lastSuccessUnix);
        this.lastFailureEl.textContent = formatUnixSecondsToLocalDateTime(status.lastFailureUnix);
        this.lastFailureSummaryEl.textContent = status.lastFailureSummary || "—";
        this.reloadReasonEl.textContent = status.reloadRequiredReason || "—";
    }
}
