import {ThroughputRingBufferGraph} from "../graphs/throughput_ring_graph";
import {ThroughputRingBufferGraphTimescale} from "../graphs/throughput_ring_graph_timescale";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class ThroughputRingDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
        this.counter = 0;
    }

    title() {
        return "Last 5 Minutes Traffic";
    }

    tooltip() {
        return "<h5>Last 5 Minutes Throughput</h5><p>Mapped (AQM controlled and limited) and Unmapped (not found in your Shaped Devices file) traffic over the last five minutes.</p>"
    }

    subscribeTo() {
        return [ "Throughput" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        let graphs = this.graphDiv();

        // Add some time controls
        base.classList.add("dashlet-with-controls");
        let controls = document.createElement("div");
        controls.classList.add("dashgraph-controls", "small");

        base.appendChild(controls);
        base.appendChild(graphs);
        this.graphDivs.forEach((g) => {
            base.appendChild(g);
        });
        return base;
    }

    setup() {
        super.setup();
        this.graph = new ThroughputRingBufferGraph(this.graphDivId());
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "Throughput" && window.timePeriods.activePeriod === "Live") {
            this.graph.update(msg.data.shaped_bps, msg.data.bps);

            this.counter++;
            if (this.counter > 120) {
                // Reload the LTS graphs every 2 minutes
                this.counter = 0;
                this.ltsLoaded = false;
            }
        }
    }

    supportsZoom() {
        return true;
    }

    onTimeChange() {
        super.onTimeChange();
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new ThroughputRingBufferGraph(this.graphDivId());
        } else {
            this.graph = new ThroughputRingBufferGraphTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
    }
}
