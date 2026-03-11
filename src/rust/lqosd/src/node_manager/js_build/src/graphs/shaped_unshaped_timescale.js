import {DashboardGraph} from "./dashboard_graph";
import {periodNameToSeconds} from "../helpers/time_periods";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {get_ws_client} from "../pubsub/ws";

const wsClient = get_ws_client();

const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class ShapedUnshapedTimescale extends DashboardGraph {
    constructor(id, period) {
        super(id);

        // Graph Options
        this.option = new GraphOptionsBuilder()
            .withTimeAxis()
            .withScaledAbsYAxis("% Mapped", 40)
            .withEmptySeries()
            .withEmptyLegend()
            .build();
        this.option.series.push({
            name: "% Mapped",
            type: "line",
            data: []
        });

        this.option && this.chart.setOption(this.option);

        // Request
        let seconds = periodNameToSeconds(period);
        listenOnceForSeconds("LtsPercentShaped", seconds, (msg) => {
            const data = msg && msg.data ? msg.data : [];
            console.log(data);

            // Add data to graph
            let percents = [];
            data.forEach((r) => {
                this.option.xAxis.data.push(r.time);
                percents.push(r.percent_shaped);
            });
            this.option.series[0].data = percents;
            this.chart.setOption(this.option);
            this.chart.hideLoading();
        });
        wsClient.send({ LtsPercentShaped: { seconds } });
    }
}
