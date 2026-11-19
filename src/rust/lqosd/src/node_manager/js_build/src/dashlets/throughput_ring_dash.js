import {ThroughputRingBufferGraph} from "../graphs/throughput_ring_graph";
import {ThroughputRingBufferGraphTimescale} from "../graphs/throughput_ring_graph_timescale";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t, tn} from "../i18n/index.js";

// translatable: false — "Live" is the time-period token the page compares against, not a label
// translatable: false — "Throughput" is the websocket event name this dashlet subscribes to
// translatable: false — ".dashbox-title [data-bs-toggle='tooltip']" is a selector, not text
// translatable: false — "_zoomed" is the element-id suffix of the zoom clone

export class ThroughputRingDash extends DashletBaseInsight{
    // `.66.3`: the 30 s Executive instance was REPLACED by `LiveTrafficPulseDash`
    // (the `throughputRing30` tag now maps to the pulse, which is Live-only). This
    // class is the 5-MINUTE `throughputRing` widget only, so the window is fixed
    // at 300 s. Retired here: the `.64.3.1` A4 `windowSeconds === 30` body-height
    // branch and the A3 `.dashbox-counters` (the pulse owns its own legend +
    // big-number row). The handle `.dashbox-legend` STAYS for the 5-min widget.
    constructor(slot) {
        super(slot);
        this.windowSeconds = 300;
        this.counter = 0;
        this.backgroundSamples = [];
    }

    currentPeriod() {
        if (window.timePeriods && window.timePeriods.activePeriod) {
            return window.timePeriods.activePeriod;
        }
        return "Live";
    }

    periodLabel() {
        switch (this.currentPeriod()) {
            case "1h": return t("dashlet.throughput.ring.period.1h");
            case "6h": return t("dashlet.throughput.ring.period.6h");
            case "12h": return t("dashlet.throughput.ring.period.12h");
            case "24h": return t("dashlet.throughput.ring.period.24h");
            case "7d": return t("dashlet.throughput.ring.period.7d");
            default:
                return tn("dashlet.throughput.ring.period.minutes", Math.round(this.windowSeconds / 60));
        }
    }

    periodDescription() {
        switch (this.currentPeriod()) {
            case "1h": return t("dashlet.throughput.ring.describe.1h");
            case "6h": return t("dashlet.throughput.ring.describe.6h");
            case "12h": return t("dashlet.throughput.ring.describe.12h");
            case "24h": return t("dashlet.throughput.ring.describe.24h");
            case "7d": return t("dashlet.throughput.ring.describe.7d");
            default:
                return tn("dashlet.throughput.ring.describe.minutes", Math.round(this.windowSeconds / 60));
        }
    }

    title() {
        return t("dashlet.throughput.ring.title", {period: this.periodLabel()});
    }

    tooltip() {
        return t("dashlet.throughput.ring.tooltip", {window: this.periodDescription()});
    }

    subscribeTo() {
        return [ "Throughput" ];
    }

    keepAliveWhenHidden() {
        return true;
    }

    keepSubscribedWhenHidden() {
        return true;
    }

    buildContainer() {
        let base = super.buildContainer();
        let graphs = this.graphDiv();

        // Add some time controls
        base.classList.add("dashlet-with-controls");
        let controls = document.createElement("div");
        controls.classList.add("dashgraph-controls", "small");

        // `.64.2.3` (B, option b): the Mapped/Unmapped key lives in the widget
        // handle (for the 5-min widget only; the `.66.3` pulse supersedes it with
        // its own big-number row). Kept reactive to theme + CVD.
        const handle = base.querySelector(".dashbox-title");
        if (handle) {
            const legend = document.createElement("span");
            legend.className = "dashbox-legend";
            legend.innerHTML = this.legendKeyHtml();
            handle.appendChild(legend);
            this._legendEl = legend;
            this._themeObserver = new MutationObserver(() => this.updateLegendKey());
            this._themeObserver.observe(document.documentElement, {
                attributes: true,
                attributeFilter: ["data-bs-theme"],
            });
            this._colorBlindHandler = () => this.updateLegendKey();
            window.addEventListener("colorBlindModeChanged", this._colorBlindHandler);
        }

        base.appendChild(controls);
        base.appendChild(graphs);
        this.graphDivs.forEach((g) => {
            base.appendChild(g);
        });
        return base;
    }

    legendKeyHtml() {
        const palette = window.graphPalette || [];
        const dot = (color) => `<span class="dashbox-legend-dot" style="background:${color}"></span>`;
        return `${dot(palette[0])}<span>${t("graph.pie.mapped")}</span> ${dot(palette[1])}<span>${t("graph.pie.unmapped")}</span>`;
    }

    // `.64.2.3`: re-render the handle key from the CURRENT palette (the dots were
    // baked once in buildContainer). Self-disconnects once the widget is removed.
    updateLegendKey() {
        if (!this._legendEl) {
            return;
        }
        if (!this._legendEl.isConnected) {
            if (this._themeObserver) {
                this._themeObserver.disconnect();
                this._themeObserver = null;
            }
            if (this._colorBlindHandler) {
                window.removeEventListener("colorBlindModeChanged", this._colorBlindHandler);
                this._colorBlindHandler = null;
            }
            return;
        }
        this._legendEl.innerHTML = this.legendKeyHtml();
    }

    setup() {
        super.setup();
        this.graph = new ThroughputRingBufferGraph(this.graphDivId(), this.windowSeconds);
        window.timeGraphs.push(this);
    }

    setupZoomed() {
        const period = this.currentPeriod();
        if (period === "Live") {
            this.zoomGraph = new ThroughputRingBufferGraph(this.zoomGraphDivId(), this.windowSeconds);
        } else {
            this.zoomGraph = new ThroughputRingBufferGraphTimescale(this.zoomGraphDivId(), period);
        }
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    // `.64.2.3.1` (R3): tear down the handle's theme MutationObserver + the
    // `colorBlindModeChanged` listener immediately on a layout rebuild. Without
    // this, a REMOVED dashlet's observer/listener persist until the next theme
    // flip (BaseDashlet.dispose() is called on a rebuild). Idempotent: both
    // fields are nulled, so a second dispose is a no-op.
    dispose() {
        if (this._themeObserver) {
            this._themeObserver.disconnect();
            this._themeObserver = null;
        }
        if (this._colorBlindHandler) {
            window.removeEventListener("colorBlindModeChanged", this._colorBlindHandler);
            this._colorBlindHandler = null;
        }
        super.dispose();
    }

    onMessage(msg) {
        if (msg.event !== "Throughput") {
            return;
        }
        if (window.timePeriods.activePeriod === "Live") {
            this.#applyLiveSample(msg.data.shaped_bps, msg.data.bps);

            this.counter++;
            if (this.counter > 120) {
                // Reload the LTS graphs every 2 minutes
                this.counter = 0;
                this.ltsLoaded = false;
            }
        }
    }

    onBackgroundMessage(msg) {
        if (msg.event !== "Throughput" || window.timePeriods.activePeriod !== "Live") {
            return;
        }
        this.backgroundSamples.push({
            shaped: msg.data.shaped_bps,
            unshaped: msg.data.bps,
        });
        if (this.backgroundSamples.length > this.windowSeconds) {
            this.backgroundSamples.shift();
        }
    }

    flushBackgroundMessages() {
        if (window.timePeriods.activePeriod !== "Live" || this.backgroundSamples.length === 0) {
            this.backgroundSamples = [];
            return false;
        }
        for (let i = 0; i < this.backgroundSamples.length; i++) {
            const sample = this.backgroundSamples[i];
            this.#applyLiveSample(sample.shaped, sample.unshaped);
        }
        this.backgroundSamples = [];
        return true;
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
        let tooltipAnchor = container.querySelector(".dashbox-title [data-bs-toggle='tooltip']");
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
        this.backgroundSamples = [];
        this.updateTitlesAndTooltip();
        this.graph.chart.clear();
        this.graph.chart.showLoading();
        if (window.timePeriods.activePeriod === "Live") {
            this.graph = new ThroughputRingBufferGraph(this.graphDivId(), this.windowSeconds);
        } else {
            this.graph = new ThroughputRingBufferGraphTimescale(this.graphDivId(), window.timePeriods.activePeriod);
        }
        if (this.zoomed) {
            this.teardownZoomed();
            this.setupZoomed();
        }
    }

    #applyLiveSample(shaped, unshaped) {
        this.graph.update(shaped, unshaped);
        if (this.zoomGraph) {
            this.zoomGraph.update(shaped, unshaped);
        }
    }
}
