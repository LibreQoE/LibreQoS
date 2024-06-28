import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../helpers/scaling";

export class Top10FlowsBytes extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Top 10 Flows (by total bytes)";
    }

    subscribeTo() {
        return [ "TopFlowsBytes" ];
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
        if (msg.event === "TopFlowsBytes") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("table", "table-striped", "tiny");

            let th = document.createElement("thead");
            th.appendChild(theading("Protocol"));
            th.appendChild(theading("Local IP"));
            th.appendChild(theading("Remote IP"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("Total"));
            th.appendChild(theading("⬇ RTT"));
            th.appendChild(theading("️️⬆ RTT"));
            th.appendChild(theading("TCP Retransmits"));
            th.appendChild(theading("Remote ASN"));
            th.appendChild(theading("Country"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                let row = document.createElement("tr");

                let proto = document.createElement("td");
                proto.innerText = r.analysis;
                row.appendChild(proto);

                let localIp = document.createElement("td");
                localIp.innerText = r.local_ip;
                row.appendChild(localIp);

                let remoteIp = document.createElement("td");
                remoteIp.innerText = r.remote_ip;
                row.appendChild(remoteIp);

                let dl = document.createElement("td");
                dl.innerText = scaleNumber(r.rate_estimate_bps[0]);
                row.appendChild(dl);

                let ul = document.createElement("td");
                ul.innerText = scaleNumber(r.rate_estimate_bps[1]);
                row.appendChild(ul);

                let total = document.createElement("td");
                total.innerText = scaleNumber(r.bytes_sent[0]) + " / " + scaleNumber(r.bytes_sent[1]);
                row.appendChild(total);

                let rttD = document.createElement("td");
                rttD.innerText = scaleNanos(r.rtt_nanos[0]);
                row.appendChild(rttD);

                let rttU = document.createElement("td");
                rttU.innerText = scaleNanos(r.rtt_nanos[1]);
                row.appendChild(rttU);

                let tcp = document.createElement("td");
                tcp.innerText = r.tcp_retransmits[0] + " / " + r.tcp_retransmits[1];
                row.appendChild(tcp);

                let asn = document.createElement("td");
                asn.innerText = r.remote_asn_name;
                row.appendChild(asn);

                let country = document.createElement("td");
                country.innerText = r.remote_asn_country;
                row.appendChild(country);

                t.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}