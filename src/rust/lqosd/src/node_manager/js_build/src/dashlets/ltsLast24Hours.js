import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {LtsLast24Hours_graph} from "../graphs/ltsLast24Hours_graph";
import {get_ws_client} from "../pubsub/ws";
import {t} from "../i18n/index.js";

// translatable: false — "Cadence" is the websocket event name the dashlet filters on, not a label

const wsClient = get_ws_client();
const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class LtsLast24Hours extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return t("dashlet.name.insight_last_24_hours");
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return `<h5>${t("dashlet.lts_24h.heading")}</h5><p>${t("dashlet.lts_24h.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "Cadence" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.count = 0;
        this.graph = new LtsLast24Hours_graph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Cadence") {
            if (this.count === 0) {
                const seconds = 24 * 60 * 60;
                listenOnceForSeconds("LtsThroughput", seconds, (msg) => {
                    const data = msg && msg.data ? msg.data : [];
                    this.graph.update(data);
                });
                wsClient.send({ LtsThroughput: { seconds } });
            }
            this.count++;
            this.count %= 120;
        }
    }
}
