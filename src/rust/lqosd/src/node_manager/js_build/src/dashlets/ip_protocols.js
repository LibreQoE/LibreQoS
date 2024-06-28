import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../helpers/scaling";

export class IpProtocols extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "IP Protocols";
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
            t.classList.add("table", "table-striped", "tiny");

            let th = document.createElement("thead");
            th.appendChild(theading("Protocol"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");

            msg.data.forEach((r) => {
                let row = document.createElement("tr");

                row.appendChild(simpleRow(r[0]));
                row.appendChild(simpleRow(scaleNumber(r[1][0])));
                row.appendChild(simpleRow(scaleNumber(r[1][1])));

                t.appendChild(row);
            });


            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}