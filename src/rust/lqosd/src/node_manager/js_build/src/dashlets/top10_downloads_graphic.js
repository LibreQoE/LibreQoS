import {BaseDashlet} from "./base_dashlet";
import {TopNSankey} from "../graphs/top_n_sankey";

export class Top10DownloadersVisual extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    title() {
        return "Top 10 Downloaders (Visual)";
    }

    tooltip() {
        return "<h5>Top 10 Downloaders</h5><p>The top-10 users by bits-per-second. The ribbon goes red as they approach capacity. The name border indicates round-trip time, while the main label color indicates TCP retransmits. The legend is current download speed, followed by number of retransmits.</p>";
    }

    subscribeTo() {
        return [ "TopDownloads" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    canBeSlowedDown() {
        return true;
    }

    setup() {
        super.setup();
        this.graph = new TopNSankey(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "TopDownloads") {
            //console.log(msg);
            this.graph.processMessage(msg);
        }
    }
}