import {BaseDashlet} from "./base_dashlet";
import {clearDashDiv, TopNTableFromMsgData} from "../helpers/builders";
import {TopXTable} from "../helpers/top_x_cache";

export class Top10Downloaders extends BaseDashlet {
    constructor(slot) {
        super(slot);
        this.buffer = new TopXTable(10, 10);
    }

    title() {
        return "Top 10 Downloaders";
    }

    tooltip() {
        return "<h5>Top 10 Downloaders</h5><p>Top 10 Downloaders by bits per second, including IP address, download and upload rates, round-trip time, TCP retransmits, and shaping plan.</p>";
    }

    subscribeTo() {
        return [ "TopDownloads" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.style.height = "250px";
        base.style.overflow = "auto";
        return base;
    }

    canBeSlowedDown() {
        return true;
    }

    setup() {
        super.setup();
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads") {
            let target = document.getElementById(this.id);

            msg.data.forEach((r) => {
                this.buffer.push(r.circuit_id, r, (data) => {
                    return data.bits_per_second.down;
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
