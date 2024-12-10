import {BaseDashlet} from "./base_dashlet";
import {clearDiv, simpleRow, simpleRowHtml, theading} from "../helpers/builders";

export class LtsShaperStatus extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Shaper Status (Insight)";
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return "<h5>Shaper Status</h5><p>Status from each of the LibreQoS shapers you are running.</p>";
    }

    subscribeTo() {
        return [ "Cadence" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "250px";
        base.style.overflow = "auto";
        let content = document.createElement("div");
        content.style.width = "100%";
        content.id = "ltsShaperStatus_" + this.slot;
        this.contentId = content.id;
        base.appendChild(content);
        return base;
    }

    setup() {
        super.setup();
        this.count = 0;
    }

    onMessage(msg) {
        if (msg.event === "Cadence") {
            if (this.count === 0) {
                $.get("/local-api/ltsShaperStatus", (data) => {
                    let target = document.getElementById(this.contentId);

                    let table = document.createElement("table");
                    table.classList.add("table", "table-sm", "small");
                    let thead = document.createElement("thead");
                    thead.appendChild(theading(""));
                    thead.appendChild(theading("Shaper"));
                    thead.appendChild(theading("Last Seen (seconds)"));
                    table.appendChild(thead);
                    let tbody = document.createElement("tbody");

                    data.forEach((row) => {
                        let tr = document.createElement("tr");
                        tr.classList.add("small");
                        let color = "green";
                        if (row.last_seen_seconds_ago > 300) {
                            color = "red";
                        } else if (row.last_seen_seconds_ago > 120) {
                            color = "orange";
                        }
                        tr.appendChild(simpleRowHtml(`<span style="color: ${color}">■</span>`));
                        tr.appendChild(simpleRow(row.name));
                        tr.appendChild(simpleRow(row.last_seen_seconds_ago + "s"));
                        tbody.appendChild(tr);
                    })
                    table.appendChild(tbody);
                    clearDiv(target);
                    target.appendChild(table);

                    //console.log(data);
                });
            }
            this.count++;
            this.count %= 10;
        }
    }
}