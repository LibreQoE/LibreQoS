import {StormguardAdjustmentsGraph} from "../graphs/stormguard_adjustments_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class StormguardStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.lastUpdate = {};
        this.graphDivs = [];
    }

    title() {
        return "Stormguard Bandwidth Adjustments";
    }

    tooltip() {
        return "<h5>Stormguard Bandwidth Adjustments</h5><p>Real-time visualization of bandwidth optimization decisions over the last 5 minutes. Shows increases (positive) and decreases (negative) with animated indicators for new adjustments.</p>";
    }

    subscribeTo() {
        return ["StormguardStatus"];
    }

    buildContainer() {
        let base = super.buildContainer();
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
            if (msg.data && msg.data.perCycle) {
                this.graph.update(
                    msg.data.perCycle.adjustmentsUp,
                    msg.data.perCycle.adjustmentsDown,
                    msg.data.perCycle.sitesEvaluated
                );
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