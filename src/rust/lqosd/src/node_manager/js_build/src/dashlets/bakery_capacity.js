import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {bakeryPreflightBadge, mkBadge} from "./bakery_shared";

function clamp(n, min, max) {
    return Math.min(Math.max(n, min), max);
}

function formatPercent(value, max) {
    if (!Number.isFinite(value) || !Number.isFinite(max) || max <= 0) {
        return "—";
    }
    const pct = (value / max) * 100.0;
    return `${pct >= 10 ? pct.toFixed(0) : pct.toFixed(1)}%`;
}

function usageBarClass(plannedQdiscs, safeBudget, hardLimit) {
    if (!Number.isFinite(plannedQdiscs) || !Number.isFinite(safeBudget) || safeBudget <= 0) {
        return "bg-secondary";
    }
    if ((Number.isFinite(hardLimit) && plannedQdiscs > hardLimit) || plannedQdiscs > safeBudget) {
        return "bg-danger";
    }
    const pct = (plannedQdiscs / safeBudget) * 100.0;
    if (pct >= 85) {
        return "bg-warning";
    }
    if (pct >= 60) {
        return "bg-info";
    }
    return "bg-success";
}

function bakeryMemoryBadge(preflight) {
    if (!preflight) {
        return mkBadge("Memory Unknown", "bg-light text-secondary border");
    }
    if (preflight.memoryOk) {
        return mkBadge("Memory OK", "bg-success-subtle text-success border border-success-subtle", preflight.message || "");
    }
    return mkBadge("Memory Guard", "bg-danger-subtle text-danger border border-danger-subtle", preflight.message || "");
}

export class BakeryCapacityDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.lastPreflight = null;
        this.liveCapacityInterfaces = [];
        this.liveCapacitySafeBudget = null;
    }

    title() {
        return "Capacity / Safety";
    }

    tooltip() {
        return "<h5>Capacity / Safety</h5><p>Shows current live TC handle usage by interface against Bakery's safe budget. The badges summarize the most recent full-reload preflight.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.badgeWrap = document.createElement("div");
        this.badgeWrap.classList.add("d-flex", "flex-wrap", "gap-2", "mb-3");
        wrap.appendChild(this.badgeWrap);

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap", "mb-3");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");

        const thead = document.createElement("thead");
        const hr = document.createElement("tr");
        ["Interface", "Usage"].forEach((label) => {
            const th = document.createElement("th");
            th.textContent = label;
            hr.appendChild(th);
        });
        thead.appendChild(hr);

        this.interfacesTbody = document.createElement("tbody");

        table.appendChild(thead);
        table.appendChild(this.interfacesTbody);
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
        this.liveCapacityInterfaces = Array.isArray(msg?.data?.currentState?.liveCapacityInterfaces)
            ? msg.data.currentState.liveCapacityInterfaces
            : [];
        this.liveCapacitySafeBudget = Number.isFinite(msg?.data?.currentState?.liveCapacitySafeBudget)
            ? msg.data.currentState.liveCapacitySafeBudget
            : null;
        this.renderCapacity();
    }

    renderCapacity() {
        this.badgeWrap.innerHTML = "";
        this.badgeWrap.appendChild(bakeryPreflightBadge(this.lastPreflight));
        this.badgeWrap.appendChild(bakeryMemoryBadge(this.lastPreflight));

        this.interfacesTbody.innerHTML = "";

        const interfaces = Array.isArray(this.liveCapacityInterfaces)
            ? [...this.liveCapacityInterfaces].sort((left, right) => {
                const liveDiff = (right?.liveQdiscs || 0) - (left?.liveQdiscs || 0);
                if (liveDiff !== 0) {
                    return liveDiff;
                }
                return (left?.name || "").localeCompare(right?.name || "");
            })
            : [];

        if (interfaces.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 2;
            td.textContent = "No live interface usage data";
            tr.appendChild(td);
            this.interfacesTbody.appendChild(tr);
            return;
        }

        interfaces.forEach((entry) => {
            const tr = document.createElement("tr");
            const tdName = document.createElement("td");
            const tdUsage = document.createElement("td");

            tdName.textContent = entry?.name || "—";

            const safeBudget = this.liveCapacitySafeBudget;
            const usageText = document.createElement("div");
            usageText.classList.add("d-flex", "justify-content-between", "small", "mb-1");
            const usageLabel = document.createElement("span");
            usageLabel.textContent = Number.isFinite(entry?.liveQdiscs) && safeBudget !== null
                ? formatPercent(entry.liveQdiscs, safeBudget)
                : "—";
            const usageCount = document.createElement("span");
            usageCount.classList.add("text-body-secondary");
            usageCount.textContent = Number.isFinite(entry?.liveQdiscs)
                ? `${entry.liveQdiscs.toLocaleString()} handles`
                : "—";
            usageText.appendChild(usageLabel);
            usageText.appendChild(usageCount);

            const progress = document.createElement("div");
            progress.classList.add("progress");
            progress.style.height = "0.55rem";
            const bar = document.createElement("div");
            bar.classList.add("progress-bar", usageBarClass(entry?.liveQdiscs, safeBudget, null));
            bar.setAttribute("role", "progressbar");
            const pct = Number.isFinite(entry?.liveQdiscs) && safeBudget !== null && safeBudget > 0
                ? clamp((entry.liveQdiscs / safeBudget) * 100.0, 0, 100)
                : 0;
            bar.style.width = `${pct.toFixed(1)}%`;
            bar.setAttribute("aria-valuemin", "0");
            bar.setAttribute("aria-valuemax", safeBudget !== null ? safeBudget.toString() : "0");
            bar.setAttribute("aria-valuenow", Number.isFinite(entry?.liveQdiscs) ? entry.liveQdiscs.toString() : "0");
            progress.appendChild(bar);

            tdUsage.appendChild(usageText);
            tdUsage.appendChild(progress);

            tr.appendChild(tdName);
            tr.appendChild(tdUsage);
            this.interfacesTbody.appendChild(tr);
        });
    }
}
