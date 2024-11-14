import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, TopNTableFromMsgData, topNTableHeader, topNTableRow} from "../helpers/builders";
import {scaleNumber, rttCircleSpan, formatThroughput, formatRtt} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {TimedLRUCache, TopXTable} from "../helpers/top_x_cache";

export class Worst10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.buffer = new TopXTable(10, 10);
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
                this.buffer.push(r.circuit_id, r, (data) => {
                    return data.median_tcp_rtt;
                });
            });
            let ranked = this.buffer.getRankedArrayAndClean();

            let t = TopNTableFromMsgData(ranked);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}