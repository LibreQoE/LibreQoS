import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";

export class RttHistoDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Round-Trip Time Histogram";
    }

    subscribeTo() {
        return [ "rttHistogram" ];
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
        if (msg.event === "rttHistogram") {
            this.graph.update(msg.data);
        }
    }
}