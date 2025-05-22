import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";

export class PacketsPerSecondBar extends DashboardGraph {
    constructor(id) {
        super(id);

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, 300)
            .withScaledAbsYAxis("Packets", 40)
            .withEmptySeries()
            .withLeftGridSize("18%")
            .build();
        this.option.legend = { data: [] };

        // Enable axisPointer and custom tooltip
        this.option.tooltip = {
            trigger: 'axis',
            axisPointer: {
                type: 'cross',
                link: { xAxisIndex: 'all' },
                label: {
                    backgroundColor: '#6a7985'
                }
            },
            formatter: (params) => {
                // params is an array of series data at the hovered index
                let lines = [];
                params.forEach(param => {
                    if (param.data && typeof param.data === 'object') {
                        let val = param.data.value;
                        let ts = param.data.timestamp;
                        let dateStr = ts ? new Date(ts).toLocaleTimeString('en-US', { hour12: false }) : '';
                        lines.push(
                            param.marker + param.seriesName + ': ' + val + ' @ ' + dateStr
                        );
                    } else {
                        lines.push(
                            param.marker + param.seriesName + ': ' + param.data
                        );
                    }
                });
                return lines.join('<br>');
            }
        };

        let n = 1;
        let seriesTcpDown = {
            type: 'line',
            data: [],
            name: "TCP",
            smooth: true,
            itemStyle: {
                color: window.graphPalette[n]
            },
            areaStyle: { color: window.graphPalette[n] },
            stack: 'down',
        };
        let SeriesTcpUp = {
            type: 'line',
            data: [],
            name: "TCP",
            smooth: true,
            itemStyle: {
                color: window.graphPalette[n]
            },
            areaStyle: { color: window.graphPalette[n] },
            stack: 'up',
        };

        // ICMP
        n++;
        let seriesIcmpDown = {
            type: 'line',
            data: [],
            name: "ICMP",
            smooth: true,
            itemStyle: {
                color: window.graphPalette[n]
            },
            areaStyle: { color: window.graphPalette[n] },
            stack: 'down',
        };
        let SeriesIcmpUp = {
            type: 'line',
            data: [],
            name: "ICMP",
            smooth: true,
            itemStyle: {
                color: window.graphPalette[n]
            },
            areaStyle: { color: window.graphPalette[n] },
            stack: 'up',
        };

        // UDP
        n++;
        let seriesUdpDown = {
            type: 'line',
            data: [],
            name: "UDP",
            smooth: true,
            itemStyle: {
                color: window.graphPalette[n]
            },
            areaStyle: { color: window.graphPalette[n] },
            stack: 'down',
        };
        let SeriesUdpUp = {
            type: 'line',
            data: [],
            name: "UDP",
            smooth: true,
            itemStyle: {
                color: window.graphPalette[n]
            },
            areaStyle: { color: window.graphPalette[n] },
            stack: 'up',
        };

        this.option.series.push(seriesTcpDown);
        this.option.series.push(SeriesTcpUp);
        this.option.legend.data.push("TCP");

        this.option.series.push(seriesUdpDown);
        this.option.series.push(SeriesUdpUp);
        this.option.legend.data.push("UDP");

        this.option.series.push(seriesIcmpDown);
        this.option.series.push(SeriesIcmpUp);
        this.option.legend.data.push("ICMP");

        this.option && this.chart.setOption(this.option);
    }

    update(down, up, tcp, udp, icmp) {
        this.chart.hideLoading();

        const now = Date.now();
        this.option.series[0].data.push({ value: tcp.down, timestamp: now });
        if (this.option.series[0].data.length > 300) {
            this.option.series[0].data.shift();
        }
        this.option.series[1].data.push({ value: -tcp.up, timestamp: now });
        if (this.option.series[1].data.length > 300) {
            this.option.series[1].data.shift();
        }
        this.option.series[2].data.push({ value: udp.down, timestamp: now });
        if (this.option.series[2].data.length > 300) {
            this.option.series[2].data.shift();
        }
        this.option.series[3].data.push({ value: -udp.up, timestamp: now });
        if (this.option.series[3].data.length > 300) {
            this.option.series[3].data.shift();
        }
        this.option.series[4].data.push({ value: icmp.down, timestamp: now });
        if (this.option.series[4].data.length > 300) {
            this.option.series[4].data.shift();
        }

        this.chart.setOption(this.option);
    }
}