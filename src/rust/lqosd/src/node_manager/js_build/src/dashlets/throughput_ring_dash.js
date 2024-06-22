import {BaseDashlet} from "./base_dashlet";
import {ThroughputRingBufferGraph} from "../graphs/throughput_ring_graph";

export class ThroughputRingDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Last 5 Minutes Throughput";
    }

    subscribeTo() {
        return [ "throughput" ];
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
        if (msg.event === "throughput") {
            let shaped = msg.data.shaped_bps[0] + msg.data.shaped_bps[1];
            let unshaped = msg.data.bps[0] + msg.data.bps[1];
            this.graph.update(msg.data.shaped_bps, msg.data.bps);
        }
    }
}