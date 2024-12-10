import {DashboardGraph} from "./dashboard_graph";
import {RingBuffer} from "../lq_js_common/helpers/ringbuffer";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";

const RING_SIZE = 60 * 5; // 5 Minutes

export class RetransmitsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.ringbuffer = new RingBuffer(RING_SIZE);

        this.option = new GraphOptionsBuilder()
            .withLeftGridSize("15%")
            .withSequenceAxis(0, RING_SIZE)
            .withScaledAbsYAxisPercent("Retransmits", 40)
            .build();

        this.option.legend = {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            selectMode: false,
            data: [
                {
                    name: "Download",
                    icon: 'circle',
                },
                {
                    name: "Upload",
                    icon: 'circle',
                },
            ],
            textStyle: {
                color: '#aaa'
            },
        };
        this.option.series = [
            {
                name: 'Download',
                data: [],
                type: 'line',
                symbol: 'none',
            },
            {
                name: 'Upload',
                data: [],
                type: 'line',
                symbol: 'none',
            },
        ];
        this.option && this.chart.setOption(this.option);
    }

    update(down, up, tcp_down, tcp_up) {
        if (tcp_down === 0) {
            tcp_down = 1;
        }
        if (tcp_up === 0) {
            tcp_up = 1;
        }
        up = (up / tcp_up) * 100.0; // Percentage
        down = (down / tcp_down) * 100.0; // Percentage
        this.chart.hideLoading();
        this.ringbuffer.push(down, 0 - up);

        let series = this.ringbuffer.series();
        this.option.series[0].data = series[0];
        this.option.series[1].data = series[1];

        this.chart.setOption(this.option);
    }
}