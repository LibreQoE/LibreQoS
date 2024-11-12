import {BaseDashlet} from "./base_dashlet";
import {clearDiv, simpleRowHtml, theading} from "../helpers/builders";
import {formatThroughput, formatRetransmit, formatCakeStat} from "../helpers/scaling";

export class TopTreeSummary extends BaseDashlet {
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
        base.style.height = "250px";
        base.style.overflow = "auto";

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

        base.appendChild(t);

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
                row.appendChild(simpleRowHtml(formatThroughput(r[1].current_throughput[0] * 8, r[1].max_throughput[0])));
                row.appendChild(simpleRowHtml(formatThroughput(r[1].current_throughput[1] * 8, r[1].max_throughput[1])));
                row.appendChild(simpleRowHtml(formatRetransmit(r[1].current_retransmits[0] )))
                row.appendChild(simpleRowHtml(formatRetransmit(r[1].current_retransmits[1])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_marks[0])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_marks[1])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_drops[0])))
                row.appendChild(simpleRowHtml(formatCakeStat(r[1].current_drops[1])))
                target.appendChild(row);
            });
        }
    }
}