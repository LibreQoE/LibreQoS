import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export class CpuHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            xAxis: {
                type: 'category',
                data: [],
            },
            yAxis: {
                type: 'value',
                min: 0,
                max: 100,
            },
            series: {
                data: [],
                type: 'bar',
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(cpu) {
        this.chart.hideLoading();
        const usage = Array.isArray(cpu) ? cpu : [];
        this.option.xAxis.data = usage.map((_, index) => index.toString());
        this.option.series.data = usage.map((value) => ({
            value,
            itemStyle: {color: lerpGreenToRedViaOrange(100 - value, 100)},
        }));
        this.chart.setOption(this.option);
    }
}
