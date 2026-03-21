import {PacketsPerSecondBar} from "../graphs/packets_bar";
import {PacketsPerSecondTimescale} from "../graphs/packets_bar_insight";
import {DashletBaseInsight} from "./insight_dashlet_base";

const LIVE_SAMPLE_LIMIT = 300;

export class ThroughputPpsDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
        this.backgroundSamples = [];
    }

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

    keepAliveWhenHidden() {
        return true;
    }

    keepSubscribedWhenHidden() {
        return true;
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

    setupZoomed() {
        const period = window.timePeriods && window.timePeriods.activePeriod ? window.timePeriods.activePeriod : "Live";
        if (period === "Live") {
            this.zoomGraph = new PacketsPerSecondBar(this.zoomGraphDivId());
        } else {
            this.zoomGraph = new PacketsPerSecondTimescale(this.zoomGraphDivId(), period);
        }
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "Throughput" && window.timePeriods.activePeriod === "Live") {
            this.#applyLiveSample(msg.data.pps.down, msg.data.pps.up, msg.data.tcp_pps, msg.data.udp_pps, msg.data.icmp_pps);
        }
    }

    onBackgroundMessage(msg) {
        if (msg.event !== "Throughput" || window.timePeriods.activePeriod !== "Live") {
            return;
        }
        this.backgroundSamples.push({
            down: msg.data.pps.down,
            up: msg.data.pps.up,
            tcp: msg.data.tcp_pps,
            udp: msg.data.udp_pps,
            icmp: msg.data.icmp_pps,
        });
        if (this.backgroundSamples.length > LIVE_SAMPLE_LIMIT) {
            this.backgroundSamples.shift();
        }
    }

    flushBackgroundMessages() {
        if (window.timePeriods.activePeriod !== "Live" || this.backgroundSamples.length === 0) {
            this.backgroundSamples = [];
            return false;
        }
        for (let i = 0; i < this.backgroundSamples.length; i++) {
            const sample = this.backgroundSamples[i];
            this.#applyLiveSample(sample.down, sample.up, sample.tcp, sample.udp, sample.icmp);
        }
        this.backgroundSamples = [];
        return true;
    }

    onTimeChange() {
        super.onTimeChange();
        this.backgroundSamples = [];
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new PacketsPerSecondBar(this.graphDivId());
        } else {
            this.graph = new PacketsPerSecondTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
        if (this.zoomed) {
            this.teardownZoomed();
            this.setupZoomed();
        }
    }

    #applyLiveSample(down, up, tcp, udp, icmp) {
        this.graph.update(down, up, tcp, udp, icmp);
        if (this.zoomGraph) {
            this.zoomGraph.update(down, up, tcp, udp, icmp);
        }
    }
}
