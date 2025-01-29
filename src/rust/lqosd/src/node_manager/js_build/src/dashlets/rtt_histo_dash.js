import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {RttHistogramTimeseries} from "../graphs/rtt_histo_timeseries";

export class RttHistoDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Round-Trip Time Histogram";
    }

    tooltip() {
        return "<h5>Round-Trip Time Histogram</h5><p>Round-Trip Time Histogram, showing the distribution of round-trip times for packets in real-time.</p>";
    }

    subscribeTo() {
        return [ "RttHistogram" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new RttHistogram(this.graphDivId());
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "RttHistogram" && window.timePeriods.activePeriod === "Live") {
            this.graph.update(msg.data);
        }
    }

    onTimeChange() {
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new RttHistogram(this.graphDivId());
        } else {
            this.graph = new RttHistogramTimeseries(this.graphDivId(), window.timePeriods.activePeriod);
        }
    }
}