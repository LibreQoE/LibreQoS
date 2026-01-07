import {clearDashDiv, simpleRow, simpleRowHtml, theading} from "../helpers/builders";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";
import {RttCache} from "../helpers/rtt_cache";
import {formatRetransmit, formatRtt, rttNanosAsSpan} from "../helpers/scaling";
import {TrimToFit} from "../lq_js_common/helpers/text_utils";
import {periodNameToSeconds} from "../helpers/time_periods";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {get_ws_client} from "../pubsub/ws";

const wsClient = get_ws_client();
const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class Top10FlowsBytes extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.rttCache = new RttCache();
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Top 10 Flows (by total bytes)";
    }

    tooltip() {
        return "<h5>Top 10 Flows (by total bytes)</h5><p>Top 10 Flows by total bytes, including protocol, local and remote IP addresses, download and upload rates, total bytes, round-trip time, TCP retransmits, remote ASN, and country.</p>";
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
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "TopFlowsBytes" && window.timePeriods.activePeriod === "Live") {
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

    onTimeChange() {
        super.onTimeChange();
        let seconds = periodNameToSeconds(window.timePeriods.activePeriod);
        let spinnerDiv = document.createElement("div");
        spinnerDiv.innerHTML = "<i class='fas fa-spinner fa-spin'></i> Fetching Insight Data...";
        let target = document.getElementById(this.id);
        clearDashDiv(this.id, target);
        target.appendChild(spinnerDiv);
        listenOnceForSeconds("LtsTopFlows", seconds, (msg) => {
            const data = msg && msg.data ? msg.data : [];
            let target = document.getElementById(this.id);

            let table = document.createElement("table");
            table.classList.add("table", "table-sm", "small");
            let thead = document.createElement("thead");
            thead.appendChild(theading("Circuit"));
            thead.appendChild(theading("Protocol"));
            thead.appendChild(theading("DL"));
            thead.appendChild(theading("UL"));
            thead.appendChild(theading("RTT DL"));
            thead.appendChild(theading("RTT UL"));
            thead.appendChild(theading("Rxmits DL"));
            thead.appendChild(theading("Rxmits UL"));
            thead.appendChild(theading("ASN"));
            table.appendChild(thead);
            let tbody = document.createElement("tbody");

            data.forEach((row) => {
                let tr = document.createElement("tr");
                tr.classList.add("small");
                tr.appendChild(simpleRow(row.circuit_name));
                tr.appendChild(simpleRow(row.protocol));
                tr.appendChild(simpleRowHtml(scaleNumber(row.bytes_down)));
                tr.appendChild(simpleRowHtml(scaleNumber(row.bytes_up)));
                if (row.rtt_down === null) {
                    tr.appendChild(simpleRowHtml("-"));
                } else {
                    tr.appendChild(simpleRowHtml(formatRtt(row.rtt_down)));
                }
                if (row.rtt_up === null) {
                    tr.appendChild(simpleRowHtml("-"));
                } else {
                    tr.appendChild(simpleRowHtml(formatRtt(row.rtt_up)));
                }
                if (row.rxmit_down === null) {
                    tr.appendChild(simpleRowHtml("-"));
                } else {
                    tr.appendChild(simpleRowHtml(formatRetransmit(row.rxmit_down)));
                }
                if (row.rxmit_up === null) {
                    tr.appendChild(simpleRowHtml("-"));
                } else {
                    tr.appendChild(simpleRowHtml(formatRetransmit(row.rxmit_up)));
                }
                if (row.asn_name === null) {
                    row.asn_name = "-";
                } else {
                    tr.appendChild(simpleRow(row.asn_name));
                }
                tbody.appendChild(tr);
            })
            table.appendChild(tbody);
            clearDashDiv(this.id, target);
            target.appendChild(table);
        });
        wsClient.send({ LtsTopFlows: { seconds } });
    }
}
