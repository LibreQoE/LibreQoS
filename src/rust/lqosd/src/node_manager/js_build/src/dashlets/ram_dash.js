import {BaseDashlet} from "./base_dashlet";
import {RttHistogram} from "../graphs/rtt_histo";
import {RamPie} from "../graphs/ram_pie";

export class RamDash extends BaseDashlet{
    constructor(slot) {
        super(slot);
    }

    title() {
        return "RAM Utilization";
    }

    subscribeTo() {
        return [ "Ram" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new RamPie(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Ram") {
            let total = msg.data.total;
            let used = msg.data.used;
            let free = total - used;
            this.graph.update(free, used);
        }
    }
}