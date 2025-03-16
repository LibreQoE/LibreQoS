import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {TopNSankey} from "../graphs/top_n_sankey";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";

export class Top10DownloadersVisual extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return "Top 10 Downloaders (Visual)";
    }

    tooltip() {
        return "<h5>Top 10 Downloaders</h5><p>The top-10 users by bits-per-second. The ribbon goes red as they approach capacity. The name border indicates round-trip time, while the main label color indicates TCP retransmits. The legend is current download speed, followed by number of retransmits.</p>";
    }

    subscribeTo() {
        return [ "TopDownloads" ];
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
        this.graph = new TopNSankey(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads") {
            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r, r.bits_per_second.down);
            });
            this.timeCache.tick();
            let items = this.timeCache.get();
            this.graph.processMessage({ data: items });
        }
    }
}