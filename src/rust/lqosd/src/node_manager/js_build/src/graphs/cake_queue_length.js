import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

export class CakeQueueLength extends DashboardGraph {
    constructor(id) {
        super(id);

        let xaxis = [];
        for (let i=0; i<600; i++) {
            xaxis.push(i);
        }

        this.option = {
            title: {
                text: "Queue Length",
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
                        name: "Queue Length",
                        icon: 'circle',
                        itemStyle: {
                            color: "orange"
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
            },
            series: [
                {
                    name: 'Queue Length',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'orange',
                    },
                    symbol: 'none',
                },
                {
                    name: 'Queue Length Up',
                    data: [],
                    type: 'line',
                    lineStyle: {
                        color: 'orange',
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

        this.option.series[0].data = [];
        this.option.series[1].data = [];

        //console.log(msg);
        for (let i=msg.history_head; i<600; i++) {
            this.option.series[0].data.push(msg.history[i][0].qlen);
            this.option.series[1].data.push(0 - msg.history[i][1].qlen);
        }
        for (let i=0; i<msg.history_head; i++) {
            this.option.series[0].data.push(msg.history[i][0].qlen);
            this.option.series[1].data.push(0 - msg.history[i][1].qlen);
        }

        this.chart.setOption(this.option);
    }
}
