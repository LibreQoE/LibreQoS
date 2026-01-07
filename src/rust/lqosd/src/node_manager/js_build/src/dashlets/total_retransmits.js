import {periodNameToSeconds} from "../helpers/time_periods";
import {RetransmitsGraph} from "../graphs/retransmits_graph";
import {LtsRetransmitsGraph} from "../graphs/lts_retransmits";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class TcpRetransmitsDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
        this.counter = 0;
    }

    title() {
        return "Last 5 Minutes TCP Retransmits";
    }

    tooltip() {
        return "<h5>Last 5 Minutes TCP Retransmits</h5><p>TCP retransmits over time. Retransmits can happen because CAKE is adjusting packet pacing, but large volumes are often indicative of a network problem - particularly customer Wi-Fi problems.</p>"
    }

    subscribeTo() {
        return [ "Retransmits" ];
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
        this.graph = new RetransmitsGraph(this.graphDivId());
        this.ltsLoaded = false;
        if (window.hasLts) {
            this.graphDivs.forEach((g) => {
                let period = periodNameToSeconds(g.id.replace(this.graphDivId() + "_", ""));
                let graph = new LtsRetransmitsGraph(g.id, period);
                this.graphs.push(graph);
            });
        }
    }

    onMessage(msg) {
        if (msg.event === "Retransmits") {
            this.graph.update(msg.data.down, msg.data.up, msg.data.tcp_down, msg.data.tcp_up);
            if (!this.ltsLoaded && window.hasLts) {
                this.graphs.forEach((g) => {
                    if (g && g.chart) {
                        g.chart.showLoading("Insight retransmit history not available.");
                    }
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
