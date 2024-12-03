import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, TopNTableFromMsgData} from "../helpers/builders";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";

export class Worst10Retransmits extends BaseDashlet {
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
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "WorstRetransmits") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r);
            });
            this.timeCache.tick();

            let items = this.timeCache.get();
            items.sort((a, b) => {
                return a.tcp_retransmits.down - b.tcp_retransmits.down;
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