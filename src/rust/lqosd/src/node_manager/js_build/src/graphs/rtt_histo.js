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
    }

    update(rtt) {
        this.chart.hideLoading();
        for (let i=0; i<N_ITEMS; i++) {
            this.option.series.data[i].value = rtt[i];
        }
        this.chart.setOption(this.option);
    }
}

