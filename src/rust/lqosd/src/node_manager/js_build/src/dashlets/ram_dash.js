import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {RamPie} from "../graphs/ram_pie";

export class RamDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "RAM Utilization";
    }

    tooltip() {
        return "<h5>RAM Utilization</h5><p>Percentage of RAM used and free. This includes both LibreQoS and anything else running on the server.</p>";
    }

    supportsZoom() {
        return true;
    }

    subscribeTo() {
        return [ "Ram" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new RamPie(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Ram") {
            let total = toNumber(msg.data.total, 0);
            let used = toNumber(msg.data.used, 0);
            let free = Math.max(0, total - used);
            this.graph.update(free, used);
        }
    }
}
