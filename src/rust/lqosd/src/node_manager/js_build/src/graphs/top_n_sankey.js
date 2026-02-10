import {DashboardGraph} from "./dashboard_graph";
import {lerpColor, lerpGreenToRedViaOrange} from "../helpers/scaling";
import {isColorBlindMode} from "../helpers/colorblind";
import {toNumber} from "../lq_js_common/helpers/scaling";
/**
 * Viridis color scale interpolation (0-1 input).
 * Returns hex color string.
 */
function lerpViridis(t) {
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

export class TopNSankey extends DashboardGraph {
    constructor(id, upload=false) {
        super(id);
        this.upload = upload;
        this.nodeMap = {};
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
    }

    update(data, links) {
        this.option.series[0].data = data;
        this.option.series[0].links = links;
        this.chart.hideLoading();
        this.chart.setOption(this.option);
    }

    processMessage(msg) {
        let nodes = [];
        let links = [];

        nodes.push({
            name: "Root",
            label: "Root",
            itemStyle: {
                color: isColorBlindMode() ? "#440154" : "green",
                borderWidth: 1,
            }
        });

        this.nodeMap = {};
        msg.data.forEach((r) => {
            this.nodeMap[r.ip_address] = r.circuit_id;

            let label = {
                fontSize: 9,
                color: "#999"
            };
            if (isRedacted()) label.fontFamily = "Illegible";

            let name = r.ip_address;
            // Choose the correct direction for value and capacity coloring
            const bps = toNumber(this.upload ? r.bits_per_second.up : r.bits_per_second.down, 0);
            const planMbps = toNumber(this.upload ? r.plan.up : r.plan.down, 0);
            // Convert bits/s to MB/s (decimal) and Mbps plan to MB/s for a comparable ratio
            const bytes = bps / 8;
            const bytesAsMegabytes = bytes / 1000000;
            const maxBytes = planMbps / 8;
            const percent = Math.min(100, (maxBytes > 0 ? (bytesAsMegabytes / maxBytes) * 100 : 0));
            let capacityColor = isColorBlindMode()
                ? lerpViridis(percent / 100)
                : lerpGreenToRedViaOrange(100 - percent, 100);
            
            let rtt = Math.max(Math.min(toNumber(r.median_tcp_rtt, 0), 200), 0);
            let rttColor = isColorBlindMode()
                ? lerpViridis(rtt / 200)
                : lerpGreenToRedViaOrange(200 - rtt, 200);
            
            let percentRxmit = Math.min(100, toNumber(r.tcp_retransmits[0], 0) + toNumber(r.tcp_retransmits[1], 0)) / 100;
            let rxmitColor = isColorBlindMode()
                ? lerpViridis(percentRxmit)
                : lerpColor([0, 255, 0], [255, 0, 0], percentRxmit);
            
            nodes.push({
                name: name,
                label: label,
                itemStyle: {
                    color: rxmitColor,
                    borderWidth: 4,
                    borderColor: rttColor,
                }
            });

            let value = bps;
            links.push({
                source: "Root",
                target: name,
                value,
                lineStyle: {
                    color: capacityColor
                }
            });
        });

        this.update(nodes, links);
    }
}
