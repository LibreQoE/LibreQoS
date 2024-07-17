import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, topNTableHeader, topNTableRow} from "../helpers/builders";
import {scaleNumber, rttCircleSpan, formatRtt, formatThroughput} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";

export class Worst10Retransmits extends BaseDashlet {
    constructor(slot) {
        super(slot);
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

            let t = document.createElement("table");
            t.classList.add("table", "table-striped", "tiny");

            t.appendChild(topNTableHeader());

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                t.appendChild(topNTableRow(r));
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}