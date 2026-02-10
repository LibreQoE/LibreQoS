import {clearDashDiv, theading} from "../helpers/builders";
import {formatRetransmit, rttNanosAsSpan} from "../helpers/scaling";
import {RttCache} from "../helpers/rtt_cache";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";
import {TrimToFit} from "../lq_js_common/helpers/text_utils";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class Top10FlowsRate extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.rttCache = new RttCache();
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Top 10 Flows (by rate)";
    }

    tooltip() {
        return "<h5>Top 10 Flows (by rate)</h5><p>Top 10 Flows by rate, including protocol, local and remote IP addresses, download and upload rates, total bytes, round-trip time, TCP retransmits, remote ASN, and country.</p>";
    }

    subscribeTo() {
        return [ "TopFlowsRate" ];
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
        if (msg.event === "TopFlowsRate") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("dash-table", "table-sm", "small");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading("IP/Circuit"));
            th.appendChild(theading("Protocol"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("Total"));
            th.appendChild(theading("RTT", 2));
            th.appendChild(theading("TCP Retransmits", 2));
            th.appendChild(theading("Remote ASN"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                //console.log(r);
                let row = document.createElement("tr");
                row.classList.add("small");

                if (r.circuit_id !== "") {
                    let circuit = document.createElement("td");
                    let link = document.createElement("a");
                    link.href = "circuit.html?id=" + encodeURI(r.circuit_id);
                    link.innerText = TrimToFit(r.circuit_name);
                    link.classList.add("redactable");
                    circuit.appendChild(link);
                    row.appendChild(circuit);
                } else {
                    let localIp = document.createElement("td");
                    localIp.innerText = r.local_ip;
                    row.appendChild(localIp);
                }

                let proto = document.createElement("td");
                proto.innerText = r.analysis;
                row.appendChild(proto);

                let dl = document.createElement("td");
                dl.innerText = scaleNumber(r.rate_estimate_bps.down, 0);
                row.appendChild(dl);

                let ul = document.createElement("td");
                ul.innerText = scaleNumber(r.rate_estimate_bps.up, 0);
                row.appendChild(ul);

                let total = document.createElement("td");
                total.innerText = scaleNumber(r.bytes_sent.down, 0) + " / " + scaleNumber(r.bytes_sent.up, 0);
                row.appendChild(total);

                if (r.rtt_nanos['down'] !== undefined) {
                    this.rttCache.set(r.remote_ip + r.analysis, r.rtt_nanos);
                }
                let rtt = this.rttCache.get(r.remote_ip + r.analysis);
                if (rtt === 0) {
                    rtt = { down: 0, up: 0 };
                }

                let rttD = document.createElement("td");
                rttD.innerHTML = rttNanosAsSpan(rtt.down);
                row.appendChild(rttD);

                let rttU = document.createElement("td");
                rttU.innerHTML = rttNanosAsSpan(rtt.up);
                row.appendChild(rttU);

                let tcp1 = document.createElement("td");
                const packetsDown = toNumber(r.packets_sent.down, 0);
                const retransmitsDown = toNumber(r.tcp_retransmits.down, 0);
                tcp1.innerHTML = formatRetransmit(packetsDown > 0 ? retransmitsDown / packetsDown : 0);
                row.appendChild(tcp1);

                let tcp2 = document.createElement("td");
                const packetsUp = toNumber(r.packets_sent.up, 0);
                const retransmitsUp = toNumber(r.tcp_retransmits.up, 0);
                tcp2.innerHTML = formatRetransmit(packetsUp > 0 ? retransmitsUp / packetsUp : 0);
                row.appendChild(tcp2);

                let asn = document.createElement("td");
                asn.innerText = r.remote_asn_name;
                if (asn.innerText === "") {
                    asn.innerText = r.remote_ip;
                }
                if (asn.innerText.length > 13) {
                    asn.classList.add("tiny");
                }
                row.appendChild(asn);

                t.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}
