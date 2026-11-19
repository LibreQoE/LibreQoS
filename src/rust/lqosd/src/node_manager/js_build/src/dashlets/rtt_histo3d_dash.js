import {RttHistogram3D} from "../graphs/rtt_histo_3d";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class RttHisto3dDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
    }

    title() {
        return t("dashlet.name.round_trip_time_histogram_3d");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.rtt_histo3d.tooltip")}</p>`;
    }

    supportsZoom() {
        return true;
    }

    subscribeTo() {
        return [ "RttHistogram" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        let gd = this.graphDiv();
        gd.style.height = "500px";
        base.appendChild(gd);
        return base;
    }

    setup() {
        super.setup();
        this.graph = new RttHistogram3D(this.graphDivId());
    }

    setupZoomed() {
        this.zoomGraph = new RttHistogram3D(this.zoomGraphDivId());
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "RttHistogram") {
            this.graph.update(msg.data);
            if (this.zoomGraph) {
                this.zoomGraph.update(msg.data);
            }
        }
    }
}
