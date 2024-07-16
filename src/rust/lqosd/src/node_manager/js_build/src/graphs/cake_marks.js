import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

export class CakeMarks extends DashboardGraph {
    constructor(id) {
        super(id);

        let xaxis = [];
        for (let i=0; i<600; i++) {
            xaxis.push(i);
        }

        this.option = {
            title: {
                text: "ECN Marks",
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
                            color: "gray"
                        }
                    }, {
                        name: "Best Effort",
                        icon: 'circle',
                        itemStyle: {
                            color: "green"
                        }
                    }, {
                        name: "RT Video",
                        icon: 'circle',
                        itemStyle: {
                            color: "orange"
                        }
                    }, {
                        name: "Voice",
                        icon: 'circle',
                        itemStyle: {
                            color: "yellow"
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
                    type: 'line',
                    lineStyle: {
                        color: 'gray',
                    },
                    symbol: 'none',
                },
                {
                    name: 'Best Effort',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'green',
                    },
                    symbol: 'none',
                },
                {
                    name: 'RT Video',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'orange',
                    },
                    symbol: 'none',
                },
                {
                    name: 'Voice',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'yellow',
                    },
                    symbol: 'none',
                },
                {
                    name: 'Bulk Up',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'gray',
                    },
                    symbol: 'none',
                },
                {
                    name: 'Best Effort Up',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'green',
                    },
                    symbol: 'none',
                },
                {
                    name: 'RT Video Up',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'orange',
                    },
                    symbol: 'none',
                },
                {
                    name: 'RT Voice Up',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'yellow',
                    },
                    symbol: 'none',
                },
            ],
            tooltip: {
                trigger: 'item',
            },
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
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
                this.option.series[j].data.push(msg.history[i][0].tins[j].marks);
                this.option.series[j+4].data.push(0 - msg.history[i][1].tins[j].marks);
            }
        }
        for (let i=0; i<msg.history_head; i++) {
            for (let j=0; j<4; j++) {
                if (msg.history[i][0].tins[0] === undefined) continue;
                this.option.series[j].data.push(msg.history[i][0].tins[j].marks);
                this.option.series[j+4].data.push(0 - msg.history[i][1].tins[j].marks);
            }
        }

        this.chart.setOption(this.option);
    }
}
