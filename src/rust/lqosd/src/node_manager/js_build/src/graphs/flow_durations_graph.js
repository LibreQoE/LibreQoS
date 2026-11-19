import {DashboardGraph} from "./dashboard_graph";
import {t} from "../i18n/index.js";

export class FlowDurationsGraph extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            xAxis: {
                name: t("graph.axis.seconds"),
                type: 'value',
            },
            yAxis: {
                name: t("graph.axis.samples"),
                type: 'log',
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

        let points = [];

        data.forEach((r) => {
            points.push([r.duration, r.count]);
        });

        this.option.series.data = points;

        this.chart.setOption(this.option);
    }
}