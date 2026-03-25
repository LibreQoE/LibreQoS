import {clearDashDiv, theading} from "../helpers/builders";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {TrimToFit} from "../lq_js_common/helpers/text_utils";
import {DashletBaseInsight} from "./insight_dashlet_base";

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

export class Top10FlowsRate extends DashletBaseInsight {
    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Top 10 Flows (by rate)";
    }

    tooltip() {
        return "<h5>Top 10 Flows (by rate)</h5><p>Top 10 flows by rate, including protocol, local and remote IP addresses, download and upload rates, total bytes, and remote ASN.</p>";
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
}
