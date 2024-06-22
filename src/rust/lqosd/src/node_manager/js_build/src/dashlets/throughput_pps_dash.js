import {BaseDashlet} from "./base_dashlet";
import {PacketsPerSecondBar} from "../graphs/packets_bar";

export class ThroughputPpsDash extends BaseDashlet{
    title() {
        return "Throughput Packets/Second";
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
        this.graph = new PacketsPerSecondBar(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "throughput") {
            this.graph.update(msg.data.pps[0], msg.data.pps[1]);
        }
    }
}