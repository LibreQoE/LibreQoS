import {BaseDashlet} from "./base_dashlet";
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
        this.graph = new CpuHistogram(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Cpu") {
            this.graph.update(msg.data);
        }
    }
}