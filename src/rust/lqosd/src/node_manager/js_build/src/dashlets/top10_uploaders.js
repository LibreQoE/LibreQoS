import {clearDashDiv, TopNTableFromMsgData} from "../helpers/builders";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class Top10Uploaders extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return "Top 10 Uploaders";
    }

    tooltip() {
        return "<h5>Top 10 Uploaders</h5><p>Top 10 Uploaders by bits per second, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
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
            let t = TopNTableFromMsgData(items);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}