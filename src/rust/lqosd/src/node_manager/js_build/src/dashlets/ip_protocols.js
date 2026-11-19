import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class IpProtocols extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
    }

    title() {
        return t("dashlet.name.ip_protocols");
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.ip_protocols.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "IpProtocols" ];
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
        if (msg.event === "IpProtocols") {
            let target = document.getElementById(this.id);

            let table = document.createElement("table");
            table.classList.add("dash-table", "lqos-table", "lqos-table-compact", "small");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading(t("circuit.traffic.protocol")));
            th.appendChild(theading(t("common.table.dl_abbr")));
            th.appendChild(theading(t("common.table.ul_abbr")));
            table.appendChild(th);

            let tbody = document.createElement("tbody");

            msg.data.forEach((r) => {
                let row = document.createElement("tr");
                row.classList.add("small");

                row.appendChild(simpleRow(r[0]));
                row.appendChild(simpleRow(scaleNumber(r[1].down)));
                row.appendChild(simpleRow(scaleNumber(r[1].up)));

                table.appendChild(row);
            });


            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}
