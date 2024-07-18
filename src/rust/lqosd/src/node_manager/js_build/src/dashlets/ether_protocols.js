import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../helpers/scaling";

export class EtherProtocols extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Ethernet Protocols";
    }

    tooltip() {
        return "<h5>Ethernet Protocols</h5><p>Bytes and packets transferred over IPv4 and IPv6, and the round-trip time for each.  This data is gathered from recently completed flows, and may be a little behind realtime.</p>";
    }

    subscribeTo() {
        return [ "EtherProtocols" ];
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
        if (msg.event === "EtherProtocols") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("table", "table-sm", "small");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading("Protocol"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("Packets ⬇️"));
            th.appendChild(theading("Packets ⬆️"));
            th.appendChild(theading("⬇ RTT"));
            th.appendChild(theading("️️⬆ RTT"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            let row = document.createElement("tr");
            row.classList.add("small");
            row.appendChild(simpleRow("IPv4"));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_bytes.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_bytes.up)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_packets.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_packets.up)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v4_rtt.down)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v4_rtt.up)));
            t.appendChild(row);

            row = document.createElement("tr");
            row.appendChild(simpleRow("IPv6"));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_bytes.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_bytes.up)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_packets.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_packets.up)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v6_rtt.down)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v6_rtt.up)));
            t.appendChild(row);

            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}