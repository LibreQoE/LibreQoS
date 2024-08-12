import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

export class LtsCakeGraph extends DashboardGraph {
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
                        name: "Marks",
                        icon: 'circle',
                        itemStyle: {
                            color: window.graphPalette[0]
                        }
                    }, {
                        name: "Drops",
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
                    name: 'DownloadM Error',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'dl',
                    lineStyle: {
                        opacity: 0
                    },
                },
                {
                    name: 'DownloadM Error2',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'dl',
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: window.graphPalette[2]
                    },
                },
                {
                    name: 'UploadM Error',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'ul',
                    lineStyle: {
                        opacity: 0
                    },
                },
                {
                    name: 'UploadM Error2',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'ul',
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: window.graphPalette[2]
                    },
                },
                {
                    name: 'Marks',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    lineStyle: {
                        color: window.graphPalette[0],
                    }
                },
                {
                    name: 'MarksU',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    lineStyle: {
                        color: window.graphPalette[0],
                    }
                },

                // Drops
                {
                    name: 'DownloadD Error',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'dl',
                    lineStyle: {
                        opacity: 0
                    },
                },
                {
                    name: 'DownloadD Error2',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'dl',
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: window.graphPalette[3]
                    },
                },
                {
                    name: 'UploadD Error',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'ul',
                    lineStyle: {
                        opacity: 0
                    },
                },
                {
                    name: 'UploadD Error2',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    stack: 'ul',
                    lineStyle: {
                        opacity: 0
                    },
                    areaStyle: {
                        color: window.graphPalette[3]
                    },
                },
                {
                    name: 'Drops',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    lineStyle: {
                        color: window.graphPalette[1],
                    }
                },
                {
                    name: 'DropsU',
                    type: 'line',
                    data: [],
                    symbol: 'none',
                    lineStyle: {
                        color: window.graphPalette[1],
                    }
                },
            ],
        };
        this.option && this.chart.setOption(this.option);
    }

    update(data) {
        this.chart.hideLoading();
        //console.log(data);
        this.option.xAxis.data = [];

        for (let i=0; i<12; i++) {
            this.option.series[i].data = [];
        }
        // this.option.series[0].data = [];
        // this.option.series[1].data = [];
        // this.option.series[2].data = [];
        // this.option.series[3].data = [];
        // this.option.series[4].data = [];
        // this.option.series[5].data = [];
        for (let x=0; x<data.length; x++) {
            this.option.xAxis.data.push(data[x].time);

            // Marks
            this.option.series[0].data.push(data[x].min_marks_down);
            this.option.series[1].data.push(data[x].max_marks_down - data[x].min_marks_down);
            this.option.series[2].data.push(0.0 - data[x].max_marks_up);
            this.option.series[3].data.push((0.0 - data[x].max_marks_up - data[x].min_marks_up));
            this.option.series[4].data.push(data[x].median_marks_down);
            this.option.series[5].data.push((0.0 - data[x].median_marks_up));

            // Drops
            this.option.series[6].data.push(data[x].min_drops_down);
            this.option.series[7].data.push(data[x].max_drops_down - data[x].min_drops_down);
            this.option.series[8].data.push(0.0 - data[x].max_drops_up);
            this.option.series[9].data.push(0.0 - (data[x].max_drops_up - data[x].min_drops_up));
            this.option.series[10].data.push(data[x].median_drops_down);
            this.option.series[11].data.push(0.0 - data[x].median_drops_up);
        }
        this.chart.setOption(this.option);
    }
}