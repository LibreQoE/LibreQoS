import {DashboardGraph} from "./dashboard_graph";
import {
    CAKE_CHART_WINDOW_SECONDS,
    cakeChartTitle,
    cakeCommonGrid,
    cakeCommonXAxis,
    cakeHistoryWindow,
    cakeScatterSeries,
    cakeTooltip,
    cakePerDirectionOption,
    formatCakePackets,
    CAKE_TIN_LEGEND_PAIRS,
    attachLegendPairMirror,
    cakeTinIndexFor,
} from "./cake_history";

export class CakeDrops extends DashboardGraph {
    constructor(id) {
        super(id);
        // `lqos-74t.53` (A): mirror a legend tin toggle to its paired `… Up` line.
        attachLegendPairMirror(this.chart, CAKE_TIN_LEGEND_PAIRS);

        this.option = {
            title: cakeChartTitle("Shaper Drops", "Packets per sample"),
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
                    formatter: (val) => formatCakePackets(val),
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
            tooltip: cakeTooltip(formatCakePackets),
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
    }

    onThemeChange() {
        super.onThemeChange();
        const palette = window.graphPalette;
        this.option.legend.data.forEach((entry, i) => {
            entry.itemStyle.color = palette[i % palette.length];
        });
        this.option.series.forEach((series, i) => {
            series.itemStyle.color = palette[i % palette.length];
        });
        this.chart.setOption(this.option);
    }

    update(msg) {
        this.chart.hideLoading();

        if (msg && msg.per_direction) {
            this.option = cakePerDirectionOption(msg, "Shaper Drops (per-direction)", "drops", formatCakePackets);
            this.chart.setOption(this.option, true);
            return;
        }

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
                this.option.series[j].data.push(sample[0].tins[cakeTinIndexFor(j)].drops);
                this.option.series[j+4].data.push(0 - sample[1].tins[cakeTinIndexFor(j)].drops);
            }
        }

        this.chart.setOption(this.option);
    }
}
