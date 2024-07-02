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
            t.classList.add("table", "table-striped", "tiny");

            let th = document.createElement("thead");
            th.appendChild(theading("Country"));
            th.appendChild(theading("DL ⬇️"));
            th.appendChild(theading("UL ⬆️"));
            th.appendChild(theading("⬇ RTT"));
            th.appendChild(theading("️️⬆ RTT"));
            t.appendChild(th);

            let tbody = document.createElement("tbody");
            msg.data.forEach((r) => {
                let row = document.createElement("tr");

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
                rttd.innerText = scaleNanos(r[2].down);
                row.appendChild(rttd);

                let rttu = document.createElement("td");
                rttu.innerText = scaleNanos(r[2].up);
                row.appendChild(rttu);

                t.appendChild(row);
            });
            t.appendChild(tbody);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}