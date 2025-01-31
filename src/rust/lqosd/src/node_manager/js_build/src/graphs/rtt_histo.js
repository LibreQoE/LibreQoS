import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export const N_ITEMS = 50;

export class RttHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
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
                name: "% of Samples",
                nameLocation: 'middle',
                nameGap: 40,
                min: () => 0,
                max: () => 100,
            },
            series: {
                data: d,
                type: 'bar',
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(rtt) {
        this.chart.hideLoading();
        let sum = rtt.reduce((a, b) => a + b, 0);
        for (let i=0; i<N_ITEMS; i++) {
            this.option.series.data[i].value = (rtt[i] / sum) * 100;
        }
        this.chart.setOption(this.option);
    }
}

