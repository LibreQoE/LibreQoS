import {DashboardGraph} from "./dashboard_graph";
import {lerpColor, lerpGreenToRedViaOrange} from "../helpers/scaling";
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

        this.chart.on('click', (params) => {
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
        });
    }

    processMessage(msg) {
        let nodes = [];
        let links = [];

        nodes.push({
            name: "Root",
            label: "Root",
        });

        this.nodeMap = {};
        msg.data.forEach((r) => {
            this.nodeMap[r.ip_address] = r.circuit_id;

            let label = {
                fontSize: 9,
                color: "#999"
            };
            if (isRedacted()) label.fontFamily = "Illegible";

            let name = r.ip_address+ " (" + scaleNumber(r.bits_per_second.down, 0) + ", " + r.tcp_retransmits[0].toFixed(1) + "/" + r.tcp_retransmits[1].toFixed(1) + ")";
            let bytes = r.bits_per_second.down / 8;
            let bytesAsMegabits = bytes / 1000000;
            let maxBytes = r.plan.down / 8;
            let percent = Math.min(100, (bytesAsMegabits / maxBytes) * 100);
            let capacityColor = lerpGreenToRedViaOrange(100 - percent, 100);

            let rttColor = lerpGreenToRedViaOrange(200 - r.median_tcp_rtt, 200);

            let percentRxmit = Math.min(100, r.tcp_retransmits.down + r.tcp_retransmits.up) / 100;
            let rxmitColor = lerpColor([0, 255, 0], [255, 0, 0], percentRxmit);

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