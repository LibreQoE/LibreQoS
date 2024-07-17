import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRow, simpleRowHtml, theading} from "../helpers/builders";
import {formatRtt, formatPercent} from "../helpers/scaling";

export class TreeCapacityDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Tree Nodes At Capacity";
    }

    tooltip() {
        return "<h5>Tree Nodes at Capacity</h5><p>Distribution Nodes approaching their maximum capacity, possibly in need of an upgrade or a better shaping policy.</p>";
    }

    subscribeTo() {
        return [ "TreeCapacity" ];
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
        if (msg.event === "TreeCapacity") {
            //console.log(msg.data);
            let target = document.getElementById(this.id);

            let table = document.createElement("table");
            table.classList.add("table", "table-striped", "small");
            let thead = document.createElement("thead");
            thead.classList.add("small");
            thead.appendChild(theading("Node"));
            thead.appendChild(theading("% Utilization (DL)"));
            thead.appendChild(theading("% Utilization (UL)"));
            thead.appendChild(theading("RTT"));
            table.appendChild(thead);
            let tbody = document.createElement("tbody");

            msg.data.forEach((node) => {
                if (node.max_down === 0 || node.max_up === 0) {
                    // No divisions by zero
                    return;
                }
                let down = node.down / node.max_down;
                let up = node.up / node.max_up;

                if (down < 0.75 && up < 0.75) {
                    // Not at capacity
                    return;
                }

                let row = document.createElement("tr");
                row.classList.add("small");

                let linkCol = document.createElement("td");
                let link = document.createElement("a");
                link.href = "/node.html?id=" + node.id;
                link.innerText = node.name;
                link.classList.add("tiny", "redactable");
                linkCol.appendChild(link);

                row.appendChild(linkCol());
                row.appendChild(simpleRowHtml(formatPercent(down*100)));
                row.appendChild(simpleRowHtml(formatPercent(up*100)));
                row.appendChild(simpleRowHtml(formatRtt(node.rtt)));

                tbody.appendChild(row);
            });
            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}