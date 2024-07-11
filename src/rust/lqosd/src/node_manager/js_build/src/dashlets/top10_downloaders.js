import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {clearDashDiv, theading} from "../helpers/builders";
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

            let th = document.createElement("thead");
            th.appendChild(theading("IP Address/Circuit"));
            th.appendChild(theading("Plan"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("RTT (ms)"));
            th.appendChild(theading("TCP Retransmits", 2));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                let row = document.createElement("tr");

                if (r.circuit_id !== "") {
                    let link = document.createElement("a");
                    link.href = "circuit.html?id=" + encodeURI(r.circuit_id);
                    link.innerText = r.ip_address;
                    redactCell(link);
                    row.append(link);
                } else {
                    let ip = document.createElement("td");
                    ip.innerText = r.ip_address;
                    redactCell(ip);
                    row.append(ip);
                }

                let shaped = document.createElement("td");
                shaped.classList.add("tiny");
                shaped.innerText = r.plan.down + " / " + r.plan.up;
                row.append(shaped);

                let dl = document.createElement("td");
                dl.innerHTML = formatThroughput(r.bits_per_second.down, r.plan.down);
                row.append(dl);

                let ul = document.createElement("td");
                ul.innerHTML = formatThroughput(r.bits_per_second.up, r.plan.up);
                row.append(ul);

                let rtt = document.createElement("td");
                rtt.innerHTML = formatRtt(r.median_tcp_rtt);
                row.append(rtt);

                let tcp_xmit_down = document.createElement("td");
                tcp_xmit_down.innerHTML = formatRetransmit(r.tcp_retransmits.down);
                row.append(tcp_xmit_down);

                let tcp_xmit_up = document.createElement("td");
                tcp_xmit_up.innerHTML = formatRetransmit(r.tcp_retransmits.up);
                row.append(tcp_xmit_up);

                t.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}