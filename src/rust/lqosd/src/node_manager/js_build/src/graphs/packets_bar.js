import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../lq_js_common/helpers/scaling";

export class PacketsPerSecondBar extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            grid: {
                x: '15%',
            },
            xAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (value) => { return scaleNumber(value, 0); }
                }
            },
            yAxis: {
                type: 'category',
                data: ['TCP', 'UDP', 'ICMP', 'Other'],
            },
            series: [
                {
                    type: 'bar',
                    data: [0, 0, 0, 0],
                },
                {
                    type: 'bar',
                    data: [0, 0, 0, 0],
                }
            ]
        }
        this.option && this.chart.setOption(this.option);
    }

    update(down, up, tcp, udp, icmp) {
        this.chart.hideLoading();
        this.option.series[0].data = [
            tcp.down,
            udp.down,
            icmp.down,
            Math.max(0, down - (tcp.down + udp.down + icmp.down)),
        ];
        this.option.series[1].data = [
            tcp.up,
            udp.up,
            icmp.up,
            Math.max(0, up - (tcp.up + udp.up + icmp.up)),
        ];
        this.chart.setOption(this.option);
    }
}