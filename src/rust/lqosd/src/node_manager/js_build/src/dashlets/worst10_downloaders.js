import {clearDashDiv, simpleRow, simpleRowHtml, theading, TopNTableFromMsgData} from "../helpers/builders";
import {TimedCache} from "../lq_js_common/helpers/timed_cache";
import {periodNameToSeconds} from "../helpers/time_periods";
import {formatRetransmit, formatRtt} from "../helpers/scaling";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {get_ws_client} from "../pubsub/ws";

const wsClient = get_ws_client();
const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class Worst10Downloaders extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.timeCache = new TimedCache(10);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Worst 10 Round-Trip Time";
    }

    tooltip() {
        return "<h5>Worst 10 Round-Trip Time</h5><p>Worst 10 Downloaders by round-trip time, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
    }

    subscribeTo() {
        return [ "WorstRTT" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    setup() {
        super.setup();
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "WorstRTT" && window.timePeriods.activePeriod === "Live") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                let key = r.circuit_id;
                this.timeCache.addOrUpdate(key, r);
            });
            this.timeCache.tick();

            let items = this.timeCache.get();
            let t = TopNTableFromMsgData(items);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }

    onTimeChange() {
        super.onTimeChange();
        let seconds = periodNameToSeconds(window.timePeriods.activePeriod);
        let spinnerDiv = document.createElement("div");
        spinnerDiv.innerHTML = "<i class='fas fa-spinner fa-spin'></i> Fetching Insight Data...";
        let target = document.getElementById(this.id);
        clearDashDiv(this.id, target);
        target.appendChild(spinnerDiv);
        listenOnceForSeconds("LtsWorst10Rtt", seconds, (msg) => {
            const data = msg && msg.data ? msg.data : [];
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
                    tr.appendChild(simpleRow(scaleNumber(toNumber(row.bytes_down, 0) * 1000000, 0)));
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
        wsClient.send({ LtsWorst10Rtt: { seconds } });
    }
}
