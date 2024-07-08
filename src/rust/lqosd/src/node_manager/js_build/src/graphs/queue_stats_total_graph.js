import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

export class QueueStatsTotalGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        let xaxis = [];
        for (let i=0; i<RING_SIZE; i++) {
            xaxis.push(i);
        }

        this.option = {
            legend: {
                orient: "horizontal",
                right: 10,
                top: "bottom",
                selectMode: false,
                data: [
                    {
                        name: "ECN Marks",
                        icon: 'circle',
                        itemStyle: {
                            color: "green"
                        }
                    }, {
                        name: "Cake Drops",
                        icon: 'circle',
                        itemStyle: {
                            color: "orange"
                        }
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
                    name: 'ECN Marks',
                    data: [],
                    type: 'line',
                    lineStyle: { color: "green" },
                    symbol: 'none',
                },
                {
                    name: 'ECN Marks Up',
                    data: [],
                    type: 'line',
                    lineStyle: { color: "green" },
                    symbol: 'none',
                },
                {
                    name: 'Cake Drops',
                    data: [],
                    type: 'line',
                    lineStyle: { color: "orange" },
                    symbol: 'none',
                },
                {
                    name: 'Cake Drops Up',
                    data: [],
                    type: 'line',
                    lineStyle: { color: "orange" },
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

    update(marks, drops) {
        this.chart.hideLoading();
        this.ringbuffer.push(marks, drops);

        let series = this.ringbuffer.series();
        for (let i=0; i<this.option.series.length; i++) {
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
            data.push({ marks: { down: 0, up: 0 }, drops: { down: 0, up: 0 } });
        }
        this.head = 0;
        this.data = data;
    }

    push(marks, drops) {
        this.data[this.head] = {
            marks: marks,
            drops: drops,
        };
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [[], [], [], []];
        for (let i=this.head; i<this.size; i++) {
            result[0].push(this.data[i].marks.down);
            result[1].push(0 - this.data[i].marks.up);
            result[2].push(this.data[i].drops.down);
            result[3].push(0 - this.data[i].drops.up);
        }
        for (let i=0; i<this.head; i++) {
            result[0].push(this.data[i].marks.down);
            result[1].push(0 - this.data[i].marks.up);
            result[2].push(this.data[i].drops.down);
            result[3].push(0 - this.data[i].drops.up);
        }
        return result;
    }
}