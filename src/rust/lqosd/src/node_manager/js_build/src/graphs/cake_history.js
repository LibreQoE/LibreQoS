import {scaleNumber, toNumber, trimTrailingZeros} from "../lq_js_common/helpers/scaling";
import {CAKE_TIN_LEGEND_PAIRS, cakeTinIndexFor} from "./cake_tin_order.mjs";

export const CAKE_CHART_WINDOW_SECONDS = 180;
export const CAKE_CHART_WINDOW_LABEL = "Last 3 minutes · 1s samples";

export function cakeChartXAxis(windowSize = CAKE_CHART_WINDOW_SECONDS) {
    const labels = new Array(windowSize).fill("");
    const wholeMinutes = Math.floor(windowSize / 60);

    for (let minute = wholeMinutes; minute >= 1; minute--) {
        const index = windowSize - (minute * 60);
        if (index >= 0 && index < windowSize) {
            labels[index] = `-${minute}m`;
        }
    }

    if (windowSize > 0) {
        if (labels[0] === "") {
            labels[0] = `-${Math.ceil(windowSize / 60)}m`;
        }
        labels[windowSize - 1] = "Now";
    }

    return labels;
}

export function cakeChartTitle(text, unitsLabel) {
    return {
        text,
        subtext: `${CAKE_CHART_WINDOW_LABEL} · ${unitsLabel}`,
        left: 10,
        top: 10,
        textStyle: {
            fontSize: 14,
            fontWeight: 700,
        },
        subtextStyle: {
            fontSize: 11,
            color: "#8c939d",
        },
    };
}

export function cakeCommonGrid() {
    return {
        left: 58,
        right: 16,
        top: 62,
        bottom: 46,
        containLabel: true,
    };
}

export function cakeCommonXAxis(windowSize = CAKE_CHART_WINDOW_SECONDS) {
    return {
        type: "category",
        data: cakeChartXAxis(windowSize),
        boundaryGap: false,
        axisTick: {
            show: false,
        },
        axisLabel: {
            color: "#8c939d",
            fontSize: 10,
            interval: 0,
        },
        axisLine: {
            lineStyle: {
                color: "rgba(148, 163, 184, 0.28)",
            },
        },
        splitLine: {
            show: false,
        },
        axisPointer: {
            show: true,
            label: {
                show: false,
            },
        },
    };
}

export function cakeScatterSeries(name, color) {
    return {
        name,
        data: [],
        type: "scatter",
        symbol: "circle",
        symbolSize: 4,
        itemStyle: {
            color,
            opacity: 0.72,
        },
        emphasis: {
            scale: 1.8,
            itemStyle: {
                color,
                opacity: 0.98,
                borderColor: "rgba(255, 255, 255, 0.92)",
                borderWidth: 1,
            },
        },
    };
}

// `lqos-74t.53`: the tin wire-order convention + legend pairs live in
// `cake_tin_order.mjs` (so the pure node test can import them); re-exported here
// for the chart modules.
export {CAKE_TIN_LEGEND_PAIRS, cakeTinIndexFor};

/**
 * `lqos-74t.53` (A): ECharts toggles a legend item by SERIES NAME, but every tin
 * has two series (its down line + the paired `… Up` line). Mirror the selection
 * so a legend click hides/restores BOTH. `pairs` is explicit (the Voice up series
 * is `"RT Voice Up"`); attach ONCE per chart (constructor), not per `setOption`.
 */
export function attachLegendPairMirror(chart, pairs) {
    if (!chart?.on || !Array.isArray(pairs) || pairs.length === 0) {
        return;
    }
    chart.on("legendselectchanged", (params) => {
        const name = params?.name;
        if (typeof name !== "string") {
            return;
        }
        const pair = pairs.find((entry) => entry.down === name || entry.up === name);
        if (!pair) {
            return;
        }
        const other = pair.down === name ? pair.up : pair.down;
        const selected = params?.selected || {};
        const want = selected[name] !== false;
        if (selected[other] === want) {
            return; // already in sync (or a redundant feedback event)
        }
        chart.dispatchAction({ type: want ? "legendSelect" : "legendUnSelect", name: other });
    });
}

export function cakeTooltip(unitFormatter) {
    return {
        trigger: "axis",
        axisPointer: {
            type: "line",
            snap: false,
        },
        confine: true,
        formatter: (params) => {
            const rows = Array.isArray(params) ? params : [params];
            if (rows.length === 0) {
                return "";
            }
            const heading = rows[0]?.axisValueLabel || "Selected sample";
            const lines = [
                `<div style="margin-bottom:4px;color:#8c939d;font-size:11px;">${heading}</div>`,
            ];

            rows.forEach((entry) => {
                const raw = Array.isArray(entry.value) ? entry.value[1] : entry.value;
                if (raw === null || raw === undefined || Number.isNaN(Number(raw))) {
                    return;
                }
                const isUpload = entry.seriesName.endsWith(" Up");
                const label = isUpload
                    ? entry.seriesName.slice(0, -3)
                    : entry.seriesName;
                const direction = isUpload ? "Upload" : "Download";
                lines.push(
                    `${entry.marker}${label} <span style="color:#8c939d">(${direction})</span>: <strong>${unitFormatter(Math.abs(Number(raw)))}</strong>`,
                );
            });

            return lines.join("<br>");
        },
    };
}

export function formatCakeBytes(value) {
    return `${scaleNumber(Math.abs(toNumber(value, 0)), 1)}B`;
}

export function formatCakeBitsPerSecond(value) {
    return `${scaleNumber(Math.abs(toNumber(value, 0)), 1)}bps`;
}

export function formatCakePackets(value) {
    return `${scaleNumber(Math.abs(toNumber(value, 0)), 0)} pkts`;
}

export function formatCakeMillisecondsFromUs(value) {
    const ms = Math.abs(toNumber(value, 0)) / 1000;
    if (ms >= 100) {
        return `${Math.round(ms)}ms`;
    }
    if (ms >= 10) {
        return `${trimTrailingZeros(ms.toFixed(1))}ms`;
    }
    return `${trimTrailingZeros(ms.toFixed(2))}ms`;
}

/**
 * Bridge mode (K2 slice 2 §7 /a): build a full per-direction
 * aggregate option for the marks/drops graphs. The bridge wire has no
 * per-tin drops/marks, so exactly one Down + one Up series are rendered from
 * `CakeDiffTransit.drops`/`.marks`, with the honest "per-direction" label.
 * Returns a complete option (title/grid/axes/series/tooltip) so the caller
 * can swap it in with `setOption(option, true)` and drop the per-tin series.
 */
export function cakePerDirectionOption(msg, titleText, field, unitFormatter) {
    const down = [];
    const up = [];
    for (const sample of cakeHistoryWindow(msg, CAKE_CHART_WINDOW_SECONDS)) {
        if (!sample || !sample[0] || !sample[1]) {
            down.push(null);
            up.push(null);
            continue;
        }
        down.push(sample[0][field] ?? 0);
        up.push(0 - (sample[1][field] ?? 0));
    }
    return {
        title: cakeChartTitle(titleText, "Packets per sample"),
        grid: cakeCommonGrid(),
        legend: {
            orient: "horizontal",
            right: 10,
            top: "bottom",
            // do NOT set selectedMode:false — it disables the legend
            textStyle: {
                color: '#aaa'
            },
            data: [
                {
                    name: "Down",
                    icon: 'circle',
                    itemStyle: { color: window.graphPalette[0] }
                }, {
                    name: "Up",
                    icon: 'circle',
                    itemStyle: { color: window.graphPalette[1] }
                }
            ]
        },
        xAxis: cakeCommonXAxis(CAKE_CHART_WINDOW_SECONDS),
        yAxis: {
            type: 'value',
            axisLabel: {
                formatter: (val) => unitFormatter(val),
            }
        },
        series: [
            { ...cakeScatterSeries("Down", window.graphPalette[0]), data: down },
            { ...cakeScatterSeries("Up", window.graphPalette[1]), data: up },
        ],
        tooltip: cakeTooltip(unitFormatter),
        animation: false,
    };
}

export function cakeHistoryWindow(msg, windowSize = CAKE_CHART_WINDOW_SECONDS) {
    if (!msg || !Array.isArray(msg.history) || msg.history.length === 0) {
        return new Array(windowSize).fill(null);
    }

    const historySize = msg.history.length;
    const historyHead = Number.isFinite(msg.history_head) ? msg.history_head : 0;
    const ordered = [];

    for (let i = historyHead; i < historySize; i++) {
        ordered.push(msg.history[i]);
    }
    for (let i = 0; i < historyHead; i++) {
        ordered.push(msg.history[i]);
    }

    const available = ordered.slice(-Math.min(windowSize, ordered.length));
    if (available.length >= windowSize) {
        return available;
    }

    return new Array(windowSize - available.length).fill(null).concat(available);
}
