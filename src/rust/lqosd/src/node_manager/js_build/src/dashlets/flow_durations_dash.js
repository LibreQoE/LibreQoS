import {BaseDashlet} from "./base_dashlet";
import {FlowDurationsGraph} from "../graphs/flow_durations_graph";

export class FlowDurationDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Flow Duration";
    }

    tooltip() {
        return "<h5>Flow Duration</h5><p>How long do flows last in seconds? Most flows are very short, while some can last a very long time.</p>";
    }

    subscribeTo() {
        return [ "FlowDurations" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new FlowDurationsGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "FlowDurations") {
            this.graph.update(msg.data);
        }
    }
}