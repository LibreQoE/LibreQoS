import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {RingBuffer} from "../lq_js_common/helpers/ringbuffer";

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