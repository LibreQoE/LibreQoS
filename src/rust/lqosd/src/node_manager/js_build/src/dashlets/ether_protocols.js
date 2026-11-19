import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../lq_js_common/helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class EtherProtocols extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
    }

    title() {
        return t("dashlet.name.ether_protocols");
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.ether_protocols.tooltip")}</p>`;
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

            let table = document.createElement("table");
            table.classList.add("dash-table", "lqos-table", "lqos-table-compact", "small");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading(t("circuit.traffic.protocol")));
            th.appendChild(theading(t("common.table.dl_abbr")));
            th.appendChild(theading(t("common.table.ul_abbr")));
            th.appendChild(theading(t("common.table.packets_dl_abbr")));
            th.appendChild(theading(t("common.table.packets_ul_abbr")));
            th.appendChild(theading(t("common.table.rtt_dl_arrow")));
            th.appendChild(theading(t("common.table.rtt_ul_arrow")));
            table.appendChild(th);

            let tbody = document.createElement("tbody");
            let row = document.createElement("tr");
            row.classList.add("small");
            row.appendChild(simpleRow("IPv4"));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_bytes.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_bytes.up)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_packets.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v4_packets.up)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v4_rtt.down)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v4_rtt.up)));
            table.appendChild(row);

            row = document.createElement("tr");
            row.appendChild(simpleRow("IPv6"));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_bytes.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_bytes.up)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_packets.down)));
            row.appendChild(simpleRow(scaleNumber(msg.data.v6_packets.up)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v6_rtt.down)));
            row.appendChild(simpleRow(scaleNanos(msg.data.v6_rtt.up)));
            table.appendChild(row);

            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}
