import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {periodNameToSeconds} from "../helpers/time_periods";
import {get_ws_client} from "../pubsub/ws";

export const N_ITEMS = 50;
const wsClient = get_ws_client();

const listenOnceForSeconds = (eventName, seconds, handler) => {
    const wrapped = (msg) => {
        if (!msg || msg.seconds !== seconds) return;
        wsClient.off(eventName, wrapped);
        handler(msg);
    };
    wsClient.on(eventName, wrapped);
};

export class RttHistogramTimeseries extends DashboardGraph {
    constructor(id, period) {
        super(id);
        this.period = period;

        let d = [];
        let axis = [];
        for (let i=0; i<N_ITEMS; i++) {
            d.push({
                value: i,
                itemStyle: {color: lerpGreenToRedViaOrange(N_ITEMS-i, N_ITEMS)},
            });
            axis.push((i*10).toString());
        }
        this.option = {
            xAxis: {
                type: 'category',
                data: axis,
                name: "RTT",
                nameLocation: 'middle',
                nameGap: 40,
            },
            yAxis: {
                type: 'value',
                name: "Samples",
                nameLocation: 'middle',
                nameGap: 40,
            },
            series: {
                data: d,
                type: 'bar',
            }
        };
        this.option && this.chart.setOption(this.option);

        let seconds = periodNameToSeconds(period);
        console.log("Requesting Insight History Data (RTT Histo)");
        listenOnceForSeconds("LtsRttHisto", seconds, (msg) => {
            const rtt = msg && msg.data ? msg.data : [];
            console.log(rtt);
            this.option.series.data = [];
            for (let i=0; i<rtt.length; i++) {
                this.option.series.data.push(rtt[i].value);
            }
            this.chart.setOption(this.option);
            this.chart.hideLoading();
        });
        wsClient.send({ LtsRttHisto: { seconds } });
    }

    update(rtt) {
        this.chart.hideLoading();
        for (let i=0; i<N_ITEMS; i++) {
            this.option.series.data[i].value = rtt[i];
        }
        this.chart.setOption(this.option);
    }
}
