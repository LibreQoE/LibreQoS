import {BaseDashlet} from "./base_dashlet";
import {BitsPerSecondGauge} from "../graphs/bits_gauge";

export class ThroughputBpsDash extends BaseDashlet{
    title() {
        return "Throughput Bits/Second";
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
        this.graph = new BitsPerSecondGauge(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Throughput") {
            this.graph.update(msg.data.bps.down, msg.data.bps.up, msg.data.max.down, msg.data.max.up);
        }
    }
}