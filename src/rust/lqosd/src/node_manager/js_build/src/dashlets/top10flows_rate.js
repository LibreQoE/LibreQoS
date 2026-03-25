import {clearDashDiv, theading} from "../helpers/builders";
import {formatRetransmit, rttNanosAsSpan} from "../helpers/scaling";
import {RttCache} from "../helpers/rtt_cache";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";
import {TrimToFit} from "../lq_js_common/helpers/text_utils";
import {DashletBaseInsight} from "./insight_dashlet_base";

const flowCacheKey = (row) => `${row.remote_ip}|${row.analysis}`;
const MAX_VISIBLE_ASN_CHARS = 12;
const compactAsnLabel = (label) => {
    if (!label) return "";
    return label.length > MAX_VISIBLE_ASN_CHARS
        ? `${label.slice(0, MAX_VISIBLE_ASN_CHARS)}…`
        : label;
};

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
        base.classList.add("dashbox-body-scroll", "dashbox-body-scroll-top10");
        return base;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "TopFlowsRate") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("dash-table", "lqos-table", "lqos-table-compact", "small", "lqos-topflow-table");

            let th = document.createElement("thead");
            th.classList.add("small");
            const keyHeading = theading("IP/Circuit");
            keyHeading.classList.add("lqos-topflow-key-cell");
            th.appendChild(keyHeading);
            const protocolHeading = theading("Protocol");
            protocolHeading.classList.add("lqos-topflow-protocol-cell");
            th.appendChild(protocolHeading);
            const dlHeading = theading("DL ⬇️");
            dlHeading.classList.add("lqos-topflow-rate-cell");
            th.appendChild(dlHeading);
            const ulHeading = theading("UL ⬆️");
            ulHeading.classList.add("lqos-topflow-rate-cell");
            th.appendChild(ulHeading);
            const totalHeading = theading("Total");
            totalHeading.classList.add("lqos-topflow-total-cell");
            th.appendChild(totalHeading);
            const rttHeading = theading("RTT", 2);
            rttHeading.classList.add("lqos-topflow-rtt-cell");
            th.appendChild(rttHeading);
            const retransmitHeading = theading("TCP Retransmits", 2);
            retransmitHeading.classList.add("lqos-topflow-retrans-cell");
            th.appendChild(retransmitHeading);
            const asnHeading = theading("Remote ASN");
            asnHeading.classList.add("lqos-asn-cell");
            th.appendChild(asnHeading);
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                //console.log(r);
                let row = document.createElement("tr");
                row.classList.add("small");

                if (r.circuit_id !== "") {
                    let circuit = document.createElement("td");
                    circuit.classList.add("lqos-topflow-key-cell");
                    let link = document.createElement("a");
                    link.href = "circuit.html?id=" + encodeURI(r.circuit_id);
                    link.innerText = TrimToFit(r.circuit_name);
                    link.title = r.circuit_name || "";
                    link.classList.add("redactable", "lqos-table-cell-ellipsis");
                    circuit.appendChild(link);
                    row.appendChild(circuit);
                } else {
                    let localIp = document.createElement("td");
                    localIp.classList.add("lqos-topflow-key-cell");
                    const localIpText = document.createElement("span");
                    localIpText.innerText = r.local_ip;
                    localIpText.title = r.local_ip || "";
                    localIpText.classList.add("redactable", "lqos-table-cell-ellipsis");
                    localIp.appendChild(localIpText);
                    row.appendChild(localIp);
                }

                let proto = document.createElement("td");
                proto.classList.add("lqos-topflow-protocol-cell");
                const protoText = document.createElement("span");
                protoText.innerText = r.analysis;
                protoText.title = r.analysis || "";
                protoText.classList.add("lqos-table-cell-ellipsis");
                proto.appendChild(protoText);
                row.appendChild(proto);

                let dl = document.createElement("td");
                dl.classList.add("lqos-topflow-rate-cell");
                dl.innerText = scaleNumber(r.rate_estimate_bps.down, 0);
                row.appendChild(dl);

                let ul = document.createElement("td");
                ul.classList.add("lqos-topflow-rate-cell");
                ul.innerText = scaleNumber(r.rate_estimate_bps.up, 0);
                row.appendChild(ul);

                let total = document.createElement("td");
                total.classList.add("lqos-topflow-total-cell");
                total.innerText = scaleNumber(r.bytes_sent.down, 0) + " / " + scaleNumber(r.bytes_sent.up, 0);
                row.appendChild(total);

                const cacheKey = flowCacheKey(r);
                if (r.rtt_nanos['down'] !== undefined) {
                    this.rttCache.set(cacheKey, r.rtt_nanos);
                }
                let rtt = this.rttCache.get(cacheKey);
                if (rtt === 0) {
                    rtt = { down: 0, up: 0 };
                }

                let rttD = document.createElement("td");
                rttD.classList.add("lqos-topflow-rtt-cell");
                rttD.innerHTML = rttNanosAsSpan(rtt.down);
                row.appendChild(rttD);

                let rttU = document.createElement("td");
                rttU.classList.add("lqos-topflow-rtt-cell");
                rttU.innerHTML = rttNanosAsSpan(rtt.up);
                row.appendChild(rttU);

                let tcp1 = document.createElement("td");
                tcp1.classList.add("lqos-topflow-retrans-cell");
                const packetsDown = toNumber(r.packets_sent.down, 0);
                const retransmitsDown = toNumber(r.tcp_retransmits.down, 0);
                tcp1.innerHTML = formatRetransmit(packetsDown > 0 ? retransmitsDown / packetsDown : 0);
                row.appendChild(tcp1);

                let tcp2 = document.createElement("td");
                tcp2.classList.add("lqos-topflow-retrans-cell");
                const packetsUp = toNumber(r.packets_sent.up, 0);
                const retransmitsUp = toNumber(r.tcp_retransmits.up, 0);
                tcp2.innerHTML = formatRetransmit(packetsUp > 0 ? retransmitsUp / packetsUp : 0);
                row.appendChild(tcp2);

                let asn = document.createElement("td");
                asn.classList.add("lqos-asn-cell");
                const asnLabel = (r.remote_asn_name && r.remote_asn_name.length > 0) ? r.remote_asn_name : r.remote_ip;
                const asnText = document.createElement("span");
                asnText.classList.add("lqos-table-cell-ellipsis");
                if (asnLabel && asnLabel.length > MAX_VISIBLE_ASN_CHARS) {
                    asnText.classList.add("tiny");
                }
                asnText.textContent = compactAsnLabel(asnLabel || "");
                asnText.title = asnLabel || "";
                asn.appendChild(asnText);
                row.appendChild(asn);

                tbody.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}
