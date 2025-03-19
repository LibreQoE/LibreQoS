import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {PacketsPerSecondBar} from "../graphs/packets_bar";
import {PacketsPerSecondTimescale} from "../graphs/packets_bar_insight";

export class ThroughputPpsDash extends BaseDashlet{
    title() {
        return "Throughput PPS";
    }

    tooltip() {
        return "<h5>Throughput Packets/Second</h5><p>Shows the current throughput in packets per second. Traffic is divided between upload (from the ISP) and download (to the ISP) traffic.</p>";
    }

    supportsZoom() {
        return true;
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
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "Throughput" && window.timePeriods.activePeriod === "Live") {
            this.graph.update(msg.data.pps.down, msg.data.pps.up, msg.data.tcp_pps, msg.data.udp_pps, msg.data.icmp_pps);
        }
    }

    onTimeChange() {
        super.onTimeChange();
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new PacketsPerSecondBar(this.graphDivId());
        } else {
            this.graph = new PacketsPerSecondTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
    }
}