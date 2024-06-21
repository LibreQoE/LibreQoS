import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../scaling";

const RING_SIZE = 60 * 5; // 5 Minutes

export class ThroughputRingBufferGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        let xaxis = [];
        for (let i=0; i<RING_SIZE; i++) {
            xaxis.push(i);
        }

        this.option = {
            xAxis: {
                type: 'category',
                data: xaxis,
            },
            yAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (val) => {
                        return scaleNumber(Math.abs(val));
                    },
                }
            },
            series: [
                {
                    name: 'shaped0',
                    data: [],
                    type: 'line',
                    stack: 'shaped',
                    lineStyle: {
                        opacity: 0,
                    },
                    symbol: 'none',
                },
                {
                    name: 'shaped1',
                    data: [],
                    type: 'line',
                    stack: 'shaped',
                    lineStyle: {
                        opacity: 0,
                    },
                    symbol: 'none',
                    areaStyle: {
                        color: 'green'
                    }
                },
                {
                    name: 'unshaped0',
                    data: [],
                    type: 'line',
                },
                {
                    name: 'unshaped1',
                    data: [],
                    type: 'line',
                },
            ],
            tooltip: {
                trigger: 'item',
            },
        }
        this.option && this.chart.setOption(this.option);
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

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push([0, 0, 0, 0]);
        }
        this.head = 0;
        this.data = data;
    }

    push(shaped, unshaped) {
        this.data[this.head][0] = 0.0 - shaped[0];
        this.data[this.head][1] = shaped[1];
        this.data[this.head][2] = 0.0 - unshaped[0];
        this.data[this.head][3] = unshaped[1];
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [
            [], [], [], []
        ];
        for (let i=this.head; i<this.size; i++) {
            for (let j=0; j<4; j++) {
                result[j].push(this.data[i][j]);
            }
        }
        for (let i=0; i<this.head; i++) {
            for (let j=0; j<4; j++) {
                result[j].push(this.data[i][j]);
            }
        }
        return result;
    }
}