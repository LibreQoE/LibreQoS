import {BaseDashlet} from "./base_dashlet";
import {clearDiv, simpleRowHtml, theading} from "../helpers/builders";
import {formatThroughput, formatRetransmit, formatCakeStat, lerpGreenToRedViaOrange} from "../helpers/scaling";
import {DashboardGraph} from "../graphs/dashboard_graph";
import {isRedacted} from "../helpers/redact";

let lastRtt = {};

class TopTreeSankeyGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            series: [
                {
                    nodeAlign: 'left',
                    type: 'sankey',
                    data: [],
                    links: []
                }
            ]
        };
        this.option && this.chart.setOption(this.option);
        /*this.chart.on('click', (params) => {
            //console.log(params.name);
            let name = params.name;
            // If it contains a >, it's a link
            if (name.indexOf(" > ") === -1) {
                rootId = idOfNode(name);
            } else {
                rootId = idOfNode(params.data.source);
            }
        });*/
        //$("#btnRoot").click(() => { rootId = 0; });
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }
}

export class TopTreeSankey extends BaseDashlet {
    constructor(slot) {
        super(slot);
    }

    canBeSlowedDown() {
        return true;
    }

    title() {
        return "Top Level Sankey";
    }

    tooltip() {
        return "<h5>Network Tree</h5><p>Summary of the top-level network tree, rendered as a Sankey. Ribbon width shows relative bandwidth, ribbon color percentage of capacity, and node color overall RTT.</p>";
    }

    subscribeTo() {
        return [ "TreeSummary" ];
    }

    buildContainer() {
        let base = super.buildContainer();
        base.appendChild(this.graphDiv());
        return base;
    }

    setup() {
        super.setup();
        this.graph = new TopTreeSankeyGraph(this.graphDivId());
    }

    onMessage(msg) {
        if (msg.event === "TreeSummary") {
            //console.log(msg.data);

            let redact = isRedacted();

            let nodes = [];
            let links = [];

            nodes.push({
                name: "Root",
                label: "Root",
            });

            msg.data.forEach((r) => {
                let label = {
                    fontSize: 9,
                    color: "#999"
                };
                if (redact) label.fontSize = 0;

                let name = r[1].name;
                let bytes = r[1].current_throughput[0];
                let bytesAsMegabits = bytes / 1000000;
                let maxBytes = r[1].max_throughput[0] / 8;
                let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
                let capacityColor = lerpGreenToRedViaOrange(100 - percent, 100);

                if (r[1].rtts.length > 0) {
                    lastRtt[name] = r[1].rtts[0];
                } else {
                    lastRtt[name] = 0;
                }
                let color = lerpGreenToRedViaOrange(200 - lastRtt[name], 200);

                nodes.push({
                    name: r[1].name,
                    label: label,
                    itemStyle: {
                        color: color
                    }
                });
                links.push({
                    source: "Root",
                    target: r[1].name,
                    value: r[1].current_throughput[0] + r[1].current_throughput[1],
                    lineStyle: {
                        color: capacityColor,
                    }
                });
            });
            this.graph.update(nodes, links);
        }
    }
}