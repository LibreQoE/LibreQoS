import {QueueStatsTotalGraph} from "../graphs/queue_stats_total_graph";
import {LtsCakeGraph} from "../graphs/lts_cake_stats_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {periodNameToSeconds} from "../helpers/time_periods";
import {get_ws_client} from "../pubsub/ws";
import {t} from "../i18n/index.js";

// translatable: false — "Error" is the websocket error event this helper unsubscribes from, not a label
// translatable: false — "Live" is the time-period token the page compares against, not a label
// translatable: false — "_history_graph" is the DOM id suffix this dashlet appends

const wsClient = get_ws_client();

const listenOnceForSeconds = (eventName, seconds, onSuccess, onError) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        wsClient.off("Error", errorHandler);
        onSuccess(msg);
    };
    const errorHandler = (msg) => {
        wsClient.off(eventName, wrapped);
        wsClient.off("Error", errorHandler);
        if (onError) {
            onError(msg);
        }
    };
    wsClient.on(eventName, wrapped);
    wsClient.on("Error", errorHandler);
};

export class QueueStatsTotalDash extends DashletBaseInsight {
    constructor(slot) {
        super(slot);
        this.historyGraph = null;
        this.historyPeriod = null;
    }

    title() {
        return t("dashlet.name.total_cake_stats");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.cake_stats.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "QueueStatsTotal" ];
    }

    historyGraphDivId() {
        return this.id + "_history_graph";
    }

    buildContainer() {
        let base = super.buildContainer();
        let liveGraph = this.graphDiv();
        liveGraph.id = this.graphDivId();
        liveGraph.classList.add("queue-live-graph");

        let historyGraph = document.createElement("div");
        historyGraph.id = this.historyGraphDivId();
        historyGraph.classList.add("dashgraph");
        historyGraph.style.display = "none";

        base.appendChild(liveGraph);
        base.appendChild(historyGraph);
        return base;
    }

    setup() {
        super.setup();
        this.graph = new QueueStatsTotalGraph(this.graphDivId());
        window.timeGraphs.push(this);
        this.onTimeChange();
    }

    onMessage(msg) {
        if (msg.event === "QueueStatsTotal" && window.timePeriods.activePeriod === "Live") {
            this.#applyLiveSample(msg.marks, msg.drops);
        }
    }

    onTimeChange() {
        super.onTimeChange();

        const liveGraph = document.getElementById(this.graphDivId());
        const historyGraph = document.getElementById(this.historyGraphDivId());
        if (!liveGraph || !historyGraph) {
            return;
        }

        const periodName = window.timePeriods.activePeriod;
        const isLive = periodName === "Live";

        liveGraph.style.display = isLive ? "" : "none";
        historyGraph.style.display = isLive ? "none" : "";

        if (isLive) {
            return;
        }

        if (!window.hasLts) {
            historyGraph.innerHTML = `<p class="text-secondary small">${t("common.insight.entitled_history")}</p>`;
            return;
        }

        const seconds = periodNameToSeconds(periodName);
        this.historyPeriod = periodName;

        if (this.historyGraph === null) {
            historyGraph.innerHTML = "";
            this.historyGraph = new LtsCakeGraph(this.historyGraphDivId(), seconds);
        } else {
            this.historyGraph.period = seconds;
            this.historyGraph.chart.showLoading();
        }

        listenOnceForSeconds(
            "LtsCake",
            seconds,
            (msg) => {
                if (this.historyPeriod !== periodName) {
                    return;
                }
                const data = msg && msg.data ? msg.data : [];
                if (this.historyGraph === null) {
                    this.historyGraph = new LtsCakeGraph(this.historyGraphDivId(), seconds);
                }
                this.historyGraph.update(data);
                if (data.length === 0) {
                    this.historyGraph.chart.showLoading(t("common.state.no_history"));
                } else {
                    this.historyGraph.chart.hideLoading();
                }
            },
            () => {
                if (this.historyPeriod === periodName) {
                    if (this.historyGraph !== null && this.historyGraph.chart) {
                        this.historyGraph.chart.showLoading(t("dashlet.cake_stats.load_failed"));
                    } else {
                        historyGraph.innerHTML = `<p class="text-danger small">${t("dashlet.cake_stats.load_failed")}</p>`;
                    }
                }
            },
        );
        wsClient.send({ LtsCake: { seconds } });
    }

    #applyLiveSample(marks, drops) {
        this.graph.update(marks, drops);
    }
}
