import {clearDashDiv, TopNTableFromMsgData} from "../helpers/builders";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

// translatable: false — "Live" is the time-period token the page compares against, not a label

export class Top10Uploaders extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return t("dashlet.name.top_10_uploaders");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.top10.uploaders.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "TopUploads" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    canBeSlowedDown() {
        return true;
    }

    setup() {
        super.setup();
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "TopUploads" && window.timePeriods.activePeriod === "Live") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r, r.bits_per_second.up);
            });
            this.timeCache.tick();

            let items = this.timeCache.get();
            let table = TopNTableFromMsgData(items);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}