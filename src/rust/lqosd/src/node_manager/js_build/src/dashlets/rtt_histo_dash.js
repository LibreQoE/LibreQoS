import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";

export class RttHistoDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
        this.size = 6;
    }

    title() {
        return "Round-Trip Time Histogram";
    }

    subscribeTo() {
        return [ "rtt" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new RttHistogram(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "histogram") {
            this.graph.update(msg.data);
        }
    }
}