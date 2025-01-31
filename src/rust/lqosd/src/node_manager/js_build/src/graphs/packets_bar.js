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

        this.option.series[0].data.push(tcp.down);
        if (this.option.series[0].data.length > 300) {
            this.option.series[0].data.shift();
        }
        this.option.series[1].data.push(-tcp.up);
        if (this.option.series[1].data.length > 300) {
            this.option.series[1].data.shift();
        }
        this.option.series[2].data.push(udp.down);
        if (this.option.series[2].data.length > 300) {
            this.option.series[2].data.shift();
        }
        this.option.series[3].data.push(-udp.up);
        if (this.option.series[3].data.length > 300) {
            this.option.series[3].data.shift();
        }
        this.option.series[4].data.push(icmp.down);
        if (this.option.series[4].data.length > 300) {
            this.option.series[4].data.shift();
        }

        this.chart.setOption(this.option);
    }
}