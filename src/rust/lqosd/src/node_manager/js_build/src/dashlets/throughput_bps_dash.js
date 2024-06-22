import {BaseDashlet} from "./base_dashlet";
import {BitsPerSecondGauge} from "../graphs/bits_gauge";

export class ThroughputBpsDash extends BaseDashlet{
    title() {
        return "Throughput Bits/Second";
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
        this.graph = new BitsPerSecondGauge(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "throughput") {
            this.graph.update(msg.data.bps[0], msg.data.bps[1], msg.data.max[0], msg.data.max[1]);
        }
    }
}