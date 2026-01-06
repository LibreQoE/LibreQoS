import {QueueStatsTotalGraph} from "../graphs/queue_stats_total_graph";
import {LtsCakeGraph} from "../graphs/lts_cake_stats_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {periodNameToSeconds} from "../helpers/time_periods";
import {get_ws_client} from "../pubsub/ws";

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
        return "Cake Stats (Total)";
    }

    tooltip() {
        return "<h5>Cake Stats (Total)</h5><p>Total number of fair queueing interventions per second. ECN Marks label packets as having possible congestion, signalling the flow to slow down. Drops discard selected packets to \"pace\" the queue for efficient, fair, low-latency throughput.</p>";
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
            this.graph.update(msg.marks, msg.drops);
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
            historyGraph.innerHTML = "<p class='text-secondary small'>You need an active LibreQoS Insight account to view this data.</p>";
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
                    this.historyGraph.chart.showLoading("No historical data available.");
                } else {
                    this.historyGraph.chart.hideLoading();
                }
            },
            () => {
                if (this.historyPeriod === periodName) {
                    if (this.historyGraph !== null && this.historyGraph.chart) {
                        this.historyGraph.chart.showLoading("Failed to load cake stats.");
                    } else {
                        historyGraph.innerHTML = "<p class='text-danger small'>Failed to load cake stats.</p>";
                    }
                }
            },
        );
        wsClient.send({ LtsCake: { seconds } });
    }
}
