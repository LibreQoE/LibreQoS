import {TopNSankey} from "../graphs/top_n_sankey";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class Worst10RetransmitsVisual extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Worst 10 TCP Re-transmits";
    }

    tooltip() {
        return "<h5>Worst 10 TCP Re-transmits</h5><p>Worst 10 Downloaders by TCP retransmits, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
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
                this.timeCache.addOrUpdate(key, r, r.tcp_retransmits[0]);
            });
            this.timeCache.tick();

            let items = this.timeCache.get();
            this.graph.processMessage({ data: items });
        }
    }
}