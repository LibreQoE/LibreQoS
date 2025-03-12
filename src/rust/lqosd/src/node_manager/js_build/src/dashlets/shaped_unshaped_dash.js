import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {ShapedUnshapedPie} from "../graphs/shaped_unshaped_pie";
import {ShapedUnshapedTimescale} from "../graphs/shaped_unshaped_timescale";

export class ShapedUnshapedDash extends BaseDashlet{
    title() {
        return "Shaped/Unshaped Traffic";
    }

    tooltip() {
        return "<h5>Shaped/Unshaped Traffic</h5><p>Shows the amount of traffic that is shaped and unshaped. Shaped traffic is limited by the configured bandwidth limits, while unshaped traffic is not.</p>";
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
        this.graph = new ShapedUnshapedPie(this.graphDivId());
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "Throughput" && window.timePeriods.activePeriod === "Live") {
            let shaped = msg.data.shaped_bps.down + msg.data.shaped_bps.up;
            let unshaped = msg.data.bps.down + msg.data.bps.up;
            this.graph.update(shaped, unshaped);
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
            this.graph = new ShapedUnshapedPie(this.graphDivId());
        } else {
            this.graph = new ShapedUnshapedTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
    }
}