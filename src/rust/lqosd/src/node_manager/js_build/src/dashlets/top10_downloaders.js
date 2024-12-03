import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, TopNTableFromMsgData} from "../helpers/builders";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";

export class Top10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return "Top 10 Downloaders";
    }

    tooltip() {
        return "<h5>Top 10 Downloaders</h5><p>Top 10 Downloaders by bits per second, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
    }

    subscribeTo() {
        return [ "TopDownloads" ];
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
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r);
            });

            let items = this.timeCache.get();
            items.sort((a, b) => {
                return b.bits_per_second.down - a.bits_per_second.down;
            });
            // Limit to 10 entries
            items = items.slice(0, 10);
            let t = TopNTableFromMsgData(items);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}
