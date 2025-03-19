import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {QueueStatsTotalGraph} from "../graphs/queue_stats_total_graph";
import {periodNameToSeconds} from "../helpers/time_periods";
import {LtsCakeGraph} from "../graphs/lts_cake_stats_graph";

export class QueueStatsTotalDash extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.counter = 0;
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
        let graphs = this.graphDiv();

        // Add some time controls
        base.classList.add("dashlet-with-controls");
        let controls = document.createElement("div");
        controls.classList.add("dashgraph-controls", "small");

        if (window.hasInsight) {
            let btnLive = this.makePeriodBtn("Live");
            btnLive.classList.add("active");
            controls.appendChild(btnLive);

            let targets = ["1h", "6h", "12h", "24h", "7d"];
            targets.forEach((t) => {
                let graph = document.createElement("div");
                graph.id = this.graphDivId() + "_" + t;
                graph.classList.add("dashgraph");
                graph.style.display = "none";
                graph.innerHTML = window.hasLts ? "Loading..." : "<p class='text-secondary small'>You need an active LibreQoS Insight account to view this data.</p>";
                this.graphDivs.push(graph);
                controls.appendChild(this.makePeriodBtn(t));
            });
        }

        base.appendChild(controls);
        base.appendChild(graphs);
        this.graphDivs.forEach((g) => {
            base.appendChild(g);
        });
        return base;
    }

    setup() {
        super.setup();
        this.graph = new QueueStatsTotalGraph(this.graphDivId());
        this.ltsLoaded = false;
        if (window.hasLts) {
            this.graphDivs.forEach((g) => {
                let period = periodNameToSeconds(g.id.replace(this.graphDivId() + "_", ""));
                let graph = new LtsCakeGraph(g.id, period);
                this.graphs.push(graph);
            });
        }
    }

    onMessage(msg) {
        if (msg.event === "QueueStatsTotal") {
            this.graph.update(msg.marks, msg.drops);

            if (!this.ltsLoaded && window.hasLts) {
                //console.log("Loading LTS data");
                this.graphs.forEach((g) => {
                    //console.log("Loading " + g.period);
                    let url = "/local-api/ltsCake/" + g.period;
                    //console.log(url);
                    $.get(url, (data) => {
                        //console.log(data);
                        g.update(data);
                    });
                });
                this.ltsLoaded = true;
            }
            this.counter++;
            if (this.counter > 120) {
                // Reload the LTS graphs every 2 minutes
                this.counter = 0;
                this.ltsLoaded = false;
            }
        }
    }
}