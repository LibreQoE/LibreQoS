import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading} from "../helpers/builders";
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

            let th = document.createElement("thead");
            th.appendChild(theading(""));
            th.appendChild(theading("IP Address/Circuit"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("RTT (ms)"));
            th.appendChild(theading("TCP Retransmits"));
            th.appendChild(theading("Shaped"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                let row = document.createElement("tr");

                let circle = document.createElement("td");
                circle.appendChild(rttCircleSpan(r.median_tcp_rtt));
                row.appendChild(circle);

                let ip = document.createElement("td");
                ip.innerText = r.ip_address;
                redactCell(ip);
                row.append(ip);

                let dl = document.createElement("td");
                dl.innerHTML = formatThroughput(r.bits_per_second.down, r.plan.down);
                row.append(dl);

                let ul = document.createElement("td");
                ul.innerHTML = formatThroughput(r.bits_per_second.up, r.plan.up);
                row.append(ul);

                let rtt = document.createElement("td");
                rtt.innerHTML = formatRtt(r.median_tcp_rtt);
                row.append(rtt);

                let tcp_xmit = document.createElement("td");
                tcp_xmit.innerText = r.tcp_retransmits.down + " / " + r.tcp_retransmits.up;
                row.append(tcp_xmit);

                let shaped = document.createElement("td");
                shaped.innerText = r.plan.down + " / " + r.plan.up;
                row.append(shaped);

                t.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}