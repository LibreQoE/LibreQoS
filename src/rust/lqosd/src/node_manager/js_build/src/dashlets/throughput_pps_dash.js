import {BaseDashlet} from "./base_dashlet";
import {PacketsPerSecondBar} from "../graphs/packets_bar";

export class ThroughputPpsDash extends BaseDashlet{
    title() {
        return "Throughput Packets/Second";
    }

    tooltip() {
        return "<h5>Throughput Packets/Second</h5><p>Shows the current throughput in packets per second. Traffic is divided between upload (from the ISP) and download (to the ISP) traffic.</p>";
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
        this.graph = new PacketsPerSecondBar(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Throughput") {
            this.graph.update(msg.data.pps.down, msg.data.pps.up, msg.data.tcp_pps, msg.data.udp_pps, msg.data.icmp_pps);
        }
    }
}