import {BaseDashlet} from "./base_dashlet";
import {BitsPerSecondGauge} from "../graphs/bits_gauge";

export class ThroughputBpsDash extends BaseDashlet{
    title() {
        return "Throughput Bits/Second";
    }

    tooltip() {
        return "<h5>Throughput Bits/Second</h5><p>Shows the current throughput in bits per second. Traffic is divided between upload (from the ISP) and download (to the ISP) traffic.</p>";
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