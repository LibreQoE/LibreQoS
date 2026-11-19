import {StormguardAdjustmentsGraph} from "../graphs/stormguard_adjustments_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class StormguardStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.lastUpdate = {};
        this.graphDivs = [];
    }

    title() {
        return t("dashlet.name.stormguard_bandwidth_adjustments");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.stormguard_bandwidth.tooltip")}</p>`;
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
        caption.innerText = t("dashlet.stormguard_bandwidth.caption");

        let debugLink = document.createElement("a");
        debugLink.href = "stormguard_debug.html";
        debugLink.classList.add("btn", "btn-sm", "btn-outline-primary");
        debugLink.innerHTML = `<i class="fas fa-bug"></i> ${t("dashlet.stormguard.debug_view")}`;

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

    setupZoomed() {
        this.zoomGraph = new StormguardAdjustmentsGraph(this.zoomGraphDivId());
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "StormguardStatus") {
            this.lastUpdate = msg.data;
            if (msg.data && Array.isArray(msg.data)) {
                this.graph.update(msg.data);
                if (this.zoomGraph) {
                    this.zoomGraph.update(msg.data);
                }
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
