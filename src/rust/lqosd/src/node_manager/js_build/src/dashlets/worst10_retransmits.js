import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, TopNTableFromMsgData} from "../helpers/builders";
import {TopXTable} from "../helpers/top_x_cache";

export class Worst10Retransmits extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.buffer = new TopXTable(10, 10);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Worst 10 TCP Re-transmits";
    }

    tooltip() {
        return "<h5>Worst 10 TCP Re-transmits</h5><p>Worst 10 Downloaders by TCP retransmits, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
    }

    subscribeTo() {
        return [ "WorstRetransmits" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "WorstRetransmits") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                this.buffer.push(r.circuit_id, r, (data) => {
                    return data.tcp_retransmits.down;
                });
            });
            let ranked = this.buffer.getRankedArrayAndClean();

            let t = TopNTableFromMsgData(ranked);

            // Display it
            clearDashDiv(this.id, target);
            target.appendChild(t);
        }
    }
}