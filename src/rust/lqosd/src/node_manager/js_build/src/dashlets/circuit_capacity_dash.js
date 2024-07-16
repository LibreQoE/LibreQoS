import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRow, simpleRowHtml, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos, formatRtt} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";

export class CircuitCapacityDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
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

            let table = document.createElement("table");
            table.classList.add("table", "table-striped", "small");
            let thead = document.createElement("thead");
            thead.classList.add("small");
            thead.appendChild(theading("Circuit"));
            thead.appendChild(theading("% Utilization (DL)"));
            thead.appendChild(theading("% Utilization (UL)"));
            thead.appendChild(theading("RTT"));
            table.appendChild(thead);
            let tbody = document.createElement("tbody");
            msg.data.forEach((c) => {
                if (c.capacity[0] < 0.9 && c.capacity[1] < 0.9) {
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

                row.appendChild(simpleRow((c.capacity[0]*100).toFixed(0)));
                row.appendChild(simpleRow((c.capacity[1]*100).toFixed(0)));
                row.appendChild(simpleRowHtml(formatRtt(c.rtt)));
                tbody.appendChild(row);
            })
            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}