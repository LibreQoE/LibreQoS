import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";

export class LtsThroughputPeriodGraph extends DashboardGraph {
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
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                data: [
                    {
                        name: "Download",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }, {
                        name: "Upload",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[1]
                        }
                    }
                ],
                textStyle: {
                    color: '#aaa'
                },
            },
            series: [
                {
                    name: 'Download Error',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'dl',
                    lineStyle: {
                        opacity: 0
                    },
                },
                {
                    name: 'Download Error2',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'dl',
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: '#ccc'
                    },
                },
                {
                    name: 'Upload Error',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'ul',
                    lineStyle: {
                        opacity: 0
                    },
                },
                {
                    name: 'Upload Error2',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'ul',
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: '#ccc'
                    },
                },
                {
                    name: 'Download',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                },
                {
                    name: 'Upload',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                },
            ],
        };
        this.option && this.chart.setOption(this.option);
    }

    update(data) {
        this.chart.hideLoading();
        //console.log(data);
        this.option.xAxis.data = [];

        this.option.series[0].data = [];
        this.option.series[1].data = [];
        this.option.series[2].data = [];
        this.option.series[3].data = [];
        this.option.series[4].data = [];
        this.option.series[5].data = [];
        for (let x=0; x<data.length; x++) {
            const minDown = toNumber(data[x].min_down, 0);
            const maxDown = toNumber(data[x].max_down, 0);
            const minUp = toNumber(data[x].min_up, 0);
            const maxUp = toNumber(data[x].max_up, 0);
            const medianDown = toNumber(data[x].median_down, 0);
            const medianUp = toNumber(data[x].median_up, 0);

            this.option.xAxis.data.push(data[x].time);
            this.option.series[0].data.push(minDown * 8);
            this.option.series[1].data.push((maxDown - minDown) * 8);
            this.option.series[2].data.push((0.0 - maxUp) * 8);
            this.option.series[3].data.push((0.0 - (maxUp - minUp)) * 8);
            //console.log(0.0 - data[x].min_up, 0.0 - data[x].max_up);

            this.option.series[4].data.push(medianDown * 8);
            this.option.series[5].data.push((0.0 - medianUp) * 8);
        }
        this.chart.setOption(this.option);
    }
}
