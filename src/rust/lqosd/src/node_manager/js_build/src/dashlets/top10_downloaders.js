import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {theading} from "../helpers/builders";
import {scaleNumber, rttCircleSpan} from "../helpers/scaling";

export class Top10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Top 10 Downloaders";
    }

    subscribeTo() {
        return [ "top10downloaders" ];
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
        if (msg.event === "top10downloaders") {
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
                row.append(ip);

                let dl = document.createElement("td");
                dl.innerText = scaleNumber(r.bits_per_second[0]);
                row.append(dl);

                let ul = document.createElement("td");
                ul.innerText = scaleNumber(r.bits_per_second[1]);
                row.append(ul);

                let rtt = document.createElement("td");
                rtt.innerText = r.median_tcp_rtt.toFixed(2);
                row.append(rtt);

                let tcp_xmit = document.createElement("td");
                tcp_xmit.innerText = r.tcp_retransmits[0] + " / " + r.tcp_retransmits[1];
                row.append(tcp_xmit);

                let shaped = document.createElement("td");
                shaped.innerText = r.plan[0] + " / " + r.plan[1];
                row.append(shaped);

                t.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            while (target.children.length > 1) {
                target.removeChild(target.lastChild);
            }
            target.appendChild(t);
        }
    }
}