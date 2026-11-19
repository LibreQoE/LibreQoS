import {TopNSankey} from "../graphs/top_n_sankey";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {retransmitFractionFromSample} from "../helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class Worst10RetransmitsVisual extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return t("dashlet.name.worst_10_retransmits_visual");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.worst10.retransmits.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "WorstRetransmits" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new TopNSankey(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "WorstRetransmits") {
            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(
                    key,
                    r,
                    retransmitFractionFromSample(r.tcp_retransmit_sample?.down)
                );
            });
            this.timeCache.tick();

            let items = this.timeCache.get();
            this.graph.processMessage({ data: items });
        }
    }
}
