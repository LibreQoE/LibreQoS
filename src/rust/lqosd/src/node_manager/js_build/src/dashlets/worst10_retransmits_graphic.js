import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, TopNTableFromMsgData, topNTableHeader, topNTableRow} from "../helpers/builders";
import {scaleNumber, rttCircleSpan, formatRtt, formatThroughput} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {TopNSankey} from "../graphs/top_n_sankey";

export class Worst10RetransmitsVisual extends BaseDashlet {
    constructor(slot) {
        super(slot);
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
            this.graph.processMessage(msg);
        }
    }
}