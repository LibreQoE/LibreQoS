import {BaseDashlet} from "./base_dashlet";
import {FlowCountGraph} from "../graphs/flows_graph";

export class TrackedFlowsCount extends BaseDashlet{
    title() {
        return "Tracked Flows";
    }

    subscribeTo() {
        return [ "FlowCount" ];
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
        if (msg.event === "FlowCount") {
            this.graph.update(msg.data);
        }
    }
}