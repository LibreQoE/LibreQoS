import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {RttHistogram3D} from "../graphs/rtt_histo_3d";

export class RttHisto3dDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Round-Trip Time Histogram 3D";
    }

    tooltip() {
        return "<h5>Round-Trip Time Histogram 3D</h5><p>Round-Trip Time Histogram, expanded to include time as a third dimension. This can be helpful for seeing how your performance is changing over time.</p>";
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

    onMessage(msg) {
        if (msg.event === "RttHistogram") {
            this.graph.update(msg.data);
        }
    }
}