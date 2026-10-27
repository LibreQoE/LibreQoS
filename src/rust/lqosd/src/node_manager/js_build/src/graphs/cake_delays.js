import {DashboardGraph} from "./dashboard_graph";
import {
    CAKE_CHART_WINDOW_SECONDS,
    cakeChartTitle,
    cakeCommonGrid,
    cakeCommonXAxis,
    cakeHistoryWindow,
    cakeScatterSeries,
    cakeTooltip,
    formatCakeMillisecondsFromUs,
    CAKE_TIN_LEGEND_PAIRS,
    attachLegendPairMirror,
    cakeTinIndexFor,
} from "./cake_history";

export class CakeDelays extends DashboardGraph {
    constructor(id) {
        super(id);
        // `lqos-74t.53` (A): mirror a legend tin toggle to its paired `… Up` line.
        attachLegendPairMirror(this.chart, CAKE_TIN_LEGEND_PAIRS);

        this.option = {
            title: cakeChartTitle("Delays", "Milliseconds"),
            grid: cakeCommonGrid(),
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                // do NOT set selectedMode:false — it disables the legend + the mirror
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
                    formatter: (val) => formatCakeMillisecondsFromUs(val),
                },
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
            tooltip: cakeTooltip(formatCakeMillisecondsFromUs),
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

 // Bridge modea): the per-tin value is the MEASURED p99
        // queue sojourn (the bridge wire's `sojourn_p99_ns`), not Linux tc's
        // cobalt base delay — label it honestly. The kernel path keeps the
        // legacy "Delays" title (base_delay_us).
        this.option.title = cakeChartTitle(
            msg && msg.per_direction ? "Delay p99" : "Delays",
            "Milliseconds",
        );

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
                this.option.series[j].data.push(sample[0].tins[cakeTinIndexFor(j)].base_delay_us);
                this.option.series[j+4].data.push(0 - sample[1].tins[cakeTinIndexFor(j)].base_delay_us);
            }
        }

        this.chart.setOption(this.option);
    }
}
