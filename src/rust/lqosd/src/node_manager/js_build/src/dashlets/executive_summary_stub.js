import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";

export class ExecutiveSummaryStub extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.size = 12;
        this.lastUpdate = null;
        this._bodyId = this.id + "_body";
    }

    canBeSlowedDown() { return true; }
    title() { return "Executive Summary (Preview)"; }
    tooltip() { return "Tracks executive summary heatmap data every 30 seconds."; }
    subscribeTo() { return ["ExecutiveHeatmaps"]; }

    buildContainer() {
        const div = super.buildContainer();
        const body = document.createElement("div");
        body.id = this._bodyId;
        body.classList.add("text-muted", "small");
        body.innerText = "Waiting for executive summary data...";
        div.appendChild(body);
        return div;
    }

    setup() {
        if (this.lastUpdate) {
            this.#render();
        }
    }

    onMessage(msg) {
        if (msg.event === "ExecutiveHeatmaps") {
            window.executiveHeatmapData = msg.data;
            this.lastUpdate = new Date();
            this.#render();
        }
    }

    #render() {
        const target = document.getElementById(this._bodyId);
        if (!target) return;

        const circuits = (window.executiveHeatmapData?.circuits ?? []).length;
        const sites = (window.executiveHeatmapData?.sites ?? []).length;
        const asns = (window.executiveHeatmapData?.asns ?? []).length;
        const ts = this.lastUpdate ? this.lastUpdate.toLocaleTimeString() : "—";

        target.classList.remove("text-muted");
        target.innerHTML = `
            <div class="d-flex flex-column flex-md-row gap-2 align-items-start">
                <div><i class="fas fa-bullseye text-primary me-1"></i>Circuits: <strong>${circuits}</strong></div>
                <div><i class="fas fa-sitemap text-info me-1"></i>Sites: <strong>${sites}</strong></div>
                <div><i class="fas fa-globe text-success me-1"></i>ASNs: <strong>${asns}</strong></div>
                <div class="text-secondary ms-md-auto">Last update: ${ts}</div>
            </div>
        `;
    }
}
