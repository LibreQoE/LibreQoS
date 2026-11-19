import {clearDiv, simpleRowTrustedHtml, theading} from "../helpers/builders";
import {formatThroughput, formatRetransmit, formatCakeStat} from "../helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {effectiveMax} from "../helpers/network_rates.mjs";
import {t} from "../i18n/index.js";

// translatable: false — "_table" is the DOM id suffix this dashlet appends

function retransmitPacketsForNode(node, direction) {
    return toNumber(
        node.current_tcp_retransmit_packets?.[direction] ?? node.current_tcp_packets?.[direction],
        0,
    );
}

export class TopTreeSummary extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return t("dashlet.name.network_tree_summary");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.tree_summary.tooltip")}</p>`;
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

        let table = document.createElement("table");
        table.id = this.id + "_table";
        table.classList.add("dash-table", "lqos-table", "lqos-table-compact", "mytable", "small");

        let th = document.createElement("thead");
        th.classList.add('small');
        th.appendChild(theading(t("topology_manager.details.branch")));
        th.appendChild(theading(t("common.table.dl_abbr")));
        th.appendChild(theading(t("common.table.ul_abbr")));
        th.appendChild(theading(t("tree.table.retr"), 2, `<h5>${t("dashlet.tree_summary.retr_heading")}</h5><p>${t("dashlet.tree_summary.retr_help")}</p>`, "tts_retransmits"));
        th.appendChild(theading(t("tree.table.marks"), 2, `<h5>${t("dashlet.tree_summary.marks_heading")}</h5><p>${t("dashlet.tree_summary.marks_help")}</p>`, "tts_marks"));
        th.appendChild(theading(t("tree.table.drops"), 2, `<h5>${t("dashlet.tree_summary.drops_heading")}</h5><p>${t("dashlet.tree_summary.drops_help")}</p>`, "tts_drops"));
        table.appendChild(th);

        scroll.appendChild(table);
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
                link.href = "/tree.html?parent=" + r[0];
                link.innerText = r[1].name;
                link.classList.add("redactable");
                nameCol.appendChild(link);

                row.appendChild(nameCol);
                const tpDown = toNumber(r[1].current_throughput[0], 0) * 8;
                const tpUp = toNumber(r[1].current_throughput[1], 0) * 8;
                const maxThroughput = effectiveMax(r[1]);
                row.appendChild(simpleRowTrustedHtml(formatThroughput(tpDown, maxThroughput[0])));
                row.appendChild(simpleRowTrustedHtml(formatThroughput(tpUp, maxThroughput[1])));

                const tcpPacketsDown = retransmitPacketsForNode(r[1], 0);
                const tcpPacketsUp = retransmitPacketsForNode(r[1], 1);
                const retransmitsDown = toNumber(r[1].current_retransmits[0], 0);
                const retransmitsUp = toNumber(r[1].current_retransmits[1], 0);

                if (tcpPacketsDown > 0) {
                    row.appendChild(simpleRowTrustedHtml(formatRetransmit(retransmitsDown / tcpPacketsDown)))
                } else {
                    row.appendChild(simpleRowTrustedHtml(""));
                }
                if (tcpPacketsUp > 0) {
                    row.appendChild(simpleRowTrustedHtml(formatRetransmit(retransmitsUp / tcpPacketsUp)))
                } else {
                    row.appendChild(simpleRowTrustedHtml(""));
                }
                row.appendChild(simpleRowTrustedHtml(formatCakeStat(r[1].current_marks[0])))
                row.appendChild(simpleRowTrustedHtml(formatCakeStat(r[1].current_marks[1])))
                row.appendChild(simpleRowTrustedHtml(formatCakeStat(r[1].current_drops[0])))
                row.appendChild(simpleRowTrustedHtml(formatCakeStat(r[1].current_drops[1])))
                target.appendChild(row);
            });
        }
    }
}
