import {DashboardGraph} from "./dashboard_graph";

export class RttHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        let d = [];
        let axis = [];
        for (let i=0; i<20; i++) {
            d.push(i);
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
                type: 'bar'
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(rtt) {
        this.chart.hideLoading();
        this.option.series.data = rtt;
        this.chart.setOption(this.option);
    }
}