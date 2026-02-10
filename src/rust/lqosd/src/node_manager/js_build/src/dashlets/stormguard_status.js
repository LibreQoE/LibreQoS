import {StormguardAdjustmentsGraph} from "../graphs/stormguard_adjustments_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class StormguardStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.lastUpdate = {};
        this.graphDivs = [];
    }

    title() {
        return "Stormguard Bandwidth";
    }

    tooltip() {
        return "<h5>Stormguard Bandwidth</h5><p>Real-time visualization of bandwidth limits for monitored sites over the last 5 minutes. Shows download bandwidth (solid lines above zero) and upload bandwidth (dashed lines below zero) for each site.</p>";
    }

    subscribeTo() {
        return ["StormguardStatus"];
    }

    buildContainer() {
        let base = super.buildContainer();
        let controls = document.createElement("div");
        controls.classList.add("d-flex", "justify-content-between", "align-items-center", "mb-2");

        let caption = document.createElement("div");
        caption.classList.add("text-muted", "small");
        caption.innerText = "Live StormGuard queue limits";

        let debugLink = document.createElement("a");
        debugLink.href = "stormguard_debug.html";
        debugLink.classList.add("btn", "btn-sm", "btn-outline-primary");
        debugLink.innerHTML = "<i class='fas fa-bug'></i> Debug view";

        controls.appendChild(caption);
        controls.appendChild(debugLink);
        base.appendChild(controls);

        let graphs = this.graphDiv();
        
        base.appendChild(graphs);
        this.graphDivs.forEach((g) => {
            base.appendChild(g);
        });
        return base;
    }

    setup() {
        super.setup();
        this.graph = new StormguardAdjustmentsGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "StormguardStatus") {
            this.lastUpdate = msg.data;
            if (msg.data && Array.isArray(msg.data)) {
                this.graph.update(msg.data);
            }
        }
    }

    canBeSlowedDown() {
        return false; // Always update in real-time
    }

    supportsZoom() {
        return true;
    }
}
