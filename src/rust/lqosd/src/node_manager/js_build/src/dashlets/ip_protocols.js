import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

export class IpProtocols extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "IP Protocols";
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return "<h5>IP Protocols</h5><p>Bytes transferred over TCP/UDP/ICMP and port numbers, matched to common services when possible. This data is gathered from recently completed flows, and may be a little behind realtime.</p>";
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

            let t = document.createElement("table");
            t.classList.add("dash-table", "table-sm", "small");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading("Protocol"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");

            msg.data.forEach((r) => {
                let row = document.createElement("tr");
                row.classList.add("small");

                row.appendChild(simpleRow(r[0]));
                row.appendChild(simpleRow(scaleNumber(r[1].down)));
                row.appendChild(simpleRow(scaleNumber(r[1].up)));

                t.appendChild(row);
            });


            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}