import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {get_ws_client} from "../pubsub/ws";

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

function updateMetric(metric, value, max, colorClass, title) {
    const safeMax = Math.max(1, Number.isFinite(max) ? max : 1);
    const safeValue = Number.isFinite(value) ? value : 0;
    const pct = Math.max(0, Math.min(100, (safeValue / safeMax) * 100));
    metric.count.textContent = `${safeValue.toLocaleString()} / ${max.toLocaleString()}`;
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
        this.metadata = null;
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

    setup() {
        const wsClient = get_ws_client();
        const wrapped = (msg) => {
            wsClient.off("TreeGuardMetadataSummary", wrapped);
            this.metadata = msg?.data || null;
            this.renderState();
        };
        wsClient.on("TreeGuardMetadataSummary", wrapped);
        wsClient.send({ TreeGuardMetadataSummary: {} });
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
        this.circuitsFq = progressMetric("fq_codel", "bg-info text-dark");

        [
            this.nodesManaged.wrap,
            this.nodesVirtual.wrap,
            this.circuitsManaged.wrap,
            this.circuitsFq.wrap,
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
        const totalNodes = Number.isFinite(this.metadata?.total_nodes) ? this.metadata.total_nodes : 0;
        const totalCircuits = Number.isFinite(this.metadata?.total_circuits) ? this.metadata.total_circuits : 0;
        const managedNodes = Number.isFinite(this.status?.managed_nodes) ? this.status.managed_nodes : 0;
        const managedCircuits = Number.isFinite(this.status?.managed_circuits) ? this.status.managed_circuits : 0;
        const virtualizedNodes = Number.isFinite(this.status?.virtualized_nodes)
            ? this.status.virtualized_nodes
            : (Number.isFinite(this.metadata?.virtualized_nodes) ? this.metadata.virtualized_nodes : 0);
        const fqCodelCircuits = Number.isFinite(this.status?.fq_codel_circuits)
            ? this.status.fq_codel_circuits
            : (Number.isFinite(this.metadata?.fq_codel_circuits) ? this.metadata.fq_codel_circuits : 0);

        updateMetric(this.nodesManaged, managedNodes, totalNodes, "bg-primary", "Nodes currently managed by TreeGuard");
        updateMetric(this.nodesVirtual, virtualizedNodes, totalNodes, "bg-warning", "Nodes currently runtime-virtualized");
        updateMetric(this.circuitsManaged, managedCircuits, totalCircuits, "bg-success", "Circuits under TreeGuard management");
        updateMetric(this.circuitsFq, fqCodelCircuits, totalCircuits, "bg-info", "Circuits with fq_codel override");

        this.footerEl.textContent = totalNodes > 0 || totalCircuits > 0
            ? "This view shows how much of the discovered topology is currently under TreeGuard influence."
            : "Waiting for TreeGuard metadata.";
    }
}
