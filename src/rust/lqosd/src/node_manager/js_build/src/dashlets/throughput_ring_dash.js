import {ThroughputRingBufferGraph} from "../graphs/throughput_ring_graph";
import {ThroughputRingBufferGraphTimescale} from "../graphs/throughput_ring_graph_timescale";
import {DashletBaseInsight} from "./insight_dashlet_base";

export class ThroughputRingDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
        this.counter = 0;
    }

    currentPeriod() {
        if (window.timePeriods && window.timePeriods.activePeriod) {
            return window.timePeriods.activePeriod;
        }
        return "Live";
    }

    periodLabel() {
        switch (this.currentPeriod()) {
            case "1h": return "Last 1 Hour";
            case "6h": return "Last 6 Hours";
            case "12h": return "Last 12 Hours";
            case "24h": return "Last 24 Hours";
            case "7d": return "Last 7 Days";
            default: return "Last 5 Minutes";
        }
    }

    periodDescription() {
        switch (this.currentPeriod()) {
            case "1h": return "the last 1 hour";
            case "6h": return "the last 6 hours";
            case "12h": return "the last 12 hours";
            case "24h": return "the last 24 hours";
            case "7d": return "the last 7 days";
            default: return "the last five minutes";
        }
    }

    title() {
        return this.periodLabel() + " Traffic";
    }

    tooltip() {
        return "<h5>" + this.periodLabel() + " Throughput</h5><p>Mapped (AQM controlled and limited) and Unmapped (not found in your Shaped Devices file) traffic over " + this.periodDescription() + ".</p>"
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
        window.timeGraphs.push(this);
    }

    onMessage(msg) {
        if (msg.event === "Throughput" && window.timePeriods.activePeriod === "Live") {
            this.graph.update(msg.data.shaped_bps, msg.data.bps);

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

    updateTitleFor(container, titleText) {
        if (!container) return;
        let titleEl = container.querySelector(".dashbox-title");
        if (!titleEl) return;
        let textNode = null;
        titleEl.childNodes.forEach((node) => {
            if (node.nodeType === 3) {
                textNode = node;
            }
        });
        if (textNode) {
            textNode.nodeValue = titleText;
        } else {
            titleEl.insertBefore(document.createTextNode(titleText), titleEl.firstChild);
        }
    }

    updateTooltip() {
        let container = document.getElementById(this.id);
        if (!container) return;
        let tooltipAnchor = container.querySelector(".dashbox-title a[data-bs-toggle='tooltip']");
        if (!tooltipAnchor) return;
        let html = this.tooltip();
        tooltipAnchor.setAttribute("title", html);
        tooltipAnchor.setAttribute("data-bs-original-title", html);
        tooltipAnchor.setAttribute("data-bs-title", html);
        if (typeof bootstrap === "undefined" || !bootstrap.Tooltip) return;
        if (bootstrap.Tooltip.getInstance) {
            let instance = bootstrap.Tooltip.getInstance(tooltipAnchor);
            if (instance && instance.dispose) {
                instance.dispose();
            }
        }
        if (bootstrap.Tooltip.getOrCreateInstance) {
            bootstrap.Tooltip.getOrCreateInstance(tooltipAnchor);
        } else {
            new bootstrap.Tooltip(tooltipAnchor);
        }
    }

    updateTitlesAndTooltip() {
        let titleText = this.title();
        this.updateTitleFor(document.getElementById(this.id), titleText);
        this.updateTitleFor(document.getElementById(this.id + "_zoomed"), titleText);
        this.updateTooltip();
    }

    onTimeChange() {
        super.onTimeChange();
        this.updateTitlesAndTooltip();
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new ThroughputRingBufferGraph(this.graphDivId());
        } else {
            this.graph = new ThroughputRingBufferGraphTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
    }
}
