import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../helpers/scaling";

export class TopTreeSummary extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Network Tree";
    }

    subscribeTo() {
        return [ "TreeSummary" ];
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
        if (msg.event === "TreeSummary") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("table", "table-striped", "tiny");

            let th = document.createElement("thead");
            th.appendChild(theading("Branch"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("TCP Re-xmit"));
            th.appendChild(theading("Cake Marks"));
            th.appendChild(theading("Cake Drops"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                let row = document.createElement("tr");
                row.appendChild(simpleRow(r[1].name));
                row.appendChild(simpleRow(scaleNumber(r[1].current_throughput[0] * 8)));
                row.appendChild(simpleRow(scaleNumber(r[1].current_throughput[1] * 8)));
                row.appendChild(simpleRow(r[1].current_retransmits[0] + " / " + r[1].current_retransmits[1]))
                row.appendChild(simpleRow(r[1].current_marks[0] + " / " + r[1].current_marks[1]));
                row.appendChild(simpleRow(r[1].current_drops[0] + " / " + r[1].current_drops[1]));
                t.appendChild(row);
            });

            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}