import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, TopNTableFromMsgData, topNTableHeader, topNTableRow} from "../helpers/builders";
import {scaleNumber, rttCircleSpan, formatThroughput, formatRtt, formatRetransmit} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {TimedLRUCache} from "../helpers/timed_lru_cache";

export class Top10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.buffer = new TimedLRUCache(10, 300);
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
                this.buffer.set(r.circuit_id, r);
            });
            this.buffer.tick();

            let results = { data: this.buffer.toArray() };
            //results.sort((a, b) => a.bits_per_second.down - b.bits_per_second.down);

            let t = TopNTableFromMsgData(msg);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}
