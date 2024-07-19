import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRowHtml, theading} from "../helpers/builders";
import {formatRtt, formatPercent} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";

export class CircuitCapacityDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Circuits At Capacity";
    }

    tooltip() {
        return "<h5>Circuits at Capacity</h5><p>Customer circuits using close to their maximum capacities, and possibly in need of an upsell.</p>";
    }

    subscribeTo() {
        return [ "CircuitCapacity" ];
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
        if (msg.event === "CircuitCapacity") {
            let target = document.getElementById(this.id);

            // Sort msg.data by capacity[0]
            msg.data.sort((a, b) => {
                return b.capacity[0] - a.capacity[0];
            });

            let table = document.createElement("table");
            table.classList.add("dash-table", "table-sm", "small");
            let thead = document.createElement("thead");
            thead.classList.add("small");
            thead.appendChild(theading("Circuit"));
            thead.appendChild(theading("% Utilization (DL)"));
            thead.appendChild(theading("% Utilization (UL)"));
            thead.appendChild(theading("RTT"));
            table.appendChild(thead);
            let tbody = document.createElement("tbody");
            let count = 0;
            msg.data.forEach((c) => {
                if (c.capacity[0] < 0.9 && c.capacity[1] < 0.9) {
                    return;
                }
                if (count >= 7) {
                    return;
                }
                let row = document.createElement("tr");
                row.classList.add("small");

                let linkCol = document.createElement("td");
                let link = document.createElement("a");
                link.href = "circuit.html?id=" + encodeURI(c.circuit_id);
                link.innerText = c.circuit_name;
                redactCell(link);
                linkCol.appendChild(link);
                row.appendChild(linkCol);

                row.appendChild(simpleRowHtml(formatPercent(c.capacity[0]*100)));
                row.appendChild(simpleRowHtml(formatPercent(c.capacity[1]*100)));
                row.appendChild(simpleRowHtml(formatRtt(c.rtt)));
                tbody.appendChild(row);

                count++;
            })
            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}