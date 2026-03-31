import {DashboardGraph} from "./dashboard_graph";
import {
    CAKE_CHART_WINDOW_SECONDS,
    cakeChartTitle,
    cakeCommonGrid,
    cakeCommonXAxis,
    cakeHistoryWindow,
    cakeScatterSeries,
    cakeTooltip,
    formatCakePackets,
} from "./cake_history";

export class CakeQueueLength extends DashboardGraph {
    constructor(id) {
        super(id);

        this.option = {
            title: cakeChartTitle("Queue Length", "Packets"),
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
                        name: "Queue Length",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }
                ]
            },
            xAxis: cakeCommonXAxis(CAKE_CHART_WINDOW_SECONDS),
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => formatCakePackets(val),
                },
            },
            series: [
                cakeScatterSeries("Queue Length", window.graphPalette[0]),
                cakeScatterSeries("Queue Length Up", window.graphPalette[4] || window.graphPalette[0]),
            ],
            tooltip: cakeTooltip(formatCakePackets),
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.series[0].itemStyle.color = window.graphPalette[0];
        this.option.series[1].itemStyle.color = window.graphPalette[4] || window.graphPalette[0];
        this.chart.setOption(this.option);
    }

    update(msg) {
        this.chart.hideLoading();

        this.option.series[0].data = [];
        this.option.series[1].data = [];

        for (const sample of cakeHistoryWindow(msg, CAKE_CHART_WINDOW_SECONDS)) {
            if (!sample) {
                this.option.series[0].data.push(null);
                this.option.series[1].data.push(null);
                continue;
            }
            this.option.series[0].data.push(sample[0].qlen);
            this.option.series[1].data.push(0 - sample[1].qlen);
        }

        this.chart.setOption(this.option);
    }
}
