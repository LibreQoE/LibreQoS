import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export class DevicePingHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        let d = [];
        let axis = [];
        for (let i=0; i<20; i++) {
            d.push({
                value: 0,
                itemStyle: {color: lerpGreenToRedViaOrange(20-i, 20)},
            });
            axis.push((i*10).toString());
        }
        this.option = {
            title: {
                text: "Ping Time Histogram",
            },
            xAxis: {
                type: 'category',
                data: axis,
                name: "Ping Time (ms)"
            },
            yAxis: {
                type: 'value',
                name: "Samples"
            },
            series: {
                data: d,
                type: 'bar',
            }
        };
        this.option && this.chart.setOption(this.option);
    }

    update(ping_time_nanos) {
        this.chart.hideLoading();
        let ping_time_ms = ping_time_nanos / 1000000;
        let bucket = Math.floor(ping_time_ms / 10);
        bucket = Math.min(bucket, 19);
        this.option.series.data[bucket].value += 1;
        this.chart.setOption(this.option);
    }
}

