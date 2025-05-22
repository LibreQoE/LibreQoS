import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {clearDashDiv, simpleRowHtml, theading} from "../helpers/builders";
import {formatRtt, formatPercent} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";

export class CircuitCapacityDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
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

            // Update TimedCache with incoming data
            msg.data.forEach((c) => {
                this.timeCache.addOrUpdate(
                    c.circuit_id,
                    c,
                    Math.max(c.capacity[0], c.capacity[1])
                );
            });
            this.timeCache.tick();
            const cached = this.timeCache.get();

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
            for (let i = 0; i < cached.length; i++) {
                const c = cached[i];
                if (c.capacity[0] < 0.9 && c.capacity[1] < 0.9) {
                    continue;
                }
                if (count >= 7) {
                    break;
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
            }
            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(table);
        }
    }
}