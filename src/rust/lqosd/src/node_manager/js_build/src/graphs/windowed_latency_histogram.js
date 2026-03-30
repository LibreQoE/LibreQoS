import {DashboardGraph} from "./dashboard_graph";
import {lerpGreenToRedViaOrange} from "../helpers/scaling";
import {toNumber} from "../lq_js_common/helpers/scaling";
import {applyCircuitDeviceChartTheme} from "./circuit_device_chart_theme";

export class WindowedLatencyHistogram extends DashboardGraph {
    constructor(id, title = "Latency Histogram", windowMs = 300000) {
        super(id);
        if (this.dom && this.dom.classList) {
            this.dom.classList.remove("muted");
        }
        this.windowMs = windowMs;
        this.samples = [];
        this.sampleHead = 0;
        this.bucketCounts = [];

        this.bucketSizeMs = 10;
        this.bucketCount = 20; // 0..200ms in 10ms buckets

        let d = [];
        let axis = [];
        for (let i = 0; i < this.bucketCount; i++) {
            this.bucketCounts.push(0);
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

        applyCircuitDeviceChartTheme(this.option);
        this.option && this.chart.setOption(this.option);
        this.chart.hideLoading();
    }

    onThemeChange() {
        super.onThemeChange();
        applyCircuitDeviceChartTheme(this.option);
        this.chart.setOption(this.option);
    }

    updateMs(pingOrRttMs) {
        const now = Date.now();
        const ms = toNumber(pingOrRttMs, 0);
        if (ms > 0) {
            this.#pushSample(now, ms);
        }
        this.render(now);
    }

    updateManyMs(values) {
        const now = Date.now();
        if (Array.isArray(values)) {
            values.forEach((v) => {
                const ms = toNumber(v, 0);
                if (ms > 0) {
                    this.#pushSample(now, ms);
                }
            });
        }
        this.render(now);
    }

    render(nowMs) {
        const cutoff = nowMs - this.windowMs;
        while (
            this.sampleHead < this.samples.length &&
            this.samples[this.sampleHead].t < cutoff
        ) {
            const expired = this.samples[this.sampleHead];
            this.bucketCounts[expired.bucket] = Math.max(0, this.bucketCounts[expired.bucket] - 1);
            this.sampleHead++;
        }

        if (this.sampleHead > 0 && this.sampleHead >= this.samples.length / 2) {
            this.samples = this.samples.slice(this.sampleHead);
            this.sampleHead = 0;
        }

        for (let i = 0; i < this.bucketCount; i++) {
            this.option.series.data[i].value = this.bucketCounts[i];
        }

        this.chart.setOption(this.option);
    }

    #pushSample(now, ms) {
        let bucket = Math.floor(ms / this.bucketSizeMs);
        bucket = Math.min(bucket, this.bucketCount - 1);
        this.samples.push({ t: now, bucket });
        this.bucketCounts[bucket] += 1;
    }
}
