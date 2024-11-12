import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber} from "../helpers/scaling";

export class PacketsPerSecondBar extends DashboardGraph {
    constructor(id) {
        super(id);
        this.option = {
            grid: {
                x: '15%',
            },
            xAxis: {
                type: 'value',
                axisLabel: {
                    formatter: (value) => { return scaleNumber(value, 0); }
                }
            },
            yAxis: {
                type: 'category',
                data: ['UP', 'DN'],
            },
            series: [
                {
                    type: 'bar',
                    data: [0, 0],
                }
            ]
        }
        this.option && this.chart.setOption(this.option);
    }

    update(down, up) {
        this.chart.hideLoading();
        this.option.series[0].data = [
            { value: up,  },
            { value: down, }
        ];
        this.chart.setOption(this.option);
    }
}