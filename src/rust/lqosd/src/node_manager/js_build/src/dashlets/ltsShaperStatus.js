import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {clearDiv, simpleRow, simpleRowTrustedHtml, theading} from "../helpers/builders";
import {get_ws_client} from "../pubsub/ws";
import {t} from "../i18n/index.js";

// translatable: false — "Cadence" is the websocket event name the dashlet filters on, not a label

const wsClient = get_ws_client();
const listenOnce = (eventName, handler) => {
    const wrapped = (msg) => {
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class LtsShaperStatus extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return t("dashlet.name.insight_shaper_status");
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return `<h5>${t("dashlet.lts_shaper_status.heading")}</h5><p>${t("dashlet.lts_shaper_status.tooltip")}</p>`;
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
                listenOnce("LtsShaperStatus", (msg) => {
                    const data = msg && msg.data ? msg.data : [];
                    let target = document.getElementById(this.contentId);

                    let table = document.createElement("table");
                    table.classList.add("lqos-table", "lqos-table-compact", "small");
                    let thead = document.createElement("thead");
                    thead.appendChild(theading(""));
                    thead.appendChild(theading(t("common.label.shaper")));
                    thead.appendChild(theading(t("common.table.last_seen_seconds")));
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
                        tr.appendChild(simpleRowTrustedHtml(`<span style="color: ${color}">■</span>`));
                        tr.appendChild(simpleRow(row.name));
                        tr.appendChild(simpleRow(row.last_seen_seconds_ago + "s"));
                        tbody.appendChild(tr);
                    })
                    table.appendChild(tbody);
                    clearDiv(target);
                    const tableWrap = document.createElement("div");
                    tableWrap.classList.add("lqos-table-wrap");
                    tableWrap.appendChild(table);
                    target.appendChild(tableWrap);

                    //console.log(data);
                });
                wsClient.send({ LtsShaperStatus: {} });
            }
            this.count++;
            this.count %= 10;
        }
    }
}
