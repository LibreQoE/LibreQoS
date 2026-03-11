import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";

export class CakeTraffic extends DashboardGraph {
    constructor(id) {
        super(id);

        let xaxis = [];
        for (let i=0; i<600; i++) {
            xaxis.push(i);
        }

        this.option = {
            title: {
                text: "Traffic",
            },
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
            xAxis: {
                type: 'category',
                data: xaxis,
            },
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => {
                        return scaleNumber(Math.abs(val), 0);
                    },
                }
            },
            series: [
                {
                    name: 'Bulk',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[0],
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[0] }
                },
                {
                    name: 'Best Effort',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[1]
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[1] }
                },
                {
                    name: 'RT Video',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[2]
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[2] }
                },
                {
                    name: 'Voice',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[3]
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[3] }
                },
                {
                    name: 'Bulk Up',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[0],
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[0] }
                },
                {
                    name: 'Best Effort Up',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[1]
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[1] }
                },
                {
                    name: 'RT Video Up',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[2]
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[2] }
                },
                {
                    name: 'RT Voice Up',
                    data: [],
                    type: 'scatter',
                    lineStyle: {
                        color: window.graphPalette[3]
                    },
                    symbol: 'circle',
                    symbolSize: 2,
                    itemStyle: { color: window.graphPalette[3] }
                },
            ],
            tooltip: {
                trigger: 'item',
            },
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
        //console.log(msg);
        for (let i=msg.history_head; i<600; i++) {
            for (let j=0; j<4; j++) {
                if (msg.history[i][0].tins[0] === undefined) continue;
                const downBits = toNumber(msg.history[i][0].tins[j].sent_bytes, 0) * 8;
                const upBits = toNumber(msg.history[i][1].tins[j].sent_bytes, 0) * 8;
                this.option.series[j].data.push(downBits);
                this.option.series[j+4].data.push(0 - upBits);
            }
        }
        for (let i=0; i<msg.history_head; i++) {
            for (let j=0; j<4; j++) {
                if (msg.history[i][0].tins[0] === undefined) continue;
                const downBits = toNumber(msg.history[i][0].tins[j].sent_bytes, 0) * 8;
                const upBits = toNumber(msg.history[i][1].tins[j].sent_bytes, 0) * 8;
                this.option.series[j].data.push(downBits);
                this.option.series[j+4].data.push(0 - upBits);
            }
        }

        this.chart.setOption(this.option);
    }
}
