import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

function progressMetric(label, badgeClass) {
    const wrap = document.createElement("div");
    wrap.classList.add("d-flex", "flex-column", "gap-1");

    const top = document.createElement("div");
    top.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "small");
    const tag = document.createElement("span");
    tag.className = `badge ${badgeClass}`;
    tag.textContent = label;
    const count = document.createElement("span");
    count.classList.add("fw-semibold");
    top.appendChild(tag);
    top.appendChild(count);

    const barWrap = document.createElement("div");
    barWrap.classList.add("progress");
    barWrap.style.height = "0.7rem";
    const bar = document.createElement("div");
    bar.classList.add("progress-bar");
    barWrap.appendChild(bar);

    wrap.appendChild(top);
    wrap.appendChild(barWrap);
    return { wrap, count, bar };
}

function stackedMetric(label) {
    const wrap = document.createElement("div");
    wrap.classList.add("d-flex", "flex-column", "gap-1");

    const top = document.createElement("div");
    top.classList.add("d-flex", "justify-content-between", "align-items-center", "gap-2", "small");
    const title = document.createElement("span");
    title.classList.add("fw-semibold");
    title.textContent = label;
    const count = document.createElement("span");
    count.classList.add("fw-semibold");
    top.appendChild(title);
    top.appendChild(count);

    const barWrap = document.createElement("div");
    barWrap.classList.add("progress");
    barWrap.style.height = "0.7rem";

    const cakeBar = document.createElement("div");
    cakeBar.classList.add("progress-bar", "bg-success");
    const mixedBar = document.createElement("div");
    mixedBar.classList.add("progress-bar", "bg-warning");
    const fqBar = document.createElement("div");
    fqBar.classList.add("progress-bar", "bg-info");
    [cakeBar, mixedBar, fqBar].forEach((bar) => {
        barWrap.appendChild(bar);
    });

    const legend = document.createElement("div");
    legend.classList.add("d-flex", "flex-wrap", "gap-2", "small", "text-body-secondary");

    const mkLegend = (labelText, badgeClass) => {
        const span = document.createElement("span");
        span.className = `badge ${badgeClass}`;
        span.textContent = labelText;
        return span;
    };
    legend.appendChild(mkLegend("cake", "bg-success"));
    legend.appendChild(mkLegend("mixed", "bg-warning text-dark"));
    legend.appendChild(mkLegend("fq_codel", "bg-info text-dark"));

    wrap.appendChild(top);
    wrap.appendChild(barWrap);
    wrap.appendChild(legend);
    return { wrap, count, cakeBar, mixedBar, fqBar };
}

function updateMetric(metric, value, max, colorClass, title) {
    const hasMax = Number.isFinite(max) && max > 0;
    const safeMax = hasMax ? max : Math.max(1, Number.isFinite(value) ? value : 1);
    const safeValue = Number.isFinite(value) ? value : 0;
    const pct = Math.max(0, Math.min(100, (safeValue / safeMax) * 100));
    metric.count.textContent = hasMax
        ? `${safeValue.toLocaleString()} / ${max.toLocaleString()}`
        : safeValue.toLocaleString();
    metric.bar.className = `progress-bar ${colorClass}`;
    metric.bar.style.width = `${pct}%`;
    metric.bar.title = title || "";
    metric.bar.setAttribute("aria-valuenow", safeValue.toString());
    metric.bar.setAttribute("aria-valuemin", "0");
    metric.bar.setAttribute("aria-valuemax", safeMax.toString());
}

export class TreeGuardStateMixDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.status = null;
    }

    title() {
        return "State Mix";
    }

    tooltip() {
        return "<h5>TreeGuard State Mix</h5><p>Shows how much of the current topology TreeGuard is managing and how much is currently virtualized or on fq_codel.</p>";
    }

    subscribeTo() {
        return ["TreeGuardStatus"];
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        this.grid = document.createElement("div");
        this.grid.classList.add("d-flex", "flex-column", "gap-3");

        this.nodesManaged = progressMetric("Managed Nodes", "bg-primary");
        this.nodesVirtual = progressMetric("Virtualized", "bg-warning text-dark");
        this.circuitsManaged = progressMetric("Managed Circuits", "bg-success");
        this.circuitSqmMix = stackedMetric("Circuit SQM Mix");

        [
            this.nodesManaged.wrap,
            this.nodesVirtual.wrap,
            this.circuitsManaged.wrap,
            this.circuitSqmMix.wrap,
        ].forEach((el) => this.grid.appendChild(el));

        this.footerEl = document.createElement("div");
        this.footerEl.classList.add("small", "text-body-secondary", "mt-3");

        wrap.appendChild(this.grid);
        wrap.appendChild(this.footerEl);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "TreeGuardStatus") return;
        this.status = msg.data || null;
        this.renderState();
    }

    renderState() {
        const totalNodes = Number.isFinite(this.status?.total_nodes) ? this.status.total_nodes : 0;
        const totalCircuits = Number.isFinite(this.status?.total_circuits) ? this.status.total_circuits : 0;
        const managedNodes = Number.isFinite(this.status?.managed_nodes) ? this.status.managed_nodes : 0;
        const managedCircuits = Number.isFinite(this.status?.managed_circuits) ? this.status.managed_circuits : 0;
        const virtualizedNodes = Number.isFinite(this.status?.virtualized_nodes)
            ? this.status.virtualized_nodes
            : 0;
        const cakeCircuits = Number.isFinite(this.status?.cake_circuits)
            ? this.status.cake_circuits
            : 0;
        const mixedCircuits = Number.isFinite(this.status?.mixed_sqm_circuits)
            ? this.status.mixed_sqm_circuits
            : 0;
        const fqCodelCircuits = Number.isFinite(this.status?.fq_codel_circuits)
            ? this.status.fq_codel_circuits
            : 0;

        updateMetric(this.nodesManaged, managedNodes, totalNodes, "bg-primary", "Nodes currently managed by TreeGuard");
        updateMetric(this.nodesVirtual, virtualizedNodes, totalNodes, "bg-warning", "Nodes currently runtime-virtualized");
        updateMetric(this.circuitsManaged, managedCircuits, totalCircuits, "bg-success", "Circuits under TreeGuard management");
        const sqmTotal = Math.max(1, cakeCircuits + mixedCircuits + fqCodelCircuits);
        this.circuitSqmMix.count.textContent = `${cakeCircuits.toLocaleString()} / ${mixedCircuits.toLocaleString()} / ${fqCodelCircuits.toLocaleString()}`;
        [
            [this.circuitSqmMix.cakeBar, cakeCircuits, "cake/cake circuits"],
            [this.circuitSqmMix.mixedBar, mixedCircuits, "mixed cake/fq_codel circuits"],
            [this.circuitSqmMix.fqBar, fqCodelCircuits, "fq_codel/fq_codel circuits"],
        ].forEach(([bar, value, label]) => {
            const pct = Math.max(0, Math.min(100, (value / sqmTotal) * 100));
            bar.style.width = `${pct}%`;
            bar.title = `${value.toLocaleString()} ${label}`;
            bar.setAttribute("aria-valuenow", value.toString());
            bar.setAttribute("aria-valuemin", "0");
            bar.setAttribute("aria-valuemax", sqmTotal.toString());
        });

        this.footerEl.textContent = totalNodes > 0 || totalCircuits > 0
            ? "This view shows how much of the discovered topology is currently under TreeGuard influence."
            : "Waiting for TreeGuard topology totals.";
    }
}
