import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {CpuHistogram} from "../graphs/cpu_graph";

export class CpuDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "CPU Utilization";
    }

    tooltip() {
        return "<h5>CPU Utilization</h5><p>Percentage of CPU time spent on user processes, system processes, and idle time. This includes both LibreQoS and anything else running on the server.</p>";
    }

    supportsZoom() {
        return true;
    }

    subscribeTo() {
        return [ "Cpu" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.traceRender("setup-start");
        this.graph = new CpuHistogram(this.graphDivId());
        this.traceRender("setup-complete", {
            graphId: this.graphDivId(),
        });
    }

    setupZoomed() {
        this.zoomGraph = new CpuHistogram(this.zoomGraphDivId());
    }

    teardownZoomed() {
        super.teardownZoomed();
        this.zoomGraph = null;
    }

    onMessage(msg) {
        if (msg.event === "Cpu") {
            this.traceRender("onMessage", {
                eventName: msg.event,
                cpuRows: Array.isArray(msg.data) ? msg.data.length : 0,
            });
            try {
                this.graph.update(msg.data);
                if (this.zoomGraph) {
                    this.zoomGraph.update(msg.data);
                }
                this.traceRender("update-ok", {
                    eventName: msg.event,
                });
            } catch (err) {
                this.traceRender("update-error", {
                    eventName: msg.event,
                    error: err && err.message ? err.message : String(err),
                });
                throw err;
            }
        }
    }
}
