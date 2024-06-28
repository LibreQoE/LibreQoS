import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export class RttHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        let d = [];
        let axis = [];
        for (let i=0; i<20; i++) {
            d.push({
                value: i,
                itemStyle: {color: lerpGreenToRedViaOrange(20-i, 20)},
            });
            axis.push(i.toString());
        }
        this.option = {
            xAxis: {
                type: 'category',
                data: axis,
            },
            yAxis: {
                type: 'value',
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
        for (let i=0; i<20; i++) {
            this.option.series.data[i].value = rtt[i];
        }
        this.chart.setOption(this.option);
    }
}
