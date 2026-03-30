import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {applyCircuitDeviceChartTheme} from "./circuit_device_chart_theme";

export class DevicePingHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        if (this.dom && this.dom.classList) {
            this.dom.classList.remove("muted");
        }
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
        applyCircuitDeviceChartTheme(this.option);
        this.option && this.chart.setOption(this.option);
        this.chart.hideLoading();
    }

    onThemeChange() {
        super.onThemeChange();
        applyCircuitDeviceChartTheme(this.option);
        this.chart.setOption(this.option);
    }

    update(ping_time_nanos) {
        this.chart.hideLoading();
        let ping_time_ms = toNumber(ping_time_nanos, 0) / 1000000;
        let bucket = Math.floor(ping_time_ms / 10);
        bucket = Math.min(bucket, 19);
        this.option.series.data[bucket].value += 1;
        this.chart.setOption(this.option);
    }

    updateMs(ping_time_ms) {
        this.chart.hideLoading();
        ping_time_ms = toNumber(ping_time_ms, 0);
        let bucket = Math.floor(ping_time_ms / 10);
        bucket = Math.min(bucket, 19);
        this.option.series.data[bucket].value += 1;
        this.chart.setOption(this.option);
    }
}
