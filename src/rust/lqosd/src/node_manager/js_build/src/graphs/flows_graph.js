import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

export class FlowCountGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        let xaxis = [];
        for (let i=0; i<RING_SIZE; i++) {
            xaxis.push(i);
        }

        this.option = {
            grid: {
                x: '15%',
            },
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                data: [
                    {
                        name: "Active/Tracked",
                        icon: 'circle',
                    }, {
                        name: "Recently Completed",
                        icon: 'circle',
                    }
                ],
                textStyle: {
                    color: '#aaa'
                },
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
                    name: 'Active/Tracked',
                    data: [],
                    type: 'line',
                    symbol: 'none',
                },
                {
                    name: 'Recently Completed',
                    data: [],
                    type: 'line',
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

    update(recent, completed) {
        this.chart.hideLoading();
        this.ringbuffer.push(recent, completed);

        let series = this.ringbuffer.series();
        for (let i=0; i<2; i++) {
            this.option.series[i].data = series[i];
        }

        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0]);
        }
        this.head = 0;
        this.data = data;
    }

    push(recent, completed) {
        this.data[this.head] = [recent, completed];
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [[], []];
        for (let i=this.head; i<this.size; i++) {
            result[0].push(this.data[i][0]);
            result[1].push(this.data[i][1]);
        }
        for (let i=0; i<this.head; i++) {
            result[0].push(this.data[i][0]);
            result[1].push(this.data[i][1]);
        }
        return result;
    }
}