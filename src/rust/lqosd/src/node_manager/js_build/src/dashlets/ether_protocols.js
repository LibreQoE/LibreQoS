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
            t.classList.add("table", "table-striped", "tiny");

            let th = document.createElement("thead");
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
            row.appendChild(simpleRow("IPv4"));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_bytes[0])));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_bytes[1])));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_packets[0])));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_packets[1])));
            row.appendChild(simpleRow(scaleNanos(msg.data.v4_rtt[0])));
            row.appendChild(simpleRow(scaleNanos(msg.data.v4_rtt[1])));
            t.appendChild(row);

            row = document.createElement("tr");
            row.appendChild(simpleRow("IPv6"));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_bytes[0])));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_bytes[1])));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_packets[0])));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_packets[1])));
            row.appendChild(simpleRow(scaleNanos(msg.data.v6_rtt[0])));
            row.appendChild(simpleRow(scaleNanos(msg.data.v6_rtt[1])));
            t.appendChild(row);

            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}