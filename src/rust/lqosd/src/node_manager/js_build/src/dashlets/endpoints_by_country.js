import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../helpers/scaling";

export class Top10EndpointsByCountry extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Endpoints by Country";
    }

    tooltip() {
        return "<h5>Endpoints by Country</h5><p>Top 10 endpoints by country/region, ordered by download speed. This data is gathered from recently completed flows, and may be a little behind realtime.</p>";
    }

    subscribeTo() {
        return [ "EndpointsByCountry" ];
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
        if (msg.event === "EndpointsByCountry") {
            let target = document.getElementById(this.id);

            let t = document.createElement("table");
            t.classList.add("table", "table-sm", "small");

            let th = document.createElement("thead");
            th.classList.add("small");
            th.appendChild(theading("Country"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("⬇ RTT"));
            th.appendChild(theading("️️⬆ RTT"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            let count = 0;
            msg.data.forEach((r) => {
                if (count >= 10) {
                    return;
                }
                let row = document.createElement("tr");
                row.classList.add("small");

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
                rttd.innerText = scaleNanos(r[2][0]);
                row.appendChild(rttd);

                let rttu = document.createElement("td");
                rttu.innerText = scaleNanos(r[2][1]);
                row.appendChild(rttu);

                t.appendChild(row);
                count++;
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}