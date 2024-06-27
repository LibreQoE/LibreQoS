import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export class CpuHistogram extends DashboardGraph {
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
                min: 0,
                max: 100,
            },
            series: {
                data: d,
                type: 'bar',
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(cpu) {
        this.chart.hideLoading();
        this.option.series.data = [];
        for (let i=0; i<cpu.length; i++) {
            this.option.series.data.push({
                value: cpu[i],
                itemStyle: {color: lerpGreenToRedViaOrange(100-cpu[i], 100)},
            });
        }
        this.chart.setOption(this.option);
    }
}

