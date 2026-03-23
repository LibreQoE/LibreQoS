import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {bakeryPreflightBadge, mkBadge} from "./bakery_shared";

function clamp(n, min, max) {
    return Math.min(Math.max(n, min), max);
}

function formatBinaryBytes(bytes) {
    const value = typeof bytes === "number" ? bytes : parseInt(bytes, 10);
    if (!Number.isFinite(value) || value < 0) {
        return "—";
    }
    if (value === 0) {
        return "0 B";
    }

    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let scaled = value;
    let unitIndex = 0;
    while (scaled >= 1024 && unitIndex < units.length - 1) {
        scaled /= 1024;
        unitIndex += 1;
    }
    const decimals = scaled >= 100 || unitIndex === 0 ? 0 : 1;
    return `${scaled.toFixed(decimals)} ${units[unitIndex]}`;
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
        this.badgeWrap.classList.add("d-flex", "flex-wrap", "gap-2", "mb-3");
        wrap.appendChild(this.badgeWrap);

        const tableWrap = document.createElement("div");
        tableWrap.classList.add("lqos-table-wrap", "mb-3");

        const table = document.createElement("table");
        table.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");

        const thead = document.createElement("thead");
        const hr = document.createElement("tr");
        ["Interface", "Planned", "Usage", "Mix"].forEach((label) => {
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

        const safetyWrap = document.createElement("div");
        safetyWrap.classList.add("lqos-table-wrap");

        const safetyTable = document.createElement("table");
        safetyTable.classList.add("lqos-table", "lqos-table-compact", "mb-0", "small");
        const safetyBody = document.createElement("tbody");

        const mkRow = (label) => {
            const tr = document.createElement("tr");
            const tdL = document.createElement("td");
            tdL.classList.add("table-label-cell");
            tdL.style.width = "44%";
            tdL.textContent = label;
            const tdV = document.createElement("td");
            tdV.classList.add("table-value-cell");
            const valueEl = document.createElement("span");
            valueEl.textContent = "—";
            tdV.appendChild(valueEl);
            tr.appendChild(tdL);
            tr.appendChild(tdV);
            safetyBody.appendChild(tr);
            return {tr, valueEl};
        };

        this.safeBudgetRow = mkRow("Safe Budget");
        this.hardLimitRow = mkRow("Kernel Limit");
        this.estimatedMemoryRow = mkRow("Est. Memory");
        this.availableMemoryRow = mkRow("Avail. Memory");
        this.memoryFloorRow = mkRow("Safety Floor");
        this.memoryHeadroomRow = mkRow("Headroom");

        safetyTable.appendChild(safetyBody);
        safetyWrap.appendChild(safetyTable);
        wrap.appendChild(safetyWrap);
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
        this.badgeWrap.appendChild(bakeryMemoryBadge(this.lastPreflight));

        this.interfacesTbody.innerHTML = "";

        if (!this.lastPreflight) {
            this.setSafetySummary({});
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 4;
            td.textContent = "No interface budget data";
            tr.appendChild(td);
            this.interfacesTbody.appendChild(tr);
            return;
        }

        const interfaces = Array.isArray(this.lastPreflight.interfaces)
            ? [...this.lastPreflight.interfaces].sort((left, right) => {
                const plannedDiff = (right?.plannedQdiscs || 0) - (left?.plannedQdiscs || 0);
                if (plannedDiff !== 0) {
                    return plannedDiff;
                }
                return (left?.name || "").localeCompare(right?.name || "");
            })
            : [];
        this.setSafetySummary(this.lastPreflight);

        if (interfaces.length === 0) {
            const tr = document.createElement("tr");
            const td = document.createElement("td");
            td.colSpan = 4;
            td.textContent = "No interface budget data";
            tr.appendChild(td);
            this.interfacesTbody.appendChild(tr);
            return;
        }

        interfaces.forEach((entry) => {
            const tr = document.createElement("tr");
            const tdName = document.createElement("td");
            const tdPlanned = document.createElement("td");
            const tdUsage = document.createElement("td");
            const tdMix = document.createElement("td");

            tdName.textContent = entry?.name || "—";

            const plannedValue = document.createElement("div");
            plannedValue.classList.add("fw-semibold");
            plannedValue.textContent = Number.isFinite(entry?.plannedQdiscs)
                ? entry.plannedQdiscs.toLocaleString()
                : "—";
            const plannedMeta = document.createElement("div");
            plannedMeta.classList.add("small", "text-body-secondary");
            plannedMeta.textContent = formatBinaryBytes(entry?.estimatedMemoryBytes);
            tdPlanned.appendChild(plannedValue);
            tdPlanned.appendChild(plannedMeta);

            const safeBudget = Number.isFinite(this.lastPreflight?.safeBudget) ? this.lastPreflight.safeBudget : null;
            const hardLimit = Number.isFinite(this.lastPreflight?.hardLimit) ? this.lastPreflight.hardLimit : null;
            const usageText = document.createElement("div");
            usageText.classList.add("d-flex", "justify-content-between", "small", "mb-1");
            const usageLabel = document.createElement("span");
            usageLabel.textContent = Number.isFinite(entry?.plannedQdiscs) && safeBudget !== null
                ? `${formatPercent(entry.plannedQdiscs, safeBudget)} of safe`
                : "—";
            const usageCount = document.createElement("span");
            usageCount.classList.add("text-body-secondary");
            usageCount.textContent = safeBudget !== null && hardLimit !== null
                ? `${safeBudget.toLocaleString()} / ${hardLimit.toLocaleString()}`
                : "—";
            usageText.appendChild(usageLabel);
            usageText.appendChild(usageCount);

            const progress = document.createElement("div");
            progress.classList.add("progress");
            progress.style.height = "0.55rem";
            const bar = document.createElement("div");
            bar.classList.add("progress-bar", usageBarClass(entry?.plannedQdiscs, safeBudget, hardLimit));
            bar.setAttribute("role", "progressbar");
            const pct = Number.isFinite(entry?.plannedQdiscs) && safeBudget !== null && safeBudget > 0
                ? clamp((entry.plannedQdiscs / safeBudget) * 100.0, 0, 100)
                : 0;
            bar.style.width = `${pct.toFixed(1)}%`;
            bar.setAttribute("aria-valuemin", "0");
            bar.setAttribute("aria-valuemax", safeBudget !== null ? safeBudget.toString() : "0");
            bar.setAttribute("aria-valuenow", Number.isFinite(entry?.plannedQdiscs) ? entry.plannedQdiscs.toString() : "0");
            progress.appendChild(bar);

            tdUsage.appendChild(usageText);
            tdUsage.appendChild(progress);

            const mixWrap = document.createElement("div");
            mixWrap.classList.add("d-flex", "flex-wrap", "gap-1", "mb-1");
            mixWrap.appendChild(mkBadge(
                `infra ${Number.isFinite(entry?.infraQdiscs) ? entry.infraQdiscs.toLocaleString() : "—"}`,
                "bg-light text-secondary border",
            ));
            if (Number.isFinite(entry?.cakeQdiscs) && entry.cakeQdiscs > 0) {
                mixWrap.appendChild(mkBadge(
                    `cake ${entry.cakeQdiscs.toLocaleString()}`,
                    "bg-warning-subtle text-warning border border-warning-subtle",
                ));
            }
            if (Number.isFinite(entry?.fqCodelQdiscs) && entry.fqCodelQdiscs > 0) {
                mixWrap.appendChild(mkBadge(
                    `fq_codel ${entry.fqCodelQdiscs.toLocaleString()}`,
                    "bg-info-subtle text-info border border-info-subtle",
                ));
            }
            tdMix.appendChild(mixWrap);

            tr.appendChild(tdName);
            tr.appendChild(tdPlanned);
            tr.appendChild(tdUsage);
            tr.appendChild(tdMix);
            this.interfacesTbody.appendChild(tr);
        });
    }

    setSafetySummary(preflight) {
        const setText = (row, value, className = "") => {
            row.valueEl.textContent = value;
            row.valueEl.className = className;
        };
        const setVisible = (row, visible) => {
            row.tr.hidden = !visible;
        };

        setText(this.safeBudgetRow, Number.isFinite(preflight?.safeBudget) ? preflight.safeBudget.toLocaleString() : "—");
        setText(this.hardLimitRow, Number.isFinite(preflight?.hardLimit) ? preflight.hardLimit.toLocaleString() : "—");
        setText(
            this.estimatedMemoryRow,
            Number.isFinite(preflight?.estimatedTotalMemoryBytes) ? formatBinaryBytes(preflight.estimatedTotalMemoryBytes) : "—",
        );
        setVisible(this.availableMemoryRow, Number.isFinite(preflight?.memoryAvailableBytes));
        setVisible(
            this.memoryFloorRow,
            Number.isFinite(preflight?.memoryAvailableBytes) && Number.isFinite(preflight?.memoryGuardMinAvailableBytes),
        );
        const headroom = Number.isFinite(preflight?.memoryAvailableBytes)
            && Number.isFinite(preflight?.memoryGuardMinAvailableBytes)
            ? preflight.memoryAvailableBytes - preflight.memoryGuardMinAvailableBytes
            : null;
        setVisible(this.memoryHeadroomRow, headroom !== null);

        if (Number.isFinite(preflight?.memoryAvailableBytes)) {
            setText(this.availableMemoryRow, formatBinaryBytes(preflight.memoryAvailableBytes));
        }
        if (Number.isFinite(preflight?.memoryGuardMinAvailableBytes)) {
            setText(this.memoryFloorRow, formatBinaryBytes(preflight.memoryGuardMinAvailableBytes));
        }
        if (headroom !== null) {
            setText(
                this.memoryHeadroomRow,
                `${headroom >= 0 ? "+" : "-"}${formatBinaryBytes(Math.abs(headroom))}`,
                headroom < 0 ? "text-danger" : "",
            );
        }
    }
}
