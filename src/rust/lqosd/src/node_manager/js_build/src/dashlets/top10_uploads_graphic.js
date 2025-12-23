import {TopNSankey} from "../graphs/top_n_sankey";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class Top10UploadersVisual extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return "Top 10 Uploaders (Visual)";
    }

    tooltip() {
        return "<h5>Top 10 Uploaders</h5><p>The top-10 users by bits-per-second. The ribbon goes red as they approach capacity. The name border indicates round-trip time, while the main label color indicates TCP retransmits. The legend is current upload speed, followed by number of retransmits.</p>";
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
