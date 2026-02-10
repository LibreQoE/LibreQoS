import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";

export class WindowedLatencyHistogram extends DashboardGraph {
    constructor(id, title = "Latency Histogram", windowMs = 300000) {
        super(id);
        this.windowMs = windowMs;
        this.samples = [];

        this.bucketSizeMs = 10;
        this.bucketCount = 20; // 0..200ms in 10ms buckets

        let d = [];
        let axis = [];
        for (let i = 0; i < this.bucketCount; i++) {
            d.push({
                value: 0,
                itemStyle: { color: lerpGreenToRedViaOrange(this.bucketCount - i, this.bucketCount) },
            });
            axis.push((i * this.bucketSizeMs).toString());
        }

        this.option = {
            title: {
                text: title,
            },
            xAxis: {
                type: 'category',
                data: axis,
                name: "ms",
                nameLocation: 'end',
            },
            yAxis: {
                type: 'value',
                name: "Samples",
            },
            series: {
                data: d,
                type: 'bar',
            },
        };

        this.option && this.chart.setOption(this.option);
        this.chart.hideLoading();
    }

    updateMs(pingOrRttMs) {
        const now = Date.now();
        const ms = toNumber(pingOrRttMs, 0);
        if (ms > 0) {
            this.samples.push({ t: now, ms: ms });
        }
        this.render(now);
    }

    updateManyMs(values) {
        const now = Date.now();
        if (Array.isArray(values)) {
            values.forEach((v) => {
                const ms = toNumber(v, 0);
                if (ms > 0) {
                    this.samples.push({ t: now, ms: ms });
                }
            });
        }
        this.render(now);
    }

    render(nowMs) {
        const cutoff = nowMs - this.windowMs;
        while (this.samples.length > 0 && this.samples[0].t < cutoff) {
            this.samples.shift();
        }

        const counts = new Array(this.bucketCount).fill(0);
        this.samples.forEach((s) => {
            let bucket = Math.floor(s.ms / this.bucketSizeMs);
            bucket = Math.min(bucket, this.bucketCount - 1);
            counts[bucket] += 1;
        });

        for (let i = 0; i < this.bucketCount; i++) {
            this.option.series.data[i].value = counts[i];
        }

        this.chart.setOption(this.option);
    }
}

