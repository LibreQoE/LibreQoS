import {BakeryCircuitsGraph} from "../graphs/bakery_circuits_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class BakeryStatusDashlet extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.lastUpdate = {};
        this.graphDivs = [];
    }

    title() {
        return "Bakery Circuit Activity";
    }

    tooltip() {
        return "<h5>Bakery Circuit Activity</h5><p>Real-time visualization of active and lazy circuits over the last 5 minutes. Shows how the queue management system is activating and deactivating circuits based on traffic.</p>";
    }

    subscribeTo() {
        return ["BakeryStatus"];
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
        this.graph = new BakeryCircuitsGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "BakeryStatus") {
            this.lastUpdate = msg.data;
            if (msg.data && msg.data.currentState) {
                this.graph.update(
                    msg.data.currentState.activeCircuits,
                    msg.data.currentState.lazyCircuits
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