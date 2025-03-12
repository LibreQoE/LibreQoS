import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {TopNSankey} from "../graphs/top_n_sankey";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";

export class Worst10DownloadersVisual extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Worst 10 Round-Trip Time";
    }

    tooltip() {
        return "<h5>Worst 10 Round-Trip Time</h5><p>Worst 10 Downloaders by round-trip time, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
    }

    subscribeTo() {
        return [ "WorstRTT" ];
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
        if (msg.event === "WorstRTT") {
            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r, r.median_tcp_rtt);
            });
            this.timeCache.tick();
            let items = this.timeCache.get();
            this.graph.processMessage({ data: items });
        }
    }
}