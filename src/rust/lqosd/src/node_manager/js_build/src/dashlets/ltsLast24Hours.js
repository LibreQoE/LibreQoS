import {BaseDashlet} from "./base_dashlet";
import {LtsLast24Hours_graph} from "../graphs/ltsLast24Hours_graph";

export class LtsLast24Hours extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Last 24 Hours Throughput (Insight)";
    }

    canBeSlowedDown() {
        return true;
    }

    tooltip() {
        return "<h5>24 Hour Throughput</h5><p>Throughput for the last 24 hours. The error bars indicate minimum and maximum within a time range, while the points indicate median. Many other systems will inadvertently give incorrect data by averaging across large periods of time, and not sampling frequently enough (1 second for Mb/s).</p>";
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
                // Test
                $.get("/local-api/lts24", (data) => {
                    this.graph.update(data);
                });
            }
            this.count++;
            this.count %= 120;
        }
    }
}