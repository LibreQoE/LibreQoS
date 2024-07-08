import {BaseDashlet} from "./base_dashlet";
import {ThroughputRingBufferGraph} from "../graphs/throughput_ring_graph";

export class ThroughputRingDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Last 5 Minutes Throughput";
    }

    tooltip() {
        return "<h5>Last 5 Minutes Throughput</h5><p>Shaped (AQM controlled and limited) and Unshaped (not found in your Shaped Devices file) traffic over the last five minutes.</p>"
    }

    subscribeTo() {
        return [ "Throughput" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new ThroughputRingBufferGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Throughput") {
            this.graph.update(msg.data.shaped_bps, msg.data.bps);
        }
    }
}