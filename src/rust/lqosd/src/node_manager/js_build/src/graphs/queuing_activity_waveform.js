import {DashboardGraph} from "./dashboard_graph";
import {scaleNumber, toNumber} from "../lq_js_common/helpers/scaling";

const WINDOW_MS = 30_000;
const MAX_SAMPLES = 90;
const RENDER_INTERVAL_MS = 100;
const DISPLAY_LAG_MS = 1_200;
const SAMPLE_INTERVAL_MS = 1_000;

function formatSecondsAgo(nowMs, valueMs) {
    const delta = Math.max(0, Math.round((nowMs - valueMs) / 1000));
    return delta === 0 ? "now" : `-${delta}s`;
}

function formatBps(value) {
    return `${scaleNumber(Math.abs(value), 1)}bps`;
}

function formatQoo(value) {
    const n = toNumber(value, NaN);
    if (!Number.isFinite(n)) {
        return "-";
    }
    return `${Math.round(Math.max(0, Math.min(100, n)))}`;
}

function getWaveformTheme() {
    const isDark = document.documentElement.getAttribute("data-bs-theme") !== "light";
    if (isDark) {
        return {
            axisText: "rgba(216, 226, 244, 0.62)",
            axisName: "rgba(216, 226, 244, 0.55)",
            axisLine: "rgba(216, 226, 244, 0.28)",
            axisTick: "rgba(216, 226, 244, 0.2)",
            splitLine: "rgba(216, 226, 244, 0.045)",
            throughputLine: "#32d3bd",
            throughputGlow: "rgba(50, 211, 189, 0.34)",
            throughputAreaTop: "rgba(50, 211, 189, 0.48)",
            throughputAreaMid: "rgba(42, 156, 145, 0.3)",
            throughputAreaBottom: "rgba(9, 20, 31, 0.03)",
            qooLine: "#8fb0ff",
            qooGlow: "rgba(143, 176, 255, 0.3)",
            qooAreaTop: "rgba(143, 176, 255, 0.18)",
            qooAreaBottom: "rgba(16, 22, 34, 0.01)",
            qooBandGood: "rgba(42, 168, 108, 0.08)",
            qooBandWarn: "rgba(228, 154, 44, 0.07)",
            qooBandPoor: "rgba(225, 84, 84, 0.08)",
            ceilingInactive: "rgba(255, 255, 255, 0.32)",
            ceilingInactiveGlow: "rgba(255, 255, 255, 0.12)",
            ceilingActive: "#ffffff",
            ceilingActiveGlow: "rgba(255, 255, 255, 0.48)",
        };
    }

    return {
        axisText: "rgba(54, 67, 86, 0.72)",
        axisName: "rgba(54, 67, 86, 0.62)",
        axisLine: "rgba(77, 94, 118, 0.28)",
        axisTick: "rgba(77, 94, 118, 0.18)",
        splitLine: "rgba(77, 94, 118, 0.08)",
        throughputLine: "#129a87",
        throughputGlow: "rgba(18, 154, 135, 0.18)",
        throughputAreaTop: "rgba(18, 154, 135, 0.26)",
        throughputAreaMid: "rgba(18, 154, 135, 0.14)",
        throughputAreaBottom: "rgba(255, 255, 255, 0.02)",
        qooLine: "#4f79d9",
        qooGlow: "rgba(79, 121, 217, 0.18)",
        qooAreaTop: "rgba(79, 121, 217, 0.1)",
        qooAreaBottom: "rgba(255, 255, 255, 0.01)",
        qooBandGood: "rgba(46, 163, 111, 0.06)",
        qooBandWarn: "rgba(214, 148, 40, 0.06)",
        qooBandPoor: "rgba(196, 82, 82, 0.06)",
        ceilingInactive: "rgba(110, 120, 134, 0.34)",
        ceilingInactiveGlow: "rgba(110, 120, 134, 0.08)",
        ceilingActive: "rgba(255, 255, 255, 0.96)",
        ceilingActiveGlow: "rgba(104, 118, 138, 0.26)",
    };
}

function directionalSeriesData(samples, direction, valueKey, windowStart, windowEnd) {
    if (!samples || samples.length === 0) {
        return [];
    }

    const points = [];
    let previous = null;
    let next = null;

    for (const sample of samples) {
        if (sample.timestamp < windowStart) {
            previous = sample;
            continue;
        }
        if (sample.timestamp > windowEnd) {
            next = sample;
            break;
        }
        points.push([
            sample.timestamp,
            toNumber(sample?.[valueKey]?.[direction], 0),
        ]);
    }

    if (previous) {
        points.unshift([
            windowStart,
            toNumber(previous?.[valueKey]?.[direction], 0),
        ]);
    }
    if (next) {
        points.push([
            Math.min(next.timestamp, windowEnd),
            toNumber(next?.[valueKey]?.[direction], 0),
        ]);
    } else if (points.length > 0) {
        points.push([
            windowEnd,
            points[points.length - 1][1],
        ]);
    }

    return points;
}

function scalarSeriesData(samples, valueKey, windowStart, windowEnd) {
    if (!samples || samples.length === 0) {
        return [];
    }

    const points = [];
    let previous = null;
    let next = null;

    for (const sample of samples) {
        if (sample.timestamp < windowStart) {
            previous = sample;
            continue;
        }
        if (sample.timestamp > windowEnd) {
            next = sample;
            break;
        }
        points.push([
            sample.timestamp,
            sample?.[valueKey] ?? null,
        ]);
    }

    if (previous) {
        points.unshift([
            windowStart,
            previous?.[valueKey] ?? null,
        ]);
    }
    if (next) {
        points.push([
            Math.min(next.timestamp, windowEnd),
            next?.[valueKey] ?? null,
        ]);
    } else if (points.length > 0) {
        points.push([
            windowEnd,
            points[points.length - 1][1],
        ]);
    }

    return points;
}

function ceilingSeriesData(samples, direction, windowStart, windowEnd) {
    const throughputData = directionalSeriesData(samples, direction, "throughputBps", windowStart, windowEnd);
    const ceilingData = directionalSeriesData(samples, direction, "ceilingBps", windowStart, windowEnd);

    const pointCount = Math.min(throughputData.length, ceilingData.length);
    const base = [];
    const active = [];
    for (let i = 0; i < pointCount; i++) {
        const timestamp = ceilingData[i][0];
        const nextTimestamp = i + 1 < pointCount ? ceilingData[i + 1][0] : windowEnd;
        const ceilingBps = toNumber(ceilingData[i][1], 0);
        const throughputBps = toNumber(throughputData[i][1], 0);
        const atCeiling = ceilingBps > 0 && throughputBps >= (ceilingBps * 0.95);

        if (base.length === 0) {
            base.push([timestamp, ceilingBps]);
            active.push([timestamp, atCeiling ? ceilingBps : null]);
        }

        base.push([nextTimestamp, ceilingBps]);
        active.push([nextTimestamp, atCeiling ? ceilingBps : null]);
    }
    return { base, active };
}

function qooMarkAreas(colors) {
    return [
        [
            { yAxis: 0, itemStyle: { color: colors.qooBandPoor } },
            { yAxis: 50 },
        ],
        [
            { yAxis: 50, itemStyle: { color: colors.qooBandWarn } },
            { yAxis: 80 },
        ],
        [
            { yAxis: 80, itemStyle: { color: colors.qooBandGood } },
            { yAxis: 100 },
        ],
    ];
}

export class QueuingActivityWaveform extends DashboardGraph {
    constructor(id) {
        super(id);
        if (this.dom && this.dom.classList) {
            this.dom.classList.remove("muted");
        }
        this.colors = getWaveformTheme();
        this.direction = "down";
        this.samples = [];
        this.sampleClockBase = null;
        this.lastSampleTimestamp = null;
        this.lastPruneTimestamp = 0;
        this.cachedSeries = {
            direction: this.direction,
        };
        this.renderNow = Date.now();
        this.renderRaf = null;
        this.lastRenderTimestamp = 0;

        this.option = {
            animation: false,
            animationDurationUpdate: 0,
            animationEasingUpdate: "linear",
            backgroundColor: "transparent",
            grid: [
                {
                    left: "9%",
                    right: "9%",
                    top: 12,
                    height: "45%",
                },
                {
                    left: "9%",
                    right: "9%",
                    top: "57%",
                    height: "22%",
                },
            ],
            legend: {
                show: false,
            },
            tooltip: {
                trigger: "axis",
                axisPointer: {
                    type: "line",
                },
                formatter: (params) => {
                    if (!params || params.length === 0) {
                        return "";
                    }
                    const when = params[0].value?.[0] || this.renderNow;
                    const lines = [`<div><b>${formatSecondsAgo(this.renderNow, when)}</b></div>`];
                    params.forEach((entry) => {
                        const y = Array.isArray(entry.value) ? entry.value[1] : entry.value;
                        const formatted = entry.seriesName === "Circuit QoO"
                            ? `${formatQoo(y)} / 100`
                            : formatBps(y);
                        lines.push(
                            `<div><span style="display:inline-block;margin-right:5px;border-radius:10px;width:9px;height:9px;background-color:${entry.color};"></span>${entry.seriesName}: <b>${formatted}</b></div>`,
                        );
                    });
                    return lines.join("");
                },
            },
            xAxis: [
                {
                    type: "value",
                    gridIndex: 0,
                    min: () => this.renderNow - WINDOW_MS,
                    max: () => this.renderNow,
                    axisLine: {
                        lineStyle: {
                            color: this.colors.axisLine,
                        },
                    },
                    axisTick: { show: false },
                    axisLabel: { show: false },
                    splitLine: { show: false },
                },
                {
                    type: "value",
                    gridIndex: 1,
                    min: () => this.renderNow - WINDOW_MS,
                    max: () => this.renderNow,
                    axisLine: {
                        lineStyle: {
                            color: this.colors.axisLine,
                        },
                    },
                    axisTick: {
                        lineStyle: {
                            color: this.colors.axisTick,
                        },
                    },
                    axisLabel: {
                        color: this.colors.axisText,
                        formatter: (value) => formatSecondsAgo(this.renderNow, value),
                    },
                    splitLine: { show: false },
                },
            ],
            yAxis: [
                {
                    type: "value",
                    gridIndex: 0,
                    name: "Throughput",
                    nameLocation: "middle",
                    nameGap: 52,
                    nameTextStyle: {
                        color: this.colors.axisName,
                    },
                    axisLine: {
                        lineStyle: {
                            color: this.colors.axisLine,
                        },
                    },
                    splitLine: {
                        lineStyle: {
                            color: this.colors.splitLine,
                        },
                    },
                    axisLabel: {
                        color: this.colors.axisText,
                        formatter: (val) => formatBps(val),
                    },
                },
                {
                    type: "value",
                    gridIndex: 1,
                    min: 0,
                    max: 100,
                    name: "Circuit QoO",
                    nameLocation: "middle",
                    nameGap: 44,
                    nameTextStyle: {
                        color: this.colors.axisName,
                    },
                    axisLine: {
                        lineStyle: {
                            color: this.colors.axisLine,
                        },
                    },
                    splitLine: {
                        lineStyle: {
                            color: this.colors.splitLine,
                        },
                    },
                    axisLabel: {
                        color: this.colors.axisText,
                        formatter: (val) => `${Math.round(val)}`,
                    },
                    axisTick: {
                        lineStyle: {
                            color: this.colors.axisTick,
                        },
                    },
                },
            ],
            series: [
                {
                    name: "Throughput",
                    type: "line",
                    xAxisIndex: 0,
                    yAxisIndex: 0,
                    showSymbol: false,
                    smooth: false,
                    lineStyle: {
                        width: 2.4,
                        color: this.colors.throughputLine,
                        shadowBlur: 10,
                        shadowColor: this.colors.throughputGlow,
                    },
                    areaStyle: {
                        color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
                            { offset: 0, color: this.colors.throughputAreaTop },
                            { offset: 0.45, color: this.colors.throughputAreaMid },
                            { offset: 1, color: this.colors.throughputAreaBottom },
                        ]),
                    },
                    data: [],
                },
                {
                    name: "Ceiling Base",
                    type: "line",
                    xAxisIndex: 0,
                    yAxisIndex: 0,
                    showSymbol: false,
                    silent: true,
                    smooth: false,
                    connectNulls: true,
                    step: "start",
                    lineStyle: {
                        width: 2.8,
                        color: this.colors.ceilingInactive,
                        shadowBlur: 14,
                        shadowColor: this.colors.ceilingInactiveGlow,
                    },
                    data: [],
                },
                {
                    name: "Ceiling Active",
                    type: "line",
                    xAxisIndex: 0,
                    yAxisIndex: 0,
                    showSymbol: false,
                    silent: true,
                    smooth: false,
                    connectNulls: false,
                    step: "start",
                    lineStyle: {
                        width: 2.8,
                        color: this.colors.ceilingActive,
                        shadowBlur: 14,
                        shadowColor: this.colors.ceilingActiveGlow,
                    },
                    data: [],
                },
                {
                    name: "Circuit QoO",
                    type: "line",
                    xAxisIndex: 1,
                    yAxisIndex: 1,
                    showSymbol: false,
                    connectNulls: false,
                    smooth: false,
                    lineStyle: {
                        width: 2.1,
                        color: this.colors.qooLine,
                        shadowBlur: 10,
                        shadowColor: this.colors.qooGlow,
                    },
                    areaStyle: {
                        color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
                            { offset: 0, color: this.colors.qooAreaTop },
                            { offset: 1, color: this.colors.qooAreaBottom },
                        ]),
                    },
                    markArea: {
                        silent: true,
                        itemStyle: {
                            opacity: 1,
                        },
                        data: qooMarkAreas(this.colors),
                    },
                    data: [],
                },
            ],
        };

        this.chart.hideLoading();
        this.chart.setOption(this.option);
        this.scheduleRenderLoop();
    }

    scheduleRenderLoop() {
        const step = (timestamp) => {
            if (!this.chart) {
                this.renderRaf = null;
                return;
            }
            if (!this.lastRenderTimestamp || (timestamp - this.lastRenderTimestamp) >= RENDER_INTERVAL_MS) {
                this.lastRenderTimestamp = timestamp;
                this.render();
            }
            this.renderRaf = window.requestAnimationFrame(step);
        };
        this.renderRaf = window.requestAnimationFrame(step);
    }

    quantizeTimestamp(arrivalTimestamp) {
        const arrival = toNumber(arrivalTimestamp, Date.now());
        if (this.sampleClockBase === null) {
            this.sampleClockBase = arrival;
            this.lastSampleTimestamp = arrival;
            return arrival;
        }

        let quantized = this.sampleClockBase +
            (Math.round((arrival - this.sampleClockBase) / SAMPLE_INTERVAL_MS) * SAMPLE_INTERVAL_MS);
        if (this.lastSampleTimestamp !== null && quantized <= this.lastSampleTimestamp) {
            quantized = this.lastSampleTimestamp + SAMPLE_INTERVAL_MS;
        }
        this.lastSampleTimestamp = quantized;
        return quantized;
    }

    setDirection(direction) {
        this.direction = direction === "up" ? "up" : "down";
        this.cachedSeries.direction = this.direction;
        this.rebuildCachedSeries();
        this.render();
    }

    pushSample(sample) {
        const normalized = {
            timestamp: this.quantizeTimestamp(sample.timestamp || Date.now()),
            throughputBps: {
                down: toNumber(sample.throughputBps?.down, 0),
                up: toNumber(sample.throughputBps?.up, 0),
            },
            ceilingBps: {
                down: toNumber(sample.ceilingBps?.down, 0),
                up: toNumber(sample.ceilingBps?.up, 0),
            },
            qooScore: (() => {
                const qoo = toNumber(sample.qooScore, NaN);
                return Number.isFinite(qoo) ? Math.max(0, Math.min(100, qoo)) : null;
            })(),
        };

        this.samples.push(normalized);
        if (this.samples.length > MAX_SAMPLES) {
            this.samples.shift();
        }
        this.pruneSamples(normalized.timestamp);
        this.rebuildCachedSeries();
    }

    pruneSamples(referenceTimestamp = Date.now()) {
        if (referenceTimestamp - this.lastPruneTimestamp < SAMPLE_INTERVAL_MS) {
            return;
        }
        this.lastPruneTimestamp = referenceTimestamp;
        const cutoff = referenceTimestamp - WINDOW_MS - (SAMPLE_INTERVAL_MS * 2);
        while (this.samples.length > 0 && this.samples[0].timestamp < cutoff) {
            this.samples.shift();
        }
    }

    rebuildCachedSeries() {
        if (!this.samples.length) {
            this.cachedSeries = {
                direction: this.direction,
            };
            return;
        }
        this.cachedSeries = {
            direction: this.direction,
        };
    }

    currentSeriesState() {
        const latest = this.samples.length > 0 ? this.samples[this.samples.length - 1] : null;
        if (!latest) {
            return {
                throughputBps: 0,
                ceilingBps: 0,
                atCeiling: false,
            };
        }
        const throughputBps = toNumber(latest.throughputBps[this.direction], 0);
        const ceilingBps = toNumber(latest.ceilingBps[this.direction], 0);
        return {
            throughputBps,
            ceilingBps,
            atCeiling: ceilingBps > 0 && throughputBps >= (ceilingBps * 0.95),
        };
    }

    render() {
        if (!this.chart) {
            return;
        }
        this.renderNow = Date.now();
        const displayNow = this.renderNow - DISPLAY_LAG_MS;
        const windowStart = displayNow - WINDOW_MS;
        const throughputData = directionalSeriesData(
            this.samples,
            this.direction,
            "throughputBps",
            windowStart,
            displayNow,
        );
        const qooData = scalarSeriesData(
            this.samples,
            "qooScore",
            windowStart,
            displayNow,
        );
        const ceilingSeries = ceilingSeriesData(
            this.samples,
            this.direction,
            windowStart,
            displayNow,
        );

        const patch = {
            xAxis: [
                {
                    min: windowStart,
                    max: displayNow,
                },
                {
                    min: windowStart,
                    max: displayNow,
                },
            ],
            series: [
                {
                    name: "Throughput",
                    data: throughputData,
                },
                {
                    name: "Ceiling Base",
                    data: ceilingSeries.base,
                    lineStyle: {
                        color: this.colors.ceilingInactive,
                        shadowColor: this.colors.ceilingInactiveGlow,
                    },
                },
                {
                    name: "Ceiling Active",
                    data: ceilingSeries.active,
                    lineStyle: {
                        color: this.colors.ceilingActive,
                        shadowColor: this.colors.ceilingActiveGlow,
                    },
                },
                {
                    name: "Circuit QoO",
                    data: qooData,
                },
            ],
        };

        this.chart.setOption(patch, false, true);
    }

    onThemeChange() {
        this.colors = getWaveformTheme();
        this.option.xAxis[0].axisLine.lineStyle.color = this.colors.axisLine;
        this.option.xAxis[1].axisLine.lineStyle.color = this.colors.axisLine;
        this.option.xAxis[1].axisTick.lineStyle.color = this.colors.axisTick;
        this.option.xAxis[1].axisLabel.color = this.colors.axisText;
        this.option.yAxis[0].nameTextStyle.color = this.colors.axisName;
        this.option.yAxis[0].axisLine.lineStyle.color = this.colors.axisLine;
        this.option.yAxis[0].splitLine.lineStyle.color = this.colors.splitLine;
        this.option.yAxis[0].axisLabel.color = this.colors.axisText;
        this.option.yAxis[1].nameTextStyle.color = this.colors.axisName;
        this.option.yAxis[1].axisLine.lineStyle.color = this.colors.axisLine;
        this.option.yAxis[1].axisTick.lineStyle.color = this.colors.axisTick;
        this.option.yAxis[1].splitLine.lineStyle.color = this.colors.splitLine;
        this.option.yAxis[1].axisLabel.color = this.colors.axisText;
        this.option.series[0].lineStyle.color = this.colors.throughputLine;
        this.option.series[0].lineStyle.shadowColor = this.colors.throughputGlow;
        this.option.series[0].areaStyle.color = new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: this.colors.throughputAreaTop },
            { offset: 0.45, color: this.colors.throughputAreaMid },
            { offset: 1, color: this.colors.throughputAreaBottom },
        ]);
        this.option.series[1].lineStyle.color = this.colors.ceilingInactive;
        this.option.series[1].lineStyle.shadowColor = this.colors.ceilingInactiveGlow;
        this.option.series[2].lineStyle.color = this.colors.ceilingActive;
        this.option.series[2].lineStyle.shadowColor = this.colors.ceilingActiveGlow;
        this.option.series[3].lineStyle.color = this.colors.qooLine;
        this.option.series[3].lineStyle.shadowColor = this.colors.qooGlow;
        this.option.series[3].areaStyle.color = new echarts.graphic.LinearGradient(0, 0, 0, 1, [
            { offset: 0, color: this.colors.qooAreaTop },
            { offset: 1, color: this.colors.qooAreaBottom },
        ]);
        this.option.series[3].markArea.data = qooMarkAreas(this.colors);
        this.chart.setOption(this.option);
        this.render();
    }

    dispose() {
        if (this.renderRaf) {
            window.cancelAnimationFrame(this.renderRaf);
            this.renderRaf = null;
        }
        if (this.chart) {
            this.chart.dispose();
        }
    }
}
