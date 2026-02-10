import {clearDiv, simpleRowHtml, theading} from "../helpers/builders";
import {formatThroughput, formatRetransmit, formatCakeStat} from "../helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class TopTreeSummary extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Network Tree";
    }

    tooltip() {
        return "<h5>Network Tree</h5><p>Summary of the top-level network tree, including branch name, download and upload rates, TCP retransmits, Cake marks, and Cake drops.</p>";
    }

    subscribeTo() {
        return [ "TreeSummary" ];
    }

    buildContainer() {
        let base = super.buildContainer();

        // Match the height of adjacent graph dashlets (e.g. Top Level Sankey)
        // by keeping the dashlet chrome (title) outside the scroll area.
        const scroll = document.createElement("div");
        scroll.style.height = "250px";
        scroll.style.overflow = "auto";

        let t = document.createElement("table");
        t.id = this.id + "_table";
        t.classList.add("dash-table", "table-sm", "mytable", "small");

        let th = document.createElement("thead");
        th.classList.add('small');
        th.appendChild(theading("Branch"));
        th.appendChild(theading("DL ⬇️"));
        th.appendChild(theading("UL ⬆️"));
        th.appendChild(theading("Retr", 2, "<h5>TCP Retransmits</h5><p>Number of TCP retransmits in the last second.</p>", "tts_retransmits"));
        th.appendChild(theading("Marks", 2, "<h5>Cake Marks</h5><p>Number of times the Cake traffic manager has applied ECN marks to avoid congestion.</p>", "tts_marks"));
        th.appendChild(theading("Drops", 2, "<h5>Cake Drops</h5><p>Number of times the Cake traffic manager has dropped packets to avoid congestion.</p>", "tts_drops"));
        t.appendChild(th);

        scroll.appendChild(t);
        base.appendChild(scroll);

        return base;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "TreeSummary") {
            let target = document.getElementById(this.id + "_table");

            clearDiv(target, 1);

            msg.data.forEach((r) => {
                let row = document.createElement("tr");
                row.classList.add("small");

                let nameCol = document.createElement("td");
                let link = document.createElement("a");
                link.href = "/tree.html?id=" + r[0];
                link.innerText = r[1].name;
                link.classList.add("redactable");
                nameCol.appendChild(link);

                row.appendChild(nameCol);
                const tpDown = toNumber(r[1].current_throughput[0], 0) * 8;
                const tpUp = toNumber(r[1].current_throughput[1], 0) * 8;
                row.appendChild(simpleRowHtml(formatThroughput(tpDown, r[1].max_throughput[0])));
                row.appendChild(simpleRowHtml(formatThroughput(tpUp, r[1].max_throughput[1])));

                const tcpPacketsDown = toNumber(r[1].current_tcp_packets[0], 0);
                const tcpPacketsUp = toNumber(r[1].current_tcp_packets[1], 0);
                const retransmitsDown = toNumber(r[1].current_retransmits[0], 0);
                const retransmitsUp = toNumber(r[1].current_retransmits[1], 0);

                if (tcpPacketsDown > 0) {
                    row.appendChild(simpleRowHtml(formatRetransmit(retransmitsDown / tcpPacketsDown)))
                } else {
                    row.appendChild(simpleRowHtml(""));
                }
                if (tcpPacketsUp > 0) {
                    row.appendChild(simpleRowHtml(formatRetransmit(retransmitsUp / tcpPacketsUp)))
                } else {
                    row.appendChild(simpleRowHtml(""));
                }
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_marks[0])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_marks[1])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_drops[0])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_drops[1])))
                target.appendChild(row);
            });
        }
    }
}
