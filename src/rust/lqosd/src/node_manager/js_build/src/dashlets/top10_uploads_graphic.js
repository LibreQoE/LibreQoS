import {TopNSankey} from "../graphs/top_n_sankey";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class Top10UploadersVisual extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return t("dashlet.name.top_10_uploaders_visual");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.top10.uploads_graphic.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "TopUploads" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    canBeSlowedDown() {
        return true;
    }

    setup() {
        super.setup();
        this.graph = new TopNSankey(this.graphDivId(), true);
    }

    onMessage(msg) {
        if (msg.event === "TopUploads") {
            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r, r.bits_per_second.up);
            });
            this.timeCache.tick();
            let items = this.timeCache.get();
            this.graph.processMessage({ data: items });
        }
    }
}
