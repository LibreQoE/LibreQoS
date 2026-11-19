import {clearDashDiv, theading} from "../helpers/builders";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {rttNanosAsSpan} from "../helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t, tAttr} from "../i18n/index.js";

export class Top10EndpointsByCountry extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return t("dashlet.name.top_10_endpoints_by_country");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.country.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "EndpointsByCountry" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.classList.add("dashbox-body-scroll", "dashbox-body-scroll-top10");
        return base;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "EndpointsByCountry") {
            let target = document.getElementById(this.id);

            let table = document.createElement("table");
            table.classList.add("lqos-table", "lqos-table-compact", "small", "lqos-topn-plain");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading(""));
            th.appendChild(theading(t("circuit.traffic.country")));
            th.appendChild(theading(t("common.table.dl_abbr")));
            th.appendChild(theading(t("common.table.ul_abbr")));
            th.appendChild(theading(t("common.table.rtt_dl_arrow")));
            th.appendChild(theading(t("common.table.rtt_ul_arrow")));
            table.appendChild(th);

            let tbody = document.createElement("tbody");
            let count = 0;
            msg.data.forEach((r) => {
                if (count >= 10) {
                    return;
                }
                let row = document.createElement("tr");
                row.classList.add("small");

                let flag = document.createElement("td");
                if (r[3] !== null && r[3] !== "") {
                    let flagName = r[3].toLowerCase();
                    flag.innerHTML = `<img alt="${tAttr("common.flag_alt", {country: flagName})}" src="flags/${flagName}.svg" style="width: 20px; height: 20px;">`;
                } else {
                    flag.innerText = "️";
                }
                row.appendChild(flag);

                let country = document.createElement("td");
                country.innerText = r[0];
                row.appendChild(country);

                let dld = document.createElement("td");
                dld.innerText = scaleNumber(r[1].down);
                row.appendChild(dld);

                let dlu = document.createElement("td");
                dlu.innerText = scaleNumber(r[1].up);
                row.appendChild(dlu);

                let rttd = document.createElement("td");
                rttd.innerHTML = rttNanosAsSpan(r[2][0]);
                row.appendChild(rttd);

                let rttu = document.createElement("td");
                rttu.innerHTML = rttNanosAsSpan(r[2][1]);
                row.appendChild(rttu);

                tbody.appendChild(row);
                count++;
            });
            table.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            const tableWrap = document.createElement("div");
            tableWrap.classList.add("lqos-table-wrap");
            tableWrap.appendChild(table);
            target.appendChild(tableWrap);
        }
    }
}
