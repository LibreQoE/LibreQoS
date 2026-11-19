import {ShapedUnshapedPie} from "../graphs/shaped_unshaped_pie";
import {ShapedUnshapedTimescale} from "../graphs/shaped_unshaped_timescale";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {sumDownUpOrder} from "../lq_js_common/helpers/scaling";
import {t} from "../i18n/index.js";

// translatable: false — "Live" is the time-period token the page compares against, not a label

// translatable: false — "Throughput" is the websocket event name the dashlet filters on, not a label

export class ShapedUnshapedDash extends DashletBaseInsight {
    title() {
        return t("dashlet.name.mapped_unmapped_traffic");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.mapped.tooltip")}</p>`;
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

    setupZoomed() {
        const period = window.timePeriods && window.timePeriods.activePeriod ? window.timePeriods.activePeriod : "Live";
        if (period === "Live") {
            this.zoomGraph = new ShapedUnshapedPie(this.zoomGraphDivId());
        } else {
            this.zoomGraph = new ShapedUnshapedTimescale(this.zoomGraphDivId(), period);
        }
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "Throughput" && window.timePeriods.activePeriod === "Live") {
            let shaped = sumDownUpOrder(msg.data.shaped_bps, 0);
            let unshaped = sumDownUpOrder(msg.data.bps, 0);
            this.graph.update(shaped, unshaped);
            if (this.zoomGraph) {
                this.zoomGraph.update(shaped, unshaped);
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
            this.graph = new ShapedUnshapedPie(this.graphDivId());
        } else {
            this.graph = new ShapedUnshapedTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
        if (this.zoomed) {
            this.teardownZoomed();
            this.setupZoomed();
        }
    }
}
