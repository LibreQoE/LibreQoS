import {periodNameToSeconds} from "../helpers/time_periods";
import {RetransmitsGraph} from "../graphs/retransmits_graph";
import {LtsRetransmitsGraph} from "../graphs/lts_retransmits";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

// translatable: false — "Live" is the time-period token the period buttons compare against, not a label
// translatable: false — "Retransmits" is the websocket event name the dashlet filters on, not a label


export class TcpRetransmitsDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
        this.counter = 0;
    }

    title() {
        return t("dashlet.name.total_tcp_retransmits");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.retransmits.tooltip")}</p>`
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
            targets.forEach((period) => {
                let graph = document.createElement("div");
                graph.id = this.graphDivId() + "_" + period;
                graph.classList.add("dashgraph");
                graph.style.display = "none";
                graph.innerHTML = window.hasLts ? t("common.state.loading") : `<p class="text-secondary small">${t("common.insight.entitled_history")}</p>`;
                this.graphDivs.push(graph);
                controls.appendChild(this.makePeriodBtn(period));
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

    setupZoomed() {
        // Zoom the live retransmit graph (dashlet-local period controls remain in the main card).
        this.zoomGraph = new RetransmitsGraph(this.zoomGraphDivId());
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "Retransmits") {
            this.graph.update(msg.data.down, msg.data.up, msg.data.tcp_down, msg.data.tcp_up);
            if (this.zoomGraph) {
                this.zoomGraph.update(msg.data.down, msg.data.up, msg.data.tcp_down, msg.data.tcp_up);
            }
            if (!this.ltsLoaded && window.hasLts) {
                this.graphs.forEach((g) => {
                    if (g && g.chart) {
                        g.chart.showLoading(t("dashlet.retransmits.no_history"));
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
