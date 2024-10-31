import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, simpleRow, theading} from "../helpers/builders";
import {scaleNumber, scaleNanos} from "../helpers/scaling";
import {QueueStatsTotalGraph} from "../graphs/queue_stats_total_graph";

export class QueueStatsTotalDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
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

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new QueueStatsTotalGraph(this.graphDivId())
    }

    onMessage(msg) {
        if (msg.event === "QueueStatsTotal") {
            this.graph.update(msg.marks, msg.drops);
        }
    }
}