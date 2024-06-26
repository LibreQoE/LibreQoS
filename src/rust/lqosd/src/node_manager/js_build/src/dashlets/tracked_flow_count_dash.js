import {BaseDashlet} from "./base_dashlet";
import {FlowCountGraph} from "../graphs/flows_graph";

export class TrackedFlowsCount extends BaseDashlet{
    title() {
        return "Tracked Flows";
    }

    subscribeTo() {
        return [ "flowCount" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new FlowCountGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "flowCount") {
            this.graph.update(msg.data);
        }
    }
}