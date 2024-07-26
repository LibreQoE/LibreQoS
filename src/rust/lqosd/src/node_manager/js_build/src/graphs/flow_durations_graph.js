import {DashboardGraph} from "./dashboard_graph";

export class FlowDurationsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            xAxis: {
                type: 'log',
                name: "Seconds"
            },
            yAxis: {
                type: 'value',
                name: "Samples"
            },
            series: {
                data: [],
                type: 'scatter',
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(data) {
        this.chart.hideLoading();

        let x = [];
        let y = [];

        data.forEach((r) => {
            x.push(r.duration);
            y.push(r.count);
        });

        this.option.xAxis.data = x;
        this.option.series.data = y;

        this.chart.setOption(this.option);
    }
}