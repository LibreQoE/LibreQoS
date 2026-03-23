import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {get_ws_client} from "../pubsub/ws";

function toneClasses(kind) {
    switch (kind) {
        case "active":
            return ["border-primary-subtle", "bg-primary-subtle", "text-primary"];
        case "ok":
            return ["border-success-subtle", "bg-success-subtle", "text-success"];
        case "warning":
            return ["border-warning-subtle", "bg-warning-subtle", "text-warning"];
        case "danger":
            return ["border-danger-subtle", "bg-danger-subtle", "text-danger"];
        default:
            return ["border-secondary-subtle", "bg-body-tertiary", "text-body-secondary"];
    }
}

function renderLoopCard(host, iconClass, title, statusText, tone, detailText) {
    host.innerHTML = "";
    host.className = "";
    host.classList.add("border", "rounded", "p-2", "h-100", "d-flex", "flex-column", "gap-1", "small");
    toneClasses(tone).forEach((cls) => host.classList.add(cls));

    const top = document.createElement("div");
    top.classList.add("d-flex", "align-items-center", "gap-2");
    const icon = document.createElement("i");
    icon.classList.add("fa", "fa-fw", iconClass);
    const titleEl = document.createElement("span");
    titleEl.classList.add("fw-semibold");
    titleEl.textContent = title;
    top.appendChild(icon);
    top.appendChild(titleEl);

    const status = document.createElement("div");
    status.classList.add("fw-semibold");
    status.textContent = statusText;

    const detail = document.createElement("div");
    detail.classList.add("text-body-secondary");
    detail.textContent = detailText;

    host.appendChild(top);
    host.appendChild(status);
    host.appendChild(detail);
}

export class TreeGuardControlLoopDashlet extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 4;
        this.lastStatus = null;
        this.metadata = null;
    }

    title() {
        return "Control Loop";
    }

    tooltip() {
        return "<h5>TreeGuard Control Loop</h5><p>Visualizes TreeGuard as a control loop: observe, evaluate, and act.</p>";
    }

    subscribeTo() {
        return ["TreeGuardStatus"];
    }

    setup() {
        const wsClient = get_ws_client();
        const wrapped = (msg) => {
            wsClient.off("TreeGuardMetadataSummary", wrapped);
            this.metadata = msg?.data || null;
            if (this.lastStatus) {
                this.renderStatus();
            }
        };
        wsClient.on("TreeGuardMetadataSummary", wrapped);
        wsClient.send({ TreeGuardMetadataSummary: {} });
    }

    buildContainer() {
        const base = super.buildContainer();
        const wrap = document.createElement("div");
        wrap.classList.add("p-2");

        const grid = document.createElement("div");
        grid.classList.add("row", "g-2");
        this.cards = [];
        for (let i = 0; i < 3; i++) {
            const col = document.createElement("div");
            col.classList.add("col-12", "col-md-4");
            const card = document.createElement("div");
            col.appendChild(card);
            grid.appendChild(col);
            this.cards.push(card);
        }

        this.footerEl = document.createElement("div");
        this.footerEl.classList.add("small", "text-body-secondary", "mt-2");

        wrap.appendChild(grid);
        wrap.appendChild(this.footerEl);
        base.appendChild(wrap);
        return base;
    }

    onMessage(msg) {
        if (msg.event !== "TreeGuardStatus") return;
        this.lastStatus = msg.data || null;
        this.renderStatus();
    }

    renderStatus() {
        const status = this.lastStatus || {};
        const totalNodes = Number.isFinite(this.metadata?.total_nodes) ? this.metadata.total_nodes : 0;
        const totalCircuits = Number.isFinite(this.metadata?.total_circuits) ? this.metadata.total_circuits : 0;
        const cpuMax = Number.isFinite(Number(status.cpu_max_pct)) ? Math.max(0, Math.trunc(Number(status.cpu_max_pct))) : null;
        const enabled = !!status.enabled;
        const dryRun = !!status.dry_run;
        const paused = !!status.paused_for_bakery_reload;

        renderLoopCard(
            this.cards[0],
            "fa-eye",
            "Observe",
            enabled ? "Watching" : "Disabled",
            enabled ? "ok" : "idle",
            `${totalNodes.toLocaleString()} nodes • ${totalCircuits.toLocaleString()} circuits`,
        );

        renderLoopCard(
            this.cards[1],
            "fa-brain",
            "Evaluate",
            paused ? "Waiting" : (dryRun ? "Dry run" : "Live decisions"),
            paused ? "warning" : (enabled ? "active" : "idle"),
            cpuMax === null ? "CPU trigger idle" : `CPU max ${cpuMax}%`,
        );

        renderLoopCard(
            this.cards[2],
            paused ? "fa-pause-circle" : "fa-bolt",
            "Act",
            paused ? "Paused for Bakery" : (dryRun ? "Would act" : "Acting live"),
            paused ? "warning" : (dryRun ? "warning" : (enabled ? "ok" : "idle")),
            (status.last_action_summary || (paused ? (status.pause_reason || "Bakery full reload in progress") : "No recent action")).toString(),
        );

        this.footerEl.textContent = paused
            ? "TreeGuard observation continues, but action is paused while Bakery is changing queue state."
            : dryRun
                ? "TreeGuard is evaluating conditions and surfacing intended actions without mutating live state."
                : "TreeGuard is free to observe, evaluate, and apply live topology or SQM decisions.";
    }
}
