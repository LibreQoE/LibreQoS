import {clearDashDiv, simpleRow, simpleRowHtml, theading} from "../helpers/builders";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";
import {formatRtt} from "../helpers/scaling";
import {TrimToFit} from "../lq_js_common/helpers/text_utils";
import {periodNameToSeconds} from "../helpers/time_periods";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {get_ws_client} from "../pubsub/ws";

const wsClient = get_ws_client();
const MAX_VISIBLE_ASN_CHARS = 12;
const MAX_VISIBLE_TOPFLOW_KEY_CHARS = 12;
const compactAsnLabel = (label) => {
    if (!label) return "";
    return label.length > MAX_VISIBLE_ASN_CHARS
        ? `${label.slice(0, MAX_VISIBLE_ASN_CHARS)}…`
        : label;
};
const compactTopflowKeyLabel = (label) => {
    if (!label) return "";
    return label.length > MAX_VISIBLE_TOPFLOW_KEY_CHARS
        ? `${label.slice(0, MAX_VISIBLE_TOPFLOW_KEY_CHARS)}…`
        : label;
};
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
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Top 10 Flows (by total bytes)";
    }

    tooltip() {
        return "<h5>Top 10 Flows (by total bytes)</h5><p>Top 10 flows by total bytes, including protocol, local and remote IP addresses, download and upload rates, total bytes, and remote ASN.</p>";
    }

    subscribeTo() {
        return [ "TopFlowsBytes" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.classList.add("dashbox-body-scroll", "dashbox-body-scroll-top10");
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
            t.classList.add("lqos-table", "lqos-table-compact", "small", "lqos-topflow-table", "lqos-topn-plain");

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
                    link.innerText = compactTopflowKeyLabel(TrimToFit(r.circuit_name, MAX_VISIBLE_TOPFLOW_KEY_CHARS + 1));
                    link.title = r.circuit_name || "";
                    link.classList.add("redactable", "lqos-table-cell-ellipsis");
                    circuit.appendChild(link);
                    row.appendChild(circuit);
                } else {
                    let localIp = document.createElement("td");
                    localIp.classList.add("lqos-topflow-key-cell");
                    const localIpText = document.createElement("span");
                    localIpText.innerText = compactTopflowKeyLabel(r.local_ip);
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
            const tableWrap = document.createElement("div");
            tableWrap.classList.add("lqos-table-wrap");
            tableWrap.appendChild(t);
            target.appendChild(tableWrap);
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
            table.classList.add("lqos-table", "lqos-table-compact", "small", "lqos-topn-plain");
            let thead = document.createElement("thead");
            thead.appendChild(theading("Circuit"));
            thead.appendChild(theading("Protocol"));
            thead.appendChild(theading("DL"));
            thead.appendChild(theading("UL"));
            thead.appendChild(theading("RTT DL"));
            thead.appendChild(theading("RTT UL"));
            thead.appendChild(theading("Rxmits DL"));
            thead.appendChild(theading("Rxmits UL"));
                const asnHeading = theading("ASN");
                asnHeading.classList.add("lqos-asn-cell");
                thead.appendChild(asnHeading);
            table.appendChild(thead);
            let tbody = document.createElement("tbody");

            data.forEach((row) => {
                let tr = document.createElement("tr");
                tr.classList.add("small");
                tr.appendChild(simpleRow(row.circuit_name || "", true));
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
                const asnCell = document.createElement("td");
                asnCell.classList.add("lqos-asn-cell");
                const asnText = document.createElement("span");
                const asnLabel = row.asn_name === null ? "-" : row.asn_name;
                asnText.classList.add("lqos-table-cell-ellipsis");
                if (asnLabel && asnLabel.length > 13) {
                    asnText.classList.add("tiny");
                }
                asnText.textContent = asnLabel || "";
                asnText.title = asnLabel || "";
                asnCell.appendChild(asnText);
                tr.appendChild(asnCell);
                tbody.appendChild(tr);
            })
            table.appendChild(tbody);
            clearDashDiv(this.id, target);
            const tableWrap = document.createElement("div");
            tableWrap.classList.add("lqos-table-wrap");
            tableWrap.appendChild(table);
            target.appendChild(tableWrap);
        });
        wsClient.send({ LtsTopFlows: { seconds } });
    }
}
