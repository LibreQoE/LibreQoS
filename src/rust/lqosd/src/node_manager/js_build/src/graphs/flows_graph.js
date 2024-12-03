import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../lq_js_common/helpers/scaling";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";

const RING_SIZE = 60 * 5; // 5 Minutes

export class FlowCountGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        this.option = new GraphOptionsBuilder()
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxis("Tracked Flows", 30)
            .withLeftGridSize("15%")
            .build();
        this.option.series = [
            {
                name: 'Active/Tracked',
                data: [],
                type: 'line',
                symbol: 'none',
            }
        ];

        this.option && this.chart.setOption(this.option);
    }

    update(recent, completed) {
        this.chart.hideLoading();
        this.ringbuffer.push(recent, completed);

        let series = this.ringbuffer.series();
        this.option.series[0].data = series[0];

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