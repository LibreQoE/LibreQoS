import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading, topNTableHeader, topNTableRow} from "../helpers/builders";
import {scaleNumber, rttCircleSpan, formatThroughput, formatRtt, formatRetransmit} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";

export class Top10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
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

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("table", "table-striped", "table-sm");

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