import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {periodNameToSeconds} from "../helpers/time_periods";
import {MinMaxSeries} from "../lq_js_common/e_charts/min_max_median_series";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {get_ws_client} from "../pubsub/ws";

const RING_SIZE = 60 * 5; // 5 Minutes
const wsClient = get_ws_client();

const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class ThroughputRingBufferGraphTimescale extends DashboardGraph {
    constructor(id, period) {
        super(id);

        this.option = new GraphOptionsBuilder()
            .withTimeAxis()
            .withScaledAbsYAxis("Throughput (bps)", 40)
            .withEmptySeries()
            .withEmptyLegend()
            .build();

        this.option && this.chart.setOption(this.option);

        let seconds = periodNameToSeconds(period);
        console.log("Requesting Insight History Data");
        listenOnceForSeconds("LtsThroughput", seconds, (msg) => {
            const data = msg && msg.data ? msg.data : [];
            console.log("Received Insight History Data", data);
            let shaperDown = new MinMaxSeries("Down", 1);
            let shaperUp = new MinMaxSeries(" Up", 1);
            data.forEach((r) => {
                this.option.xAxis.data.push(r.time);
                shaperDown.pushPositive(
                    toNumber(r.median_down, 0) * 8,
                    toNumber(r.min_down, 0) * 8,
                    toNumber(r.max_down, 0) * 8
                );
                shaperUp.pushNegative(
                    toNumber(r.median_up, 0) * 8,
                    toNumber(r.min_up, 0) * 8,
                    toNumber(r.max_up, 0) * 8
                );
            });
            shaperDown.addToOptions(this.option);
            shaperUp.addToOptions(this.option);
            this.chart.setOption(this.option);
            this.chart.hideLoading();
        });
        wsClient.send({ LtsThroughput: { seconds } });
    }

    onThemeChange() {
        super.onThemeChange();
        this.option.legend.data[0].itemStyle.color = window.graphPalette[0];
        this.option.legend.data[1].itemStyle.color = window.graphPalette[1];
        this.option.series[0].lineStyle.color = window.graphPalette[0];
        this.option.series[0].areaStyle.color = window.graphPalette[0];
        this.option.series[1].lineStyle.color = window.graphPalette[0];
        this.option.series[1].areaStyle.color = window.graphPalette[0];
        this.option.series[2].lineStyle.color = window.graphPalette[1];
        this.option.series[3].lineStyle.color = window.graphPalette[1];

        this.chart.setOption(this.option);
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
