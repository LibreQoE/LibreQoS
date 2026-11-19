import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {clearDashDiv, simpleRowTrustedHtml, theading} from "../helpers/builders";
import {formatRtt, formatPercent} from "../helpers/scaling";
import {redactCell} from "../helpers/redact";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {t} from "../i18n/index.js";

export class CircuitCapacityDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return t("dashlet.capacity.circuits.title");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.capacity.circuits.tooltip")}</p>`;
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
                if (c.capacity[0] < 0.9 && c.capacity[1] < 0.9) {
                    return;
                }
                this.timeCache.addOrUpdate(
                    c.circuit_id,
                    c,
                    Math.max(c.capacity[0], c.capacity[1])
                );
            });
            this.timeCache.tick();
            const cached = this.timeCache.get();

            let table = document.createElement("table");
            table.classList.add("dash-table", "lqos-table", "lqos-table-compact", "small");
            let thead = document.createElement("thead");
            thead.classList.add("small");
            thead.appendChild(theading(t("common.table.circuit")));
            thead.appendChild(theading(t("common.table.utilization_dl")));
            thead.appendChild(theading(t("common.table.utilization_ul")));
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
                let opacity = (10.0 - c.lastSeen) / 10.0;
                let row = document.createElement("tr");
                row.style.opacity = opacity.toFixed(1);
                row.classList.add("small");

                let linkCol = document.createElement("td");
                let link = document.createElement("a");
                link.href = "circuit.html?id=" + encodeURI(c.circuit_id);
                link.innerText = c.circuit_name;
                redactCell(link);
                linkCol.appendChild(link);
                row.appendChild(linkCol);

                row.appendChild(simpleRowTrustedHtml(formatPercent(c.capacity[0]*100)));
                row.appendChild(simpleRowTrustedHtml(formatPercent(c.capacity[1]*100)));
                row.appendChild(simpleRowTrustedHtml(formatRtt(c.rtt)));
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
