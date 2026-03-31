import {DashboardGraph} from "./dashboard_graph";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {
    CAKE_CHART_WINDOW_SECONDS,
    cakeChartTitle,
    cakeCommonGrid,
    cakeCommonXAxis,
    cakeHistoryWindow,
    cakeScatterSeries,
    cakeTooltip,
    formatCakeBitsPerSecond,
} from "./cake_history";

export class CakeTraffic extends DashboardGraph {
    constructor(id) {
        super(id);

        this.option = {
            title: cakeChartTitle("Traffic", "Bits per second"),
            grid: cakeCommonGrid(),
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                textStyle: {
                    color: '#aaa'
                },
                data: [
                    {
                        name: "Bulk",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }, {
                        name: "Best Effort",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[1]
                        }
                    }, {
                        name: "RT Video",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[2]
                        }
                    }, {
                        name: "Voice",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[3]
                        }
                    }
                ]
            },
            xAxis: cakeCommonXAxis(CAKE_CHART_WINDOW_SECONDS),
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => formatCakeBitsPerSecond(val),
                }
            },
            series: [
                cakeScatterSeries("Bulk", window.graphPalette[0]),
                cakeScatterSeries("Best Effort", window.graphPalette[1]),
                cakeScatterSeries("RT Video", window.graphPalette[2]),
                cakeScatterSeries("Voice", window.graphPalette[3]),
                cakeScatterSeries("Bulk Up", window.graphPalette[0]),
                cakeScatterSeries("Best Effort Up", window.graphPalette[1]),
                cakeScatterSeries("RT Video Up", window.graphPalette[2]),
                cakeScatterSeries("RT Voice Up", window.graphPalette[3]),
            ],
            tooltip: cakeTooltip(formatCakeBitsPerSecond),
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[1];
        this.option.legend.data[2].itemStyle.color = window.graphPalette[2];
        this.option.legend.data[3].itemStyle.color = window.graphPalette[3];
        this.option.series[0].itemStyle.color = window.graphPalette[0];
        this.option.series[1].itemStyle.color = window.graphPalette[1];
        this.option.series[2].itemStyle.color = window.graphPalette[2];
        this.option.series[3].itemStyle.color = window.graphPalette[3];
        this.option.series[4].itemStyle.color = window.graphPalette[0];
        this.option.series[5].itemStyle.color = window.graphPalette[1];
        this.option.series[6].itemStyle.color = window.graphPalette[2];
        this.option.series[7].itemStyle.color = window.graphPalette[3];
        this.chart.setOption(this.option);
    }

    update(msg) {
        this.chart.hideLoading();

        for (let i=0; i<8; i++) {
            this.option.series[i].data = [];
        }
        for (const sample of cakeHistoryWindow(msg, CAKE_CHART_WINDOW_SECONDS)) {
            for (let j=0; j<4; j++) {
                if (!sample || sample[0].tins[0] === undefined) {
                    this.option.series[j].data.push(null);
                    this.option.series[j+4].data.push(null);
                    continue;
                }
                const downBits = toNumber(sample[0].tins[j].sent_bytes, 0) * 8;
                const upBits = toNumber(sample[1].tins[j].sent_bytes, 0) * 8;
                this.option.series[j].data.push(downBits);
                this.option.series[j+4].data.push(0 - upBits);
            }
        }

        this.chart.setOption(this.option);
    }
}
