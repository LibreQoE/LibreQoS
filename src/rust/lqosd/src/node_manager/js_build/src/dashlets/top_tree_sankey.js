import {BaseDashlet} from "../lq_js_common/dashboard/base_dashlet";
import {DashboardGraph} from "../graphs/dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {isColorBlindMode} from "../helpers/colorblind";

/**
 * Viridis color scale interpolation (0-1 input).
 * Returns hex color string.
 */
function lerpViridis(t) {
    // Viridis colormap sampled at 6 points, interpolated linearly
    const stops = [
        [68, 1, 84],    // #440154
        [59, 82, 139],  // #3B528B
        [33, 145, 140], // #21918C
        [94, 201, 98],  // #5EC962
        [253, 231, 37]  // #FDE725
    ];
    if (t <= 0) return "#440154";
    if (t >= 1) return "#FDE725";
    let idx = t * (stops.length - 1);
    let i = Math.floor(idx);
    let frac = idx - i;
    let c0 = stops[i], c1 = stops[i + 1];
    let r = Math.round(c0[0] + frac * (c1[0] - c0[0]));
    let g = Math.round(c0[1] + frac * (c1[1] - c0[1]));
    let b = Math.round(c0[2] + frac * (c1[2] - c0[2]));
    return "#" + ((1 << 24) + (r << 16) + (g << 8) + b).toString(16).slice(1);
}
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

            msg.data.slice(1).forEach((r) => {
                let label = {
                    fontSize: 9,
                    color: "#999"
                };
                if (redact) label.fontFamily = "Illegible";

                let name = r[1].name;
                let bytes = r[1].current_throughput[0];
                let bytesAsMegabits = bytes / 1000000;
                let maxBytes = r[1].max_throughput[0] / 8;
                let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
                let capacityColor = isColorBlindMode()
                    ? lerpViridis(percent / 100)
                    : lerpGreenToRedViaOrange(100 - percent, 100);

                if (r[1].rtts.length > 0) {
                    lastRtt[name] = r[1].rtts[0];
                } else {
                    lastRtt[name] = 0;
                }
                let rttPercent = Math.min(100, (lastRtt[name] / 200) * 100);
                let color = isColorBlindMode()
                    ? lerpViridis(rttPercent / 100)
                    : lerpGreenToRedViaOrange(200 - lastRtt[name], 200);

                if (bytesAsMegabits > 0) {
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
                }
            });
            this.graph.update(nodes, links);
        }
    }
}
