import {DashboardGraph} from "./dashboard_graph";
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
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {isRedacted} from "../helpers/redact";

export class TopNSankey extends DashboardGraph {
    constructor(id) {
        super(id);
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

        /*this.chart.on('click', (params) => {
            let name = params.name;
            // Trim to before " ("
            name = name.substring(0, name.indexOf(" ("));
            if (name.indexOf(" > ") === -1) {
                if (this.nodeMap[name] !== undefined) {
                    window.location.href = "/circuit.html?id=" + encodeURI(this.nodeMap[name]);
                }
            } else {
                let actualName = params.data.target;
                actualName = actualName.substring(0, actualName.indexOf(" ("));
                if (this.nodeMap[actualName] !== undefined) {
                    window.location.href = "/circuit.html?id=" + encodeURI(this.nodeMap[actualName]);
                }
            }
        });*/
    }

    processMessage(msg) {
        let nodes = [];
        let links = [];

        nodes.push({
            name: "Root",
            label: "Root",
            itemStyle: {
                color: "#440154",
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
            let bytes = r.bits_per_second.down / 8;
            let bytesAsMegabits = bytes / 1000000;
            let maxBytes = r.plan.down / 8;
            let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
            let capacityColor = lerpViridis(percent / 100);
            
            let rtt = Math.max(Math.min(r.median_tcp_rtt, 200), 0);
            let rttColor = lerpViridis(rtt / 200);
            
            let percentRxmit = Math.min(100, r.tcp_retransmits[0] + r.tcp_retransmits[1]) / 100;
            let rxmitColor = lerpViridis(percentRxmit);
            
            nodes.push({
                name: name,
                label: label,
                itemStyle: {
                    color: rxmitColor,
                    borderWidth: 4,
                    borderColor: rttColor,
                }
            });
            
            links.push({
                source: "Root",
                target: name,
                value: r.bits_per_second.down,
                lineStyle: {
                    color: capacityColor
                }
            });
        });

        this.update(nodes, links);
    }
}