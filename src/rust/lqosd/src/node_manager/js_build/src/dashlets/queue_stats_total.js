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