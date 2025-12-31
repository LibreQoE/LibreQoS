import {DashboardGraph} from "./dashboard_graph";
import {GraphOptionsBuilder} from "../lq_js_common/e_charts/chart_builder";
import {periodNameToSeconds} from "../helpers/time_periods";
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

export class FlowCountGraphTimescale extends DashboardGraph {
    constructor(id, period) {
        super(id);

        this.option = new GraphOptionsBuilder()
            .withTimeAxis()
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

        let seconds = periodNameToSeconds(period);
        listenOnceForSeconds("LtsFlows", seconds, (msg) => {
            const data = msg && msg.data ? msg.data : [];
            data.forEach((d) => {
                this.option.xAxis.data.push(d.time);
                this.option.series[0].data.push(d.flow_count);
            });
            this.chart.setOption(this.option);
            this.chart.hideLoading();
        });
        wsClient.send({ LtsFlows: { seconds } });
    }
}
