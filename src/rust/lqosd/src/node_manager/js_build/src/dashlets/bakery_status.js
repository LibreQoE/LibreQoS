import {DashletBaseInsight} from "./insight_dashlet_base";
import {redactCell} from "../helpers/redact";
import {formatUnixSecondsToLocalDateTime, mkBadge} from "./bakery_shared";

const CIRCUIT_ACTIVITY_MAX_ITEMS = 1;

function truncateSummary(summary, maxChars = 180) {
    const normalized = (summary ?? "").toString().trim();
    if (normalized.length <= maxChars) {
        return normalized;
    }
    return `${normalized.slice(0, maxChars - 3).trimEnd()}...`;
}

function classifyOutcome(entry) {
    const event = (entry?.event ?? "").toString().trim().toLowerCase();
    const status = (entry?.status ?? "").toString().trim().toLowerCase();

    if (event === "reload_required") {
        return {
            label: "Reload Required",
            className: "bg-danger-subtle text-danger border border-danger-subtle",
        };
    }
    if (event === "apply_failed" || status === "error") {
        return {
            label: "Failed",
            className: "bg-danger-subtle text-danger border border-danger-subtle",
        };
    }
    if (event.endsWith("_started")) {
        return {
            label: "Running",
            className: "bg-primary-subtle text-primary border border-primary-subtle",
        };
    }
    if (event === "apply_finished") {
        return {
            label: "Completed",
            className: "bg-success-subtle text-success border border-success-subtle",
        };
    }
    if (status === "warning") {
        return {
            label: "Warning",
            className: "bg-warning-subtle text-warning border border-warning-subtle",
        };
    }
    return {
        label: "Info",
        className: "bg-light text-secondary border",
    };
}

function isCircuitActivityEntry(entry) {
    const event = (entry?.event ?? "").toString().toLowerCase();
    const summary = (entry?.summary ?? "").toString().toLowerCase();
    if (!summary && !event) {
        return false;
    }
    if (summary.includes("circuit")) {
        return true;
    }
    return event.includes("circuit");
}

function recentCircuitEntries(entries) {
    return (Array.isArray(entries) ? entries : [])
        .filter((entry) => isCircuitActivityEntry(entry))
        .slice(0, CIRCUIT_ACTIVITY_MAX_ITEMS);
}

export class BakeryStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.size = 6;
        this.lastStatus = null;
        this.entries = [];
    }

    title() {
        return "Bakery Circuit Activity";
    }

    tooltip() {
        return "<h5>Bakery Circuit Activity</h5><p>Shows live Bakery circuit-change progress while a live change is applying, then keeps the single most recent circuit-scoped Bakery operation underneath.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus", "BakeryActivity"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const progressWrap = document.createElement("div");
        progressWrap.classList.add("mb-3");

        const progressHeader = document.createElement("div");
        progressHeader.classList.add("d-flex", "justify-content-between", "align-items-center", "small", "mb-1", "gap-2", "flex-wrap");
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
        wrap.appendChild(progressWrap);

        const operationsHeader = document.createElement("div");
        operationsHeader.classList.add("small", "fw-semibold", "text-uppercase", "text-body-secondary", "mb-2");
        operationsHeader.textContent = "Recent Circuit Operations";
        wrap.appendChild(operationsHeader);

        this.operationsList = document.createElement("div");
        this.operationsList.classList.add("d-flex", "flex-column", "gap-2");
        wrap.appendChild(this.operationsList);

        base.appendChild(wrap);
        return base;
    }

    renderProgress() {
        const status = this.lastStatus || {};
        const totalCommands = Number.isFinite(status.currentApplyTotalTcCommands)
            ? status.currentApplyTotalTcCommands
            : 0;
        const completedCommands = Number.isFinite(status.currentApplyCompletedTcCommands)
            ? status.currentApplyCompletedTcCommands
            : 0;
        const activeLiveChange = status.mode === "ApplyingLiveChange" && totalCommands > 0;
        const percent = totalCommands > 0
            ? Math.max(0, Math.min(100, (completedCommands / totalCommands) * 100))
            : 0;

        this.progressSummaryEl.textContent = activeLiveChange
            ? `${status.currentApplyPhase || "Applying live change"} • ${completedCommands.toLocaleString()} / ${totalCommands.toLocaleString()} tc`
            : "No live change currently applying";
        this.progressPercentEl.textContent = activeLiveChange ? `${percent.toFixed(1)}%` : "Idle";
        this.progressBarEl.style.width = `${percent}%`;
        this.progressBarEl.textContent = activeLiveChange ? `${percent.toFixed(1)}%` : "0%";
        this.progressBarEl.setAttribute("aria-valuenow", percent.toFixed(1));
        this.progressBarEl.setAttribute("aria-valuemin", "0");
        this.progressBarEl.setAttribute("aria-valuemax", "100");
        this.progressBarEl.className = activeLiveChange
            ? "progress-bar progress-bar-striped progress-bar-animated bg-info"
            : "progress-bar bg-secondary";
    }

    renderOperations() {
        this.operationsList.innerHTML = "";
        const entries = recentCircuitEntries(this.entries);
        if (entries.length === 0) {
            const empty = document.createElement("div");
            empty.classList.add("border", "rounded", "p-2", "text-muted", "small");
            empty.textContent = "No recent circuit-scoped Bakery activity";
            this.operationsList.appendChild(empty);
            return;
        }

        entries.forEach((entry) => {
            const outcome = classifyOutcome(entry);
            const card = document.createElement("div");
            card.classList.add("border", "rounded", "p-2", "bg-body-tertiary");

            const top = document.createElement("div");
            top.classList.add("d-flex", "justify-content-between", "align-items-start", "gap-2", "flex-wrap", "mb-2");

            const summary = document.createElement("div");
            summary.classList.add("small", "fw-semibold");
            const fullSummary = (entry?.summary ?? "Bakery update").toString();
            summary.textContent = truncateSummary(fullSummary);
            summary.title = fullSummary;
            redactCell(summary);
            top.appendChild(summary);
            top.appendChild(mkBadge(outcome.label, outcome.className, fullSummary));
            card.appendChild(top);

            const footer = document.createElement("div");
            footer.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "small", "text-body-secondary", "flex-wrap");

            const left = document.createElement("div");
            left.textContent = (entry?.event ?? "bakery_update").toString().replaceAll("_", " ");
            footer.appendChild(left);

            const right = document.createElement("div");
            right.textContent = formatUnixSecondsToLocalDateTime(entry?.ts);
            footer.appendChild(right);

            card.appendChild(footer);
            this.operationsList.appendChild(card);
        });
    }

    render() {
        this.renderProgress();
        this.renderOperations();
    }

    onMessage(msg) {
        if (msg.event === "BakeryStatus") {
            this.lastStatus = msg?.data?.currentState || null;
            this.render();
            return;
        }
        if (msg.event === "BakeryActivity") {
            this.entries = Array.isArray(msg.data) ? msg.data : [];
            this.render();
        }
    }

    canBeSlowedDown() {
        return false;
    }
}
