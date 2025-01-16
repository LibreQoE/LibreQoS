import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {periodNameToSeconds} from "../helpers/time_periods";

const RING_SIZE = 60 * 5; // 5 Minutes

export class ThroughputRingBufferGraphTimescale extends DashboardGraph {
    constructor(id, period) {
        super(id);

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxis("Throughput (bps)", 40)
            .build();

        this.option.legend = {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            selectMode: false,
            data: [
                {
                    name: "Shaped Traffic",
                    icon: 'circle',
                    itemStyle: {
                        color: window.graphPalette[0]
                    }
                }, {
                    name: "Unshaped Traffic",
                    icon: 'circle',
                    itemStyle: {
                        color: window.graphPalette[1]
                    }
                }
            ],
            textStyle: {
                color: '#aaa'
            },
        };
        this.option.series = [
            {
                name: 'shaped0',
                data: [],
                type: 'line',
                stack: 'shaped',
                lineStyle: {
                    opacity: 0,
                    color: window.graphPalette[0],
                },
                symbol: 'none',
                areaStyle: {
                    color: window.graphPalette[0]
                },
            },
            {
                name: 'Shaped Traffic',
                data: [],
                type: 'line',
                stack: 'shaped',
                lineStyle: {
                    opacity: 0,
                    color: window.graphPalette[0],
                },
                symbol: 'none',
                areaStyle: {
                    color: window.graphPalette[0]
                }

            },
            {
                name: 'unshaped0',
                data: [],
                type: 'line',
                lineStyle: {
                    color: window.graphPalette[1],
                },
                symbol: 'none',
            },
            {
                name: 'Unshaped Traffic',
                data: [],
                type: 'line',
                lineStyle: {
                    color: window.graphPalette[1],
                },
                symbol: 'none',
            },
        ];
        this.option && this.chart.setOption(this.option);

        let seconds = periodNameToSeconds(period);
        console.log("Requesting Insight History Data");
        $.get("local-api/ltsThroughput/" + seconds, (data) => {
            console.log("Received Insight History Data");
        });
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[1];
        this.option.series[0].lineStyle.color = window.graphPalette[0];
        this.option.series[0].areaStyle.color = window.graphPalette[0];
        this.option.series[1].lineStyle.color = window.graphPalette[0];
        this.option.series[1].areaStyle.color = window.graphPalette[0];
        this.option.series[2].lineStyle.color = window.graphPalette[1];
        this.option.series[3].lineStyle.color = window.graphPalette[1];

        this.chart.setOption(this.option);
    }

    update(shaped, unshaped) {
        this.chart.hideLoading();
        this.ringbuffer.push(shaped, unshaped);

        let data = this.ringbuffer.series();
        this.option.series[0].data = data[0];
        this.option.series[1].data = data[1];
        this.option.series[2].data = data[2];
        this.option.series[3].data = data[3];

        this.chart.setOption(this.option);
    }
}
