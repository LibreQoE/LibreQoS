import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {FlowCountGraph} from "../graphs/flows_graph";
import {FlowCountGraphTimescale} from "../graphs/flows_graph_timeseries";

export class TrackedFlowsCount extends BaseDashlet{
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

    onMessage(msg) {
        if (msg.event === "FlowCount" && window.timePeriods.activePeriod === "Live") {
            this.graph.update(msg.active, msg.recent);
        }
    }

    onTimeChange() {
        super.onTimeChange();
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new FlowCountGraph(this.graphDivId());
        } else {
            this.graph = new FlowCountGraphTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
    }
}