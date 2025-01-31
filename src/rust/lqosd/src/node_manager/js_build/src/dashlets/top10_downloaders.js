import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, clearDiv, simpleRow, simpleRowHtml, theading, TopNTableFromMsgData} from "../helpers/builders";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {periodNameToSeconds} from "../helpers/time_periods";
import {formatRetransmit, formatRtt} from "../helpers/scaling";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

export class Top10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    title() {
        return "Top 10 Downloaders";
    }

    tooltip() {
        return "<h5>Top 10 Downloaders</h5><p>Top 10 Downloaders by bits per second, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
    }

    subscribeTo() {
        return [ "TopDownloads" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    canBeSlowedDown() {
        return true;
    }

    setup() {
        super.setup();
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads" && window.timePeriods.activePeriod === "Live") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r);
            });
            this.timeCache.tick();

            let items = this.timeCache.get();
            items.sort((a, b) => {
                return b.bits_per_second.down - a.bits_per_second.down;
            });
            // Limit to 10 entries
            items = items.slice(0, 10);
            let t = TopNTableFromMsgData(items);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }

    onTimeChange() {
        super.onTimeChange();
        let seconds = periodNameToSeconds(window.timePeriods.activePeriod);
        document.getElementById(this.id).innerHTML = "<i class='fas fa-spinner fa-spin'></i> Fetching Insight Data...";
        $.get("/local-api/ltsTop10Downloaders/" + seconds, (data) => {
            let target = document.getElementById(this.id);

            let table = document.createElement("table");
            table.classList.add("table", "table-sm", "small");
            let thead = document.createElement("thead");
            thead.appendChild(theading("Circuit"));
            thead.appendChild(theading("Bytes Downloaded"));
            thead.appendChild(theading("RTT"));
            thead.appendChild(theading("Rxmits"));
            table.appendChild(thead);
            let tbody = document.createElement("tbody");

            data.forEach((row) => {
                //console.log(row);
                let tr = document.createElement("tr");
                tr.classList.add("small");
                tr.appendChild(simpleRowHtml("<a href='circuit.html?circuit=" + row.circuit_hash + "' class='redactable'>" + row.circuit_name + "</a>"));
                tr.appendChild(simpleRow(scaleNumber(row.bytes_down * 1000000, 0)));
                if (row.rtt !== null) {
                    tr.appendChild(simpleRowHtml(formatRtt(row.rtt)));
                } else {
                    tr.appendChild(simpleRow("-"));
                }
                if (row.rxmit !== null) {
                    tr.appendChild(simpleRowHtml(formatRetransmit(row.rxmit)));
                } else {
                    tr.appendChild(simpleRow("-"));
                }
                tbody.appendChild(tr);
            })
            table.appendChild(tbody);
            clearDashDiv(this.id, target);
            target.appendChild(table);
        });
    }
}
