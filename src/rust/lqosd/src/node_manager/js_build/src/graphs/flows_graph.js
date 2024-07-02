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
            series: {
                name: 'flows',
                data: [],
                type: 'line',
            },
            tooltip: {
                trigger: 'item',
            },
            animation: false,
        }
        this.option && this.chart.setOption(this.option);
    }

    update(shaped, unshaped) {
        this.chart.hideLoading();
        this.ringbuffer.push(shaped, unshaped);

        this.option.series.data = this.ringbuffer.series();

        this.chart.setOption(this.option);
    }
}

class RingBuffer {
    constructor(size) {
        this.size = size;
        let data = [];
        for (let i=0; i<size; i++) {
            data.push(0);
        }
        this.head = 0;
        this.data = data;
    }

    push(flows) {
        this.data[this.head] = flows;
        this.head += 1;
        this.head %= this.size;
    }

    series() {
        let result = [];
        for (let i=this.head; i<this.size; i++) {
            result.push(this.data[i]);
        }
        for (let i=0; i<this.head; i++) {
            result.push(this.data[i]);
        }
        return result;
    }
}