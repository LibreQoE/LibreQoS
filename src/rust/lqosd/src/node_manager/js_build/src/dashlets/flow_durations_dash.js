import {FlowDurationsGraph} from "../graphs/flow_durations_graph";
import {DashletBaseInsight} from "./insight_dashlet_base";
import {t} from "../i18n/index.js";

export class FlowDurationDash extends DashletBaseInsight{
    constructor(slot) {
        super(slot);
    }

    title() {
        return t("dashlet.name.flow_duration");
    }

    tooltip() {
        return `<h5>${this.title()}</h5><p>${t("dashlet.flow_duration.tooltip")}</p>`;
    }

    subscribeTo() {
        return [ "FlowDurations" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new FlowDurationsGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "FlowDurations") {
            this.graph.update(msg.data);
        }
    }
}