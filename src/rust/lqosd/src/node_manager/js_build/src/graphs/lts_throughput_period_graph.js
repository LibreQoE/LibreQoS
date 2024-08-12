import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

export class LtsThroughputPeriodGraph extends DashboardGraph {
    constructor(id, period) {
        super(id);
        this.period = period;
        this.option = {
            xAxis: {
                type: 'category',
                data: [],
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
            this.option.xAxis.data.push(x);
            this.option.series[0].data.push(data[x].min_down * 8);
            this.option.series[1].data.push((data[x].max_down - data[x].min_down) * 8);
            this.option.series[2].data.push((0.0 - data[x].max_up) * 8);
            this.option.series[3].data.push((0.0 - (data[x].max_up - data[x].min_up)) * 8);
            //console.log(0.0 - data[x].min_up, 0.0 - data[x].max_up);

            this.option.series[4].data.push(data[x].median_down * 8);
            this.option.series[5].data.push((0.0 - data[x].median_up) * 8);
        }
        this.chart.setOption(this.option);
    }
}