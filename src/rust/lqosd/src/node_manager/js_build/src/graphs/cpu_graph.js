import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";

export class CpuHistogram extends DashboardGraph {
    constructor(id) {
        super(id);
        let d = [];
        let axis = [];
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
        this.option.animation = false;
        this.option && this.chart.setOption(this.option);
    }

    update(cpu) {
        this.chart.hideLoading();
        const values = Array.isArray(cpu) ? cpu : [];
        const n = values.length;

        // Ensure axis and series arrays are the correct size and reuse object identities.
        if (!this.option.xAxis) this.option.xAxis = { type: 'category', data: [] };
        if (!Array.isArray(this.option.xAxis.data)) this.option.xAxis.data = [];
        const axis = this.option.xAxis.data;
        if (!this.option.series) this.option.series = { data: [], type: 'bar' };
        if (!Array.isArray(this.option.series.data)) this.option.series.data = [];
        const data = this.option.series.data;

        if (axis.length !== n) {
            axis.length = n;
            for (let i = 0; i < n; i++) {
                axis[i] = i.toString();
            }
        }

        if (data.length !== n) {
            data.length = n;
            for (let i = 0; i < n; i++) {
                if (!data[i]) {
                    data[i] = { value: 0, itemStyle: { color: lerpGreenToRedViaOrange(100, 100) } };
                }
            }
        }

        for (let i = 0; i < n; i++) {
            const v = Number(values[i] || 0);
            data[i].value = v;
            if (!data[i].itemStyle) data[i].itemStyle = {};
            data[i].itemStyle.color = lerpGreenToRedViaOrange(100 - v, 100);
        }

        // Replace instead of merge to avoid ECharts accumulating bar items over time.
        this.chart.setOption(this.option, true);
    }
}
