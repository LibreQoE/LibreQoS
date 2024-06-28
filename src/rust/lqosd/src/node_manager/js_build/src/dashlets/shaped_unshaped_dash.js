import {BaseDashlet} from "./base_dashlet";
import {ShapedUnshapedPie} from "../graphs/shaped_unshaped_pie";

export class ShapedUnshapedDash extends BaseDashlet{
    title() {
        return "Shaped/Unshaped Traffic";
    }

    subscribeTo() {
        return [ "Throughput" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new ShapedUnshapedPie(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "Throughput") {
            let shaped = msg.data.shaped_bps[0] + msg.data.shaped_bps[1];
            let unshaped = msg.data.bps[0] + msg.data.bps[1];
            this.graph.update(shaped, unshaped);
        }
    }
}