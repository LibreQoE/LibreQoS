import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, TopNTableFromMsgData, topNTableHeader, topNTableRow} from "../helpers/builders";
import {scaleNumber, rttCircleSpan, formatThroughput, formatRtt} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {TimedLRUCache} from "../helpers/timed_lru_cache";

export class Worst10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.buffer = new TimedLRUCache(10, 300);
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
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "WorstRTT") {
            let target = document.getElementById(this.id);
            msg.data.forEach((r) => {
                this.buffer.set(r.circuit_id, r);
            });
            this.buffer.tick();

            let results = this.buffer.toArray();
            //results.sort((a, b) => b.median_tcp_rtt - a.median_tcp_rtt);

            let t = TopNTableFromMsgData(msg);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}