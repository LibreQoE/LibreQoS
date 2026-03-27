import {FlowCountGraph} from "../graphs/flows_graph";
import {FlowCountGraphTimescale} from "../graphs/flows_graph_timeseries";
import {DashletBaseInsight} from "./insight_dashlet_base";

const LIVE_SAMPLE_LIMIT = 300;

export class TrackedFlowsCount extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
        this.backgroundSamples = [];
    }

    title() {
        return "Tracked Flows";
    }

    tooltip() {
        return "<h5>Tracked Flows</h5><p>Number of flows tracked by LibreQoS. Flows are either a TCP connection, or a UDP/ICMP connection with matching endpoints and port/request type numbers. Completed flows are flows that have finished transmitting data, and have been submitted to netflow and the flow analysis system.</p>";
    }

    supportsZoom() {
        return true;
    }

    subscribeTo() {
        return [ "FlowCount" ];
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
        this.graph = new FlowCountGraph(this.graphDivId());
        window.timeGraphs.push(this);
    }

    setupZoomed() {
        const period = window.timePeriods && window.timePeriods.activePeriod ? window.timePeriods.activePeriod : "Live";
        if (period === "Live") {
            this.zoomGraph = new FlowCountGraph(this.zoomGraphDivId());
        } else {
            this.zoomGraph = new FlowCountGraphTimescale(this.zoomGraphDivId(), period);
        }
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "FlowCount" && window.timePeriods.activePeriod === "Live") {
            this.#applyLiveSample(msg.active, msg.recent);
        }
    }

    onBackgroundMessage(msg) {
        if (msg.event !== "FlowCount" || window.timePeriods.activePeriod !== "Live") {
            return;
        }
        this.backgroundSamples.push({
            active: msg.active,
            recent: msg.recent,
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
            this.#applyLiveSample(sample.active, sample.recent);
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
            this.graph = new FlowCountGraph(this.graphDivId());
        } else {
            this.graph = new FlowCountGraphTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
        if (this.zoomed) {
            this.teardownZoomed();
            this.setupZoomed();
        }
    }

    #applyLiveSample(active, recent) {
        this.graph.update(active, recent);
        if (this.zoomGraph) {
            this.zoomGraph.update(active, recent);
        }
    }
}
