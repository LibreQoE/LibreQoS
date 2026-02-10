import {ShapedUnshapedPie} from "../graphs/shaped_unshaped_pie";
import {ShapedUnshapedTimescale} from "../graphs/shaped_unshaped_timescale";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class ShapedUnshapedDash extends DashletBaseInsight {
    title() {
        return "Mapped/Unmapped Traffic";
    }

    tooltip() {
        return "<h5>Mapped/Unmapped Traffic</h5><p>Shows the amount of traffic that is mapped (shaped) and unmapped. Mapped traffic follows configured bandwidth limits; unmapped traffic is not.</p>";
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
