import {BaseDashlet} from "./base_dashlet";
import {ThroughputRingBufferGraph} from "../graphs/throughput_ring_graph";
import {LtsThroughputPeriodGraph} from "../graphs/lts_throughput_period_graph";
import {periodNameToSeconds} from "../helpers/time_periods";

export class ThroughputRingDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
        this.counter = 0;
    }

    title() {
        return "Last 5 Minutes Throughput";
    }

    tooltip() {
        return "<h5>Last 5 Minutes Throughput</h5><p>Shaped (AQM controlled and limited) and Unshaped (not found in your Shaped Devices file) traffic over the last five minutes.</p>"
    }

    subscribeTo() {
        return [ "Throughput" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        let graphs = this.graphDiv();

        // Add some time controls
        base.classList.add("dashlet-with-controls");
        let controls = document.createElement("div");
        controls.classList.add("dashgraph-controls", "small");

        let btnLive = this.makePeriodBtn("Live");
        btnLive.classList.add("active");
        controls.appendChild(btnLive);

        let targets = ["1h", "6h", "12h", "24h", "7d"];
        targets.forEach((t) => {
            let graph = document.createElement("div");
            graph.id = this.graphDivId() + "_" + t;
            graph.classList.add("dashgraph");
            graph.style.display = "none";
            graph.innerHTML = window.hasLts ? "Loading..." : "<p class='text-secondary small'>You need an active Long-Term Stats account to view this data.</p>";
            this.graphDivs.push(graph);
            controls.appendChild(this.makePeriodBtn(t));
        });

        base.appendChild(controls);
        base.appendChild(graphs);
        this.graphDivs.forEach((g) => {
            base.appendChild(g);
        });
        return base;
    }

    setup() {
        super.setup();
        this.graph = new ThroughputRingBufferGraph(this.graphDivId());
        this.ltsLoaded = false;
        if (window.hasLts) {
            this.graphDivs.forEach((g) => {
                let period = periodNameToSeconds(g.id.replace(this.graphDivId() + "_", ""));
                let graph = new LtsThroughputPeriodGraph(g.id, period);
                this.graphs.push(graph);
            });
        }
    }

    onMessage(msg) {
        if (msg.event === "Throughput") {
            this.graph.update(msg.data.shaped_bps, msg.data.bps);
            if (!this.ltsLoaded && window.hasLts) {
                //console.log("Loading LTS data");
                this.graphs.forEach((g) => {
                    //console.log("Loading " + g.period);
                    let url = "/local-api/ltsThroughput/" + g.period;
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

    supportsZoom() {
        return true;
    }
}