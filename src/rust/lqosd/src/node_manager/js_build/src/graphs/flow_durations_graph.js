import {DashboardGraph} from "./dashboard_graph";

export class FlowDurationsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            xAxis: {
                name: "Seconds",
                type: 'value',
            },
            yAxis: {
                name: "Samples",
                type: 'log',
            },
            series: {
                data: [],
                type: 'scatter',
            }
        };
        this.option.animation = false;
        this.option && this.chart.setOption(this.option);
    }

    update(data) {
        this.chart.hideLoading();

        let points = [];

        data.forEach((r) => {
            points.push([r.duration, r.count]);
        });

        this.option.series.data = points;

        // Replace instead of merge to avoid ECharts retaining stale point clouds.
        this.chart.setOption(this.option, true);
    }
}
