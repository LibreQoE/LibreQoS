import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {periodNameToSeconds} from "../helpers/time_periods";
import {get_ws_client} from "../pubsub/ws";

const wsClient = get_ws_client();

const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class PacketsPerSecondTimescale extends DashboardGraph {
    constructor(id, period) {
        super(id);
        this.period = period;

        this.option = {
            xAxis: {
                type: 'category',
                data: [],
                axisLabel: {
                    formatter: function (val)
                    {
                        return new Date(parseInt(val) * 1000).toLocaleString();
                    },
                    hideOverlap: true
                }
            },
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => {
                        return scaleNumber(Math.abs(val), 0);
                    },
                }
            },
            series: [],
            legend: {
                data: []
            },
        };
        this.option && this.chart.setOption(this.option);

        let seconds = periodNameToSeconds(period);
        console.log("Requesting Insight History Data");
        listenOnceForSeconds("LtsPackets", seconds, (msg) => {
            const data = msg && msg.data ? msg.data : [];

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

            for (let x=0; x<data.length; x++) {
                this.option.xAxis.data.push(data[x].time);
                seriesTcpDown.data.push(data[x].max_tcp_down);
                SeriesTcpUp.data.push((0.0 - data[x].max_tcp_up));

                seriesUdpDown.data.push(data[x].max_udp_down);
                SeriesUdpUp.data.push((0.0 - data[x].max_udp_up));

                seriesIcmpDown.data.push(data[x].max_icmp_down);
                SeriesIcmpUp.data.push((0.0 - data[x].max_icmp_up));
            }

            this.option.series.push(seriesTcpDown);
            this.option.series.push(SeriesTcpUp);
            this.option.legend.data.push("TCP");

            this.option.series.push(seriesUdpDown);
            this.option.series.push(SeriesUdpUp);
            this.option.legend.data.push("UDP");

            this.option.series.push(seriesIcmpDown);
            this.option.series.push(SeriesIcmpUp);
            this.option.legend.data.push("ICMP");

            this.chart.setOption(this.option);
            this.chart.hideLoading();
        });
        wsClient.send({ LtsPackets: { seconds } });
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
