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
        return "<h5>Apply Summary</h5><p>Shows live full-reload progress and the most recent Bakery apply type, command counts, and recorded build/apply durations.</p><p>During large Bakery reloads, queue polling is intentionally backed off so progress reporting stays lightweight.</p>";
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

        const progressWrap = document.createElement("div");
        progressWrap.classList.add("mb-3");

        const progressHeader = document.createElement("div");
        progressHeader.classList.add("d-flex", "justify-content-between", "align-items-center", "small", "mb-1");
        this.progressSummaryEl = document.createElement("span");
        this.progressSummaryEl.classList.add("text-body-secondary");
        this.progressPercentEl = document.createElement("span");
        this.progressPercentEl.classList.add("fw-semibold");
        progressHeader.appendChild(this.progressSummaryEl);
        progressHeader.appendChild(this.progressPercentEl);

        this.progressBarWrapEl = document.createElement("div");
        this.progressBarWrapEl.classList.add("progress");
        this.progressBarWrapEl.style.height = "0.85rem";
        this.progressBarEl = document.createElement("div");
        this.progressBarEl.classList.add("progress-bar");
        this.progressBarEl.setAttribute("role", "progressbar");
        this.progressBarEl.style.width = "0%";
        this.progressBarEl.textContent = "0%";
        this.progressBarWrapEl.appendChild(this.progressBarEl);

        progressWrap.appendChild(progressHeader);
        progressWrap.appendChild(this.progressBarWrapEl);

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
        wrap.appendChild(progressWrap);
        wrap.appendChild(tableWrap);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "BakeryStatus") {
            return;
        }
        const status = msg?.data?.currentState || {};
        const totalCommands = Number.isFinite(status.currentApplyTotalTcCommands)
            ? status.currentApplyTotalTcCommands
            : 0;
        const completedCommands = Number.isFinite(status.currentApplyCompletedTcCommands)
            ? status.currentApplyCompletedTcCommands
            : 0;
        const totalChunks = Number.isFinite(status.currentApplyTotalChunks)
            ? status.currentApplyTotalChunks
            : 0;
        const completedChunks = Number.isFinite(status.currentApplyCompletedChunks)
            ? status.currentApplyCompletedChunks
            : 0;
        const percent = totalCommands > 0
            ? Math.max(0, Math.min(100, (completedCommands / totalCommands) * 100))
            : 0;
        const activeFullReload = status.mode === "ApplyingFullReload" && totalCommands > 0;
        this.progressSummaryEl.textContent = activeFullReload
            ? `${status.currentApplyPhase || "Applying tc command chunks"} • ${completedCommands.toLocaleString()} / ${totalCommands.toLocaleString()} commands • chunk ${Math.min(completedChunks + 1, totalChunks).toLocaleString()} / ${totalChunks.toLocaleString()} • queue polling paused`
            : "No full reload currently applying";
        this.progressPercentEl.textContent = activeFullReload ? `${percent.toFixed(1)}%` : "Idle";
        this.progressBarEl.style.width = `${percent}%`;
        this.progressBarEl.textContent = activeFullReload ? `${percent.toFixed(1)}%` : "0%";
        this.progressBarEl.setAttribute("aria-valuenow", percent.toFixed(1));
        this.progressBarEl.setAttribute("aria-valuemin", "0");
        this.progressBarEl.setAttribute("aria-valuemax", "100");
        this.progressBarEl.className = activeFullReload
            ? "progress-bar progress-bar-striped progress-bar-animated bg-warning"
            : "progress-bar bg-secondary";
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
