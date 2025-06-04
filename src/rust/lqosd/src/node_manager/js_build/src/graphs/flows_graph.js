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
                encode: { x: 'timestamp', y: 'value' }
            }
        ];

        // Enable axisPointer and custom tooltip
        this.option.tooltip = {
            trigger: 'axis',
            axisPointer: {
                type: 'cross',
                link: [{ xAxisIndex: 'all' }],
                label: {
                    backgroundColor: '#6a7985'
                }
            },
            formatter: function(params) {
                // params is an array of series data
                if (!params || !params.length) return '';
                const p = params[0];
                const val = p.data.value;
                const ts = p.data.timestamp;
                const date = new Date(ts);
                const hh = String(date.getHours()).padStart(2, '0');
                const mm = String(date.getMinutes()).padStart(2, '0');
                const ss = String(date.getSeconds()).padStart(2, '0');
                return `Value: <b>${val}</b><br/>Time: <b>${hh}:${mm}:${ss}</b>`;
            }
        };

        this.option && this.chart.setOption(this.option);
    }

    update(recent, completed) {
        this.chart.hideLoading();
        this.ringbuffer.push(recent, completed);

        let series = this.ringbuffer.series();
        // ECharts expects array of {timestamp, value}
        this.option.series[0].data = series[0];

        this.chart.setOption(this.option);
    }
}